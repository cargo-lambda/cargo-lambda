use crate::{
    error::ServerError,
    instance_pool::{FunctionInstance, InstanceId, InstancePool},
    requests::{Action, NextEvent},
    state::{ExtensionCache, RuntimeState},
    watcher::WatcherConfig,
};
use cargo_lambda_metadata::DEFAULT_PACKAGE_FUNCTION;
use cargo_options::Run as CargoOptions;
use std::time::Duration;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio_graceful_shutdown::{SubsystemBuilder, SubsystemHandle};
use tracing::{debug, error, info};
use uuid::Uuid;
use watchexec::command::Command;

struct InstanceConfig {
    name: String,
    instance_id: InstanceId,
    runtime_api: String,
    cargo_options: CargoOptions,
    watcher_config: WatcherConfig,
}

pub(crate) fn init_scheduler(
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
    let (gc_tx, mut gc_rx) = mpsc::channel::<(String, InstanceId)>(10);

    if state.max_concurrency > 1 {
        let monitor_state = state.clone();
        let monitor_cargo_options = cargo_options.clone();
        let monitor_watcher_config = watcher_config.clone();
        let monitor_gc_tx = gc_tx.clone();

        subsys.start(SubsystemBuilder::new("instance monitor", move |s| {
            instance_monitor(
                s,
                monitor_state,
                monitor_cargo_options,
                monitor_watcher_config,
                monitor_gc_tx,
            )
        }));
    }

    loop {
        tokio::select! {
            Some(action) = req_rx.recv() => {
                tracing::trace!(?action, "request action received");
                let start_function_name = match action {
                    Action::Invoke(req) => {
                        let function_name = req.function_name.clone();
                        state.req_cache.upsert(req).await?;
                        Some(function_name)
                    },
                    Action::Init => {
                        state.req_cache.init(DEFAULT_PACKAGE_FUNCTION).await;
                        Some(DEFAULT_PACKAGE_FUNCTION.into())
                    },
                };

                if watcher_config.start_function() {
                    if let Some(name) = start_function_name {
                        let queue_depth = state.req_cache.queue_depth(&name).await;
                        let pools = state.instance_pools.read().await;
                        let should_spawn = if let Some(pool) = pools.get(&name) {
                            pool.instance_count().await == 0
                        } else {
                            true
                        };
                        drop(pools);

                        if should_spawn && queue_depth > 0 {
                            spawn_function_instance(
                                &subsys,
                                &state,
                                &name,
                                &cargo_options,
                                &watcher_config,
                                &gc_tx,
                            )
                            .await?;
                        }
                    }
                }
            }
            Some((name, instance_id)) = gc_rx.recv() => {
                debug!(function = ?name, ?instance_id, "cleaning up dead instance");

                let pools = state.instance_pools.read().await;
                if let Some(pool) = pools.get(&name) {
                    pool.remove_instance(&instance_id).await;
                }
                drop(pools);
            }
            _ = subsys.on_shutdown_requested() => {
                info!("terminating lambda scheduler");
                return Ok(());
            }
        }
    }
}

/// Background monitor that checks queue depths and spawns instances as needed
async fn instance_monitor(
    subsys: SubsystemHandle,
    state: RuntimeState,
    cargo_options: CargoOptions,
    watcher_config: WatcherConfig,
    gc_tx: Sender<(String, InstanceId)>,
) -> Result<(), ServerError> {
    let mut interval = tokio::time::interval(Duration::from_millis(100));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let function_names = state.req_cache.keys().await;

                for function_name in function_names {
                    let queue_depth = state.req_cache.queue_depth(&function_name).await;

                    if queue_depth == 0 {
                        continue;
                    }

                    let mut pools = state.instance_pools.write().await;
                    let pool = pools
                        .entry(function_name.clone())
                        .or_insert_with(|| InstancePool::new(state.max_concurrency));
                    let pool_clone = pool.clone();
                    drop(pools);

                    if pool_clone.should_spawn_instance(queue_depth).await {
                        debug!(
                            function = ?function_name,
                            queue_depth,
                            "spawning additional instance"
                        );

                        spawn_function_instance(
                            &subsys,
                            &state,
                            &function_name,
                            &cargo_options,
                            &watcher_config,
                            &gc_tx,
                        )
                        .await?;
                    }
                }
            }
            _ = subsys.on_shutdown_requested() => {
                info!("terminating instance monitor");
                return Ok(());
            }
        }
    }
}

/// Spawn a new function instance
async fn spawn_function_instance(
    subsys: &SubsystemHandle,
    state: &RuntimeState,
    function_name: &str,
    cargo_options: &CargoOptions,
    watcher_config: &WatcherConfig,
    gc_tx: &Sender<(String, InstanceId)>,
) -> Result<(), ServerError> {
    let instance_id = Uuid::new_v4();

    let instance = FunctionInstance::new(instance_id);

    let mut pools = state.instance_pools.write().await;
    let pool = pools
        .entry(function_name.to_string())
        .or_insert_with(|| InstancePool::new(state.max_concurrency));
    pool.add_instance(instance).await;
    drop(pools);

    info!(
        function = ?function_name,
        ?instance_id,
        "spawning function instance"
    );

    let config = InstanceConfig {
        name: function_name.to_string(),
        instance_id,
        runtime_api: state.function_addr(function_name),
        cargo_options: cargo_options.clone(),
        watcher_config: watcher_config.clone(),
    };
    let gc_tx = gc_tx.clone();
    let ext_cache = state.ext_cache.clone();

    subsys.start(SubsystemBuilder::new(
        format!("lambda runtime {}-{}", function_name, instance_id),
        move |s| start_function_instance(s, config, gc_tx, ext_cache),
    ));

    Ok(())
}

async fn start_function_instance(
    subsys: SubsystemHandle,
    config: InstanceConfig,
    gc_tx: Sender<(String, InstanceId)>,
    ext_cache: ExtensionCache,
) -> Result<(), ServerError> {
    let InstanceConfig {
        name,
        instance_id,
        runtime_api,
        cargo_options,
        mut watcher_config,
    } = config;

    let cmd = cargo_command(&name, &cargo_options)?;
    info!(
        function = ?name,
        ?instance_id,
        manifest = ?cargo_options.manifest_path,
        ?cmd,
        "starting lambda function instance"
    );

    watcher_config.bin_name = if is_valid_bin_name(&name) {
        Some(name.clone())
    } else {
        None
    };
    watcher_config.name.clone_from(&name);
    watcher_config.runtime_api = runtime_api;
    watcher_config.instance_id = Some(instance_id);

    let wx = crate::watcher::new(cmd, watcher_config, ext_cache.clone()).await?;

    tokio::select! {
        res = wx.main() => match res {
            Ok(_) => {},
            Err(error) => {
                error!(?error, ?instance_id, "failed to obtain the watchexec task");
                if let Err(error) = gc_tx.send((name.clone(), instance_id)).await {
                    error!(%error, function = ?name, ?instance_id, "failed to send message to cleanup dead function");
                }
            }
        },
        _ = subsys.on_shutdown_requested() => {
            info!(function = ?name, ?instance_id, "terminating lambda function instance");
        }
    }

    let event = NextEvent::shutdown(&format!("{name} instance {} shutting down", instance_id));
    ext_cache.send_event(event).await
}

fn is_valid_bin_name(name: &str) -> bool {
    !name.is_empty() && name != DEFAULT_PACKAGE_FUNCTION
}

#[allow(clippy::result_large_err)]
fn cargo_command(
    name: &str,
    cargo_options: &CargoOptions,
) -> Result<watchexec::command::Command, ServerError> {
    let cmd = if is_valid_bin_name(name) {
        let mut command_opts = cargo_options.clone();
        command_opts.bin.push(name.to_string());
        command_opts.command()
    } else {
        cargo_options.command()
    };

    Ok(Command::Exec {
        prog: cmd.get_program().to_string_lossy().to_string(),
        args: cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().to_string())
            .collect(),
    })
}
