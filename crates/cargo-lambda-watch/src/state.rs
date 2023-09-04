use crate::{
    error::ServerError,
    requests::{InvokeRequest, LambdaResponse, NextEvent},
};
use miette::Result;
use mpsc::{channel, Receiver, Sender};
use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock, TryLockError};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct RuntimeState {
    pub server_addr: String,
    pub req_cache: RequestCache,
    pub ext_cache: ExtensionCache,
}

#[derive(Clone, Debug)]
pub(crate) struct RequestQueue {
    tx: Arc<Sender<InvokeRequest>>,
    rx: Arc<Mutex<Receiver<InvokeRequest>>>,
}

impl RequestQueue {
    pub fn new() -> RequestQueue {
        let (tx, rx) = channel::<InvokeRequest>(100);

        RequestQueue {
            tx: Arc::new(tx),
            rx: Arc::new(Mutex::new(rx)),
        }
    }

    pub async fn pop(&self) -> Option<InvokeRequest> {
        let mut rx = self.rx.lock().await;
        rx.recv().await
    }

    pub async fn push(&self, req: InvokeRequest) -> Result<(), ServerError> {
        self.tx
            .send(req)
            .await
            .map_err(|e| ServerError::SendInvokeMessage(Box::new(e)))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RequestCache {
    inner: Arc<RwLock<HashMap<String, RequestQueue>>>,
}

impl RequestCache {
    pub fn new() -> RequestCache {
        RequestCache {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn upsert(&self, req: InvokeRequest) -> Result<Option<String>, ServerError> {
        let mut inner = self.inner.write().await;
        let name = req.function_name.clone();

        match inner.entry(name.clone()) {
            Entry::Vacant(v) => {
                let stack = RequestQueue::new();
                stack.push(req).await?;
                v.insert(stack);

                Ok(Some(name))
            }
            Entry::Occupied(o) => {
                o.into_mut().push(req).await?;
                Ok(None)
            }
        }
    }

    pub async fn pop(&self, function_name: &str) -> Option<InvokeRequest> {
        let inner = self.inner.read().await;
        let stack = match inner.get(function_name) {
            None => return None,
            Some(s) => s.clone(),
        };
        drop(inner);

        stack.pop().await
    }

    pub async fn clean(&self, function_name: &str) {
        let mut inner = self.inner.write().await;
        inner.remove(function_name);
    }
}

#[derive(Clone)]
pub(crate) struct ResponseCache {
    inner: Arc<Mutex<HashMap<String, oneshot::Sender<LambdaResponse>>>>,
}

impl ResponseCache {
    pub fn new() -> ResponseCache {
        ResponseCache {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn pop(&self, req_id: &str) -> Option<oneshot::Sender<LambdaResponse>> {
        let mut cache = self.inner.lock().await;
        cache.remove(req_id)
    }

    pub async fn push(&self, req_id: &str, resp_tx: oneshot::Sender<LambdaResponse>) {
        let mut cache = self.inner.lock().await;
        cache.insert(req_id.into(), resp_tx);
    }
}

#[derive(Clone, Default)]
pub(crate) struct ExtensionCache {
    extensions: Arc<Mutex<HashMap<String, Vec<String>>>>,
    events: Arc<Mutex<HashMap<String, Vec<String>>>>,
    senders: Arc<Mutex<HashMap<String, mpsc::Sender<NextEvent>>>>,
}

impl ExtensionCache {
    pub async fn register(&self, events: Vec<String>) -> String {
        let mut extensions = self.extensions.lock().await;
        let extension_id = Uuid::new_v4();

        extensions.insert(extension_id.to_string(), events.clone());

        let mut list = self.events.lock().await;
        for event in events {
            list.entry(event)
                .and_modify(|e| e.push(extension_id.to_string()))
                .or_insert(vec![extension_id.to_string()]);
        }

        extension_id.to_string()
    }

    pub async fn set_senders(&self, extension_id: &str, sender: mpsc::Sender<NextEvent>) {
        let extensions = self.extensions.lock().await;
        if let Some(events) = extensions.get(extension_id) {
            let mut senders = self.senders.lock().await;
            for event in events {
                let name = format!("{extension_id}_{event}");
                senders.insert(name, sender.clone());
            }
        }
    }

    pub async fn send_event(&self, event: NextEvent) -> Result<(), ServerError> {
        let events = self.events.lock().await;

        let queue = event.type_queue();

        if let Some(ids) = events.get(queue) {
            let senders = self.senders.lock().await;

            for id in ids {
                let name = format!("{id}_{queue}");
                if let Some(tx) = senders.get(&name) {
                    tx.send(event.clone())
                        .await
                        .map_err(|e| ServerError::SendEventMessage(Box::new(e)))?;
                }
            }
        }

        Ok(())
    }

    pub fn try_send_event(&self, event: NextEvent) -> Result<(), ServerError> {
        let events = self
            .events
            .try_lock()
            .map_err(|e| ServerError::TryLockSendEventMessage(Box::new(e)))?;

        let queue = event.type_queue();

        if let Some(ids) = events.get(queue) {
            let senders = self
                .senders
                .try_lock()
                .map_err(|e| ServerError::TryLockSendEventMessage(Box::new(e)))?;

            for id in ids {
                let name = format!("{id}_{queue}");
                if let Some(tx) = senders.get(&name) {
                    tx.try_send(event.clone())
                        .map_err(|e| ServerError::TrySendEventMessage(Box::new(e)))?;
                }
            }
        }

        Ok(())
    }

    pub async fn clear(&self, extension_id: &str) {
        let extensions = self.extensions.lock().await;
        if let Some(events) = extensions.get(extension_id) {
            let mut senders = self.senders.lock().await;
            for event in events {
                let name = format!("{extension_id}_{event}");
                senders.remove(&name);
            }
        }
    }
}
