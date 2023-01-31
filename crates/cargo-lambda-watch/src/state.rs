use crate::{
    error::ServerError,
    requests::{InvokeRequest, NextEvent},
};
use axum::{body::Body, response::Response};
use std::{
    collections::{hash_map::Entry, HashMap, VecDeque},
    sync::Arc,
};
use tokio::sync::{mpsc, oneshot, Mutex};
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct RuntimeState {
    pub req_cache: RequestCache,
    pub ext_cache: ExtensionCache,
}

#[derive(Clone)]
pub(crate) struct RequestQueue {
    inner: Arc<Mutex<VecDeque<InvokeRequest>>>,
}

impl RequestQueue {
    pub fn new() -> RequestQueue {
        RequestQueue {
            inner: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn pop(&self) -> Option<InvokeRequest> {
        let mut queue = self.inner.lock().await;
        queue.pop_front()
    }

    pub async fn push(&self, req: InvokeRequest) {
        let mut queue = self.inner.lock().await;
        queue.push_back(req);
    }
}

#[derive(Clone)]
pub(crate) struct RequestCache {
    server_addr: String,
    inner: Arc<Mutex<HashMap<String, RequestQueue>>>,
}

impl RequestCache {
    pub fn new(server_addr: String) -> RequestCache {
        RequestCache {
            server_addr,
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn upsert(&self, req: InvokeRequest) -> Option<(String, String)> {
        let mut inner = self.inner.lock().await;
        let name = req.function_name.clone();

        match inner.entry(name) {
            Entry::Vacant(v) => {
                let name = req.function_name.clone();
                let runtime_api = format!("{}/{}", &self.server_addr, &name);

                let stack = RequestQueue::new();
                stack.push(req).await;
                v.insert(stack);

                Some((name, runtime_api))
            }
            Entry::Occupied(o) => {
                o.into_mut().push(req).await;
                None
            }
        }
    }

    pub async fn pop(&self, function_name: &str) -> Option<InvokeRequest> {
        let inner = self.inner.lock().await;
        let stack = match inner.get(function_name) {
            None => return None,
            Some(s) => s,
        };

        stack.pop().await
    }

    pub async fn clean(&self, function_name: &str) {
        let mut inner = self.inner.lock().await;
        inner.remove(function_name);
    }
}

#[derive(Clone)]
pub(crate) struct ResponseCache {
    inner: Arc<Mutex<HashMap<String, oneshot::Sender<Response<Body>>>>>,
}

impl ResponseCache {
    pub fn new() -> ResponseCache {
        ResponseCache {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn pop(&self, req_id: &str) -> Option<oneshot::Sender<Response<Body>>> {
        let mut cache = self.inner.lock().await;
        cache.remove(req_id)
    }

    pub async fn push(&self, req_id: &str, resp_tx: oneshot::Sender<Response<Body>>) {
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
