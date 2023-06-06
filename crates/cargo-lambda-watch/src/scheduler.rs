use crate::{
    requests::InvokeRequest,
    state::RuntimeState,
    watcher::{FunctionData, WatcherConfig},
    CargoOptions,
};
use cargo_lambda_invoke::DEFAULT_PACKAGE_FUNCTION;
use std::sync::Arc;
use tokio::sync::{
    mpsc::{self, Receiver, Sender},
    Mutex,
};
use tokio_graceful_shutdown::SubsystemHandle;
use tracing::{error, info};
use watchexec::{command::Command, event::Event};

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
) {
    let (function_tx, function_rx) = mpsc::channel::<FunctionData>(10);
    let (gc_tx, mut gc_rx) = mpsc::channel::<String>(10);
    let function_rx = Arc::new(Mutex::new(function_rx));

    let wx = crate::watcher::new(
        watcher_config.clone(),
        state.ext_cache.clone(),
        function_rx,
        gc_tx,
    )
    .await
    .expect("watcher to start");

    let wx_handle = wx.main();
    tokio::pin!(wx_handle);

    loop {
        tokio::select! {
            Some(req) = req_rx.recv() => {
                let result = state.req_cache.upsert(req).await?;
                if let Some((name, api)) = result {
                    if !watcher_config.only_lambda_apis {
                        info!(function = name, "starting new lambda");
                        let function_data = function_data(name, api, cargo_options.clone());
                        _ = function_tx.send(function_data).await;
                        _ = wx.send_event(Event::default(), watchexec::event::Priority::Urgent).await;
                    }
                }
            },
            Some(name) = gc_rx.recv() => {
                state.req_cache.clean(&name).await;
            }
            res = &mut wx_handle => {
                if let Err(err) = res {
                    error!(error = %err, "watcher stopped with error");
                }
                info!("watcher stopped gracefully");
                subsys.request_shutdown()
            },
            _ = subsys.on_shutdown_requested() => {
                info!("terminating lambda scheduler");
                return;
            },
        };
    }
}

fn function_data(name: String, runtime_api: String, cargo_options: CargoOptions) -> FunctionData {
    let cmd = cargo_command(&name, &cargo_options);
    let bin_name = if is_valid_bin_name(&name) {
        Some(name.clone())
    } else {
        None
    };

    FunctionData {
        cmd,
        name,
        runtime_api,
        bin_name,
    }
}

fn is_valid_bin_name(name: &str) -> bool {
    !name.is_empty() && name != DEFAULT_PACKAGE_FUNCTION
}

pub(crate) fn cargo_command(
    name: &str,
    cargo_options: &CargoOptions,
) -> watchexec::command::Command {
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
