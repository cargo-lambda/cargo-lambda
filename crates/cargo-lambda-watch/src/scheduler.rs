use crate::{
    requests::{InvokeRequest, ServerError},
    CargoOptions,
};
use axum::{body::Body, response::Response};
use cargo_lambda_invoke::DEFAULT_PACKAGE_FUNCTION;
use ignore_files::IgnoreFile;
use std::{
    collections::{hash_map::Entry, HashMap, VecDeque},
    sync::Arc,
};
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    oneshot, Mutex,
};
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::{error, info};
use watchexec::command::Command;

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

    async fn upsert(&self, req: InvokeRequest) -> Option<(String, String)> {
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

    async fn clean(&self, function_name: &str) {
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

pub(crate) async fn init_scheduler(
    subsys: &SubsystemHandle,
    req_cache: RequestCache,
    cargo_options: CargoOptions,
    no_reload: bool,
    ignore_files: Vec<IgnoreFile>,
) -> Sender<InvokeRequest> {
    let (req_tx, req_rx) = mpsc::channel::<InvokeRequest>(100);

    subsys.start("lambda scheduler", move |s| async move {
        start_scheduler(s, req_cache, cargo_options, no_reload, ignore_files, req_rx).await;
        Ok::<_, std::convert::Infallible>(())
    });

    req_tx
}

async fn start_scheduler(
    subsys: SubsystemHandle,
    req_cache: RequestCache,
    cargo_options: CargoOptions,
    no_reload: bool,
    ignore_files: Vec<IgnoreFile>,
    mut req_rx: Receiver<InvokeRequest>,
) {
    let (gc_tx, mut gc_rx) = mpsc::channel::<String>(10);

    loop {
        tokio::select! {
            Some(req) = req_rx.recv() => {
                if let Some((name, api)) = req_cache.upsert(req).await {
                    let gc_tx = gc_tx.clone();
                    let cargo_options = cargo_options.clone();
                    let ignore_files = ignore_files.clone();
                    subsys.start("lambda runtime", move |s| start_function(s, name, api, cargo_options, no_reload, gc_tx, ignore_files));
                }
            },
            Some(gc) = gc_rx.recv() => {
                req_cache.clean(&gc).await;
            },
            _ = subsys.on_shutdown_requested() => {
                info!("terminating lambda scheduler");
                return;
            },

        };
    }
}

async fn start_function(
    subsys: SubsystemHandle,
    name: String,
    runtime_api: String,
    cargo_options: CargoOptions,
    no_reload: bool,
    gc_tx: Sender<String>,
    ignore_files: Vec<IgnoreFile>,
) -> Result<(), ServerError> {
    info!(function = ?name, manifest = ?cargo_options.manifest_path, "starting lambda function");

    let cmd = cargo_command(&name, &cargo_options);
    let bin_name = if is_valid_bin_name(&name) {
        Some(name.clone())
    } else {
        None
    };

    let wx = crate::watcher::new(
        cmd,
        name.clone(),
        runtime_api.clone(),
        bin_name,
        cargo_options.manifest_path.clone(),
        ignore_files,
        no_reload,
    )
    .await?;

    tokio::select! {
        _ = wx.main() => {
            if let Err(err) = gc_tx.send(name.clone()).await {
                error!(error = %err, function = ?name, "failed to send message to cleanup dead function");
            }
        },
        _ = subsys.on_shutdown_requested() => {
            info!(function = ?name, "terminating lambda function");
        }
    }

    Ok(())
}

fn is_valid_bin_name(name: &str) -> bool {
    !name.is_empty() && name != DEFAULT_PACKAGE_FUNCTION
}

fn cargo_command(name: &str, cargo_options: &CargoOptions) -> watchexec::command::Command {
    let mut args = vec!["run".into()];
    if let Some(features) = cargo_options.features.as_deref() {
        args.push("--features".into());
        args.push(features.into());
    }

    if cargo_options.release {
        args.push("--release".into());
    }

    if is_valid_bin_name(name) {
        args.push("--bin".into());
        args.push(name.into());
    }

    Command::Exec {
        prog: "cargo".into(),
        args,
    }
}
