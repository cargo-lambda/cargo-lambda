use crate::{
    error::ServerError,
    requests::{InvokeRequest, LambdaResponse, NextEvent},
    RUNTIME_EMULATOR_PATH,
};
use cargo_lambda_metadata::cargo::{binary_targets, FunctionRouter};
use miette::Result;
use mpsc::{channel, Receiver, Sender};
use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::{mpsc, oneshot, Mutex, RwLock};
use tracing::debug;
use uuid::Uuid;

#[derive(Clone)]
pub(crate) struct RuntimeState {
    runtime_addr: SocketAddr,
    proxy_addr: Option<SocketAddr>,
    runtime_url: String,
    manifest_path: PathBuf,
    pub initial_functions: HashSet<String>,
    pub function_router: Option<FunctionRouter>,
    pub req_cache: RequestCache,
    pub res_cache: ResponseCache,
    pub ext_cache: ExtensionCache,
}

pub(crate) type RefRuntimeState = Arc<RuntimeState>;

impl RuntimeState {
    pub(crate) fn new(
        runtime_addr: SocketAddr,
        proxy_addr: Option<SocketAddr>,
        manifest_path: PathBuf,
        initial_functions: HashSet<String>,
        function_router: Option<FunctionRouter>,
    ) -> RuntimeState {
        RuntimeState {
            runtime_addr,
            proxy_addr,
            manifest_path,
            initial_functions,
            function_router,
            runtime_url: format!("http://{runtime_addr}{RUNTIME_EMULATOR_PATH}"),
            req_cache: RequestCache::new(),
            res_cache: ResponseCache::new(),
            ext_cache: ExtensionCache::default(),
        }
    }

    pub(crate) fn addresses(&self) -> (SocketAddr, Option<SocketAddr>, String) {
        (self.runtime_addr, self.proxy_addr, self.runtime_url.clone())
    }

    pub(crate) fn function_addr(&self, name: &str) -> String {
        format!("{}/{}", &self.runtime_url, name)
    }

    pub(crate) fn is_default_function_enabled(&self) -> bool {
        self.initial_functions.len() == 1
    }

    pub(crate) fn is_function_available(&self, name: &str) -> Result<(), HashSet<String>> {
        if self.initial_functions.contains(name) {
            return Ok(());
        }

        match binary_targets(&self.manifest_path, false) {
            Err(err) => {
                tracing::error!(?err, "failed to load the project's binaries");
                Err(self.initial_functions.clone())
            }
            Ok(binaries) if binaries.contains(name) => Ok(()),
            Ok(binaries) => Err(binaries),
        }
    }
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

    pub async fn init(&self, function_name: &str) {
        let mut inner = self.inner.write().await;
        inner.insert(function_name.into(), RequestQueue::new());
        debug!(
            function_name,
            "request stack initialized before compilation"
        );
    }

    pub async fn upsert(&self, req: InvokeRequest) -> Result<Option<String>, ServerError> {
        let mut inner = self.inner.write().await;
        let function_name = req.function_name.clone();

        match inner.entry(function_name.clone()) {
            Entry::Vacant(v) => {
                let stack = RequestQueue::new();
                stack.push(req).await?;
                v.insert(stack);

                debug!(?function_name, "request stack initialized in first request");

                Ok(Some(function_name))
            }
            Entry::Occupied(o) => {
                o.into_mut().push(req).await?;
                debug!(?function_name, "request stack increased");

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
        debug!(function_name, "request stack cleaned");
    }

    pub async fn keys(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        inner.keys().cloned().collect()
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
