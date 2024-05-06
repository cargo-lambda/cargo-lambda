use crate::{
    error::ServerError,
    requests::{Action, NextEvent},
    state::{ExtensionCache, RuntimeState},
    watcher::WatcherConfig,
    CargoOptions,
};
use cargo_lambda_invoke::DEFAULT_PACKAGE_FUNCTION;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio_graceful_shutdown::{SubsystemBuilder, SubsystemHandle};
use tracing::{error, info};
use watchexec::command::Command;

pub(crate) async fn init_scheduler(
    subsys: &SubsystemHandle,
    state: RuntimeState,
    cargo_options: CargoOptions,
    watcher_config: WatcherConfig,
) -> Sender<Action> {
    let (req_tx, req_rx) = mpsc::channel::<Action>(100);

    subsys.start(SubsystemBuilder::new("lambda scheduler", move |s| {
        start_scheduler(s, state, cargo_options, watcher_config, req_rx)
    }));

    req_tx
}

async fn start_scheduler(
    subsys: SubsystemHandle,
    state: RuntimeState,
    cargo_options: CargoOptions,
    watcher_config: WatcherConfig,
    mut req_rx: Receiver<Action>,
) -> Result<(), ServerError> {
    let (gc_tx, mut gc_rx) = mpsc::channel::<String>(10);

    loop {
        tokio::select! {
            Some(action) = req_rx.recv() => {
                tracing::trace!(?action, "request action received");
                let start_function_name = match action {
                    Action::Invoke(req) => {
                        state.req_cache.upsert(req).await?
                    },
                    Action::Init => {
                        state.req_cache.init(DEFAULT_PACKAGE_FUNCTION).await;
                        Some(DEFAULT_PACKAGE_FUNCTION.into())
                    },
                };

                if watcher_config.start_function() {
                    if let Some(name) = start_function_name {
                        let runtime_api = state.function_addr(&name);
                        let gc_tx = gc_tx.clone();
                        let cargo_options = cargo_options.clone();
                        let watcher_config = watcher_config.clone();
                        let ext_cache = state.ext_cache.clone();
                        subsys.start(SubsystemBuilder::new("lambda runtime", move |s| start_function(s, name, runtime_api, cargo_options, watcher_config, gc_tx, ext_cache)));
                    }
                }
            }
            Some(name) = gc_rx.recv() => {
                state.req_cache.clean(&name).await;
            }
            _ = subsys.on_shutdown_requested() => {
                info!("terminating lambda scheduler");
                return Ok(());
            }
        };
    }
}

async fn start_function(
    subsys: SubsystemHandle,
    name: String,
    runtime_api: String,
    cargo_options: CargoOptions,
    mut watcher_config: WatcherConfig,
    gc_tx: Sender<String>,
    ext_cache: ExtensionCache,
) -> Result<(), ServerError> {
    let cmd = cargo_command(&name, &cargo_options)?;
    info!(function = ?name, manifest = ?cargo_options.manifest_path, ?cmd, "starting lambda function");

    watcher_config.bin_name = if is_valid_bin_name(&name) {
        Some(name.clone())
    } else {
        None
    };
    watcher_config.name.clone_from(&name);
    watcher_config.runtime_api = runtime_api;

    let wx = crate::watcher::new(cmd, watcher_config, ext_cache.clone()).await?;

    tokio::select! {
        res = wx.main() => match res {
            Ok(_) => {},
            Err(error) => {
                error!(?error, "failed to obtain the watchexec task");
                if let Err(error) = gc_tx.send(name.clone()).await {
                    error!(%error, function = ?name, "failed to send message to cleanup dead function");
                }
            }
        },
        _ = subsys.on_shutdown_requested() => {
            info!(function = ?name, "terminating lambda function");
        }
    }

    let event = NextEvent::shutdown(&format!("{name} function shutting down"));
    ext_cache.send_event(event).await
}

fn is_valid_bin_name(name: &str) -> bool {
    !name.is_empty() && name != DEFAULT_PACKAGE_FUNCTION
}

fn cargo_command(
    name: &str,
    cargo_options: &CargoOptions,
) -> Result<watchexec::command::Command, ServerError> {
    let mp = cargo_options
        .manifest_path
        .to_str()
        .ok_or_else(|| ServerError::InvalidManifest(cargo_options.manifest_path.clone()))?;

    let mut args = vec![
        "run".into(),
        "--manifest-path".into(),
        mp.to_string(),
        "--color".into(),
        cargo_options.color.clone(),
    ];

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

    Ok(Command::Exec {
        prog: "cargo".into(),
        args,
    })
}
