use std::sync::Arc;

use crate::{
    error::ServerError,
    requests::{InvokeRequest, NextEvent},
    state::{ExtensionCache, RuntimeState},
    watcher::WatcherConfig,
    CargoOptions,
};
use cargo_lambda_invoke::DEFAULT_PACKAGE_FUNCTION;
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    Mutex,
};
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::{error, info};
use watchexec::{command::Command, event::Event, Watchexec};

pub(crate) async fn init_scheduler(
    subsys: &SubsystemHandle,
    state: RuntimeState,
    cargo_options: CargoOptions,
    watcher_config: WatcherConfig,
) -> Sender<InvokeRequest> {
    let (req_tx, req_rx) = mpsc::channel::<InvokeRequest>(100);

    subsys.start("lambda scheduler", move |s| async move {
        start_scheduler(s, state, cargo_options, watcher_config, req_rx).await
    });

    req_tx
}

async fn start_scheduler(
    subsys: SubsystemHandle,
    state: RuntimeState,
    cargo_options: CargoOptions,
    watcher_config: WatcherConfig,
    mut req_rx: Receiver<InvokeRequest>,
) -> Result<(), ServerError> {
    let (gc_tx, mut gc_rx) = mpsc::channel::<String>(10);
    let (function_tx, function_rx) = mpsc::channel::<Command>(10);
    let function_rx = Arc::new(Mutex::new(function_rx));
    let mut wx: Option<Arc<Watchexec>> = None;

    loop {
        tokio::select! {
            Some(req) = req_rx.recv() => {
                let result = state.req_cache.upsert(req).await?;
                if let Some((name, api)) = result {
                    if !watcher_config.only_lambda_apis {
                        if let Some(wx) = wx.as_ref().cloned() {
                            info!(function = name, "starting new lambda");
                            let cmd = cargo_command(&name, &cargo_options);
                            _ = function_tx.send(cmd).await;
                            _ = wx.send_event(Event::default(), watchexec::event::Priority::Urgent).await;
                        } else {
                            let gc_tx = gc_tx.clone();
                            let cargo_options = cargo_options.clone();
                            let watcher_config = watcher_config.clone();
                            let ext_cache = state.ext_cache.clone();
                            let function_rx = function_rx.clone();
                            let Ok(new_wx) = create_watcher(
                                name,
                                api,
                                cargo_options,
                                watcher_config,
                                ext_cache.clone(),
                                function_rx
                            ).await else {
                                continue;
                            };
                            wx = Some(new_wx.clone());

                            subsys.start("lambda runtime", move |s| start_watcher(s, new_wx, gc_tx, ext_cache));
                        }
                    }
                }
            },
            Some(gc) = gc_rx.recv() => {
                state.req_cache.clean(&gc).await;
            },
            _ = subsys.on_shutdown_requested() => {
                info!("terminating lambda scheduler");
                return Ok(());
            },

        };
    }
}

async fn create_watcher(
    name: String,
    runtime_api: String,
    cargo_options: CargoOptions,
    mut watcher_config: WatcherConfig,
    ext_cache: ExtensionCache,
    function_rx: Arc<Mutex<Receiver<Command>>>,
) -> Result<Arc<Watchexec>, ServerError> {
    info!(function = ?name, manifest = ?cargo_options.manifest_path, "starting lambda function");

    let cmd = cargo_command(&name, &cargo_options);
    watcher_config.bin_name = if is_valid_bin_name(&name) {
        Some(name.clone())
    } else {
        None
    };
    watcher_config.name = name.clone();
    watcher_config.runtime_api = runtime_api;

    crate::watcher::new(cmd, watcher_config, ext_cache.clone(), function_rx).await
}

async fn start_watcher(
    subsys: SubsystemHandle,
    wx: Arc<Watchexec>,
    gc_tx: Sender<String>,
    ext_cache: ExtensionCache,
) -> Result<(), ServerError> {
    tokio::select! {
        _ = wx.main() => {
            if let Err(err) = gc_tx.send("watcher".to_string()).await {
                error!(error = %err, "failed to send message to cleanup dead watcher");
            }
        },
        _ = subsys.on_shutdown_requested() => {
            info!("terminating watcher");
        }
    }

    let event = NextEvent::shutdown(&format!("watcher shutting down"));
    ext_cache.send_event(event).await
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
