use crate::{error::ServerError, requests::NextEvent, state::ExtensionCache};
use cargo_lambda_metadata::{
    cargo::load_metadata,
    config::{ConfigOptions, load_config_without_cli_flags},
};
use ignore::create_filter;
use ignore_files::IgnoreFile;
use std::{collections::HashMap, convert::Infallible, path::PathBuf, sync::Arc, time::Duration};
use tracing::{debug, error, trace};
use watchexec::{
    ErrorHook, Watchexec,
    action::{Action, Outcome, PreSpawn},
    command::Command,
    config::{InitConfig, RuntimeConfig},
    error::RuntimeError,
    event::{Event, Priority, ProcessEnd},
    handler::SyncFnHandler,
    signal::source::MainSignal,
};

pub(crate) mod ignore;

#[derive(Clone, Debug, Default)]
pub(crate) struct WatcherConfig {
    pub runtime_api: String,
    pub name: String,
    pub bin_name: Option<String>,
    pub base: PathBuf,
    pub manifest_path: PathBuf,
    pub ignore_files: Vec<IgnoreFile>,
    pub ignore_changes: bool,
    pub only_lambda_apis: bool,
    pub env: HashMap<String, String>,
    pub wait: bool,
}

impl WatcherConfig {
    pub(crate) fn start_function(&self) -> bool {
        !self.only_lambda_apis
    }

    pub(crate) fn send_function_init(&self) -> bool {
        !self.only_lambda_apis && !self.wait
    }
}

pub(crate) async fn new(
    cmd: Command,
    wc: WatcherConfig,
    ext_cache: ExtensionCache,
) -> Result<Arc<Watchexec>, ServerError> {
    let init = crate::watcher::init();
    let runtime = crate::watcher::runtime(cmd, wc, ext_cache).await?;

    let wx = Watchexec::new(init, runtime).map_err(ServerError::WatcherError)?;
    wx.send_event(Event::default(), Priority::Urgent)
        .await
        .map_err(ServerError::WatcherError)?;

    Ok(wx)
}

fn init() -> InitConfig {
    let mut config = InitConfig::default();
    config.on_error(SyncFnHandler::from(
        |err: ErrorHook| -> std::result::Result<(), Infallible> {
            match err.error {
                RuntimeError::IoError {
                    // according to watchexec's documentation, this errors can be ignored.
                    // see: https://github.com/watchexec/watchexec/blob/e06dc0dd16f8aa88a1556583eafbd985ca2c4eea/crates/lib/src/error/runtime.rs#L13-L15
                    about: "waiting on process group",
                    ..
                } => {}
                RuntimeError::FsWatcher { .. } | RuntimeError::EventChannelTrySend { .. } => {
                    err.elevate()
                }
                e => {
                    error!(error = ?e, "internal error watching your project");
                }
            }

            Ok(())
        },
    ));

    config
}

async fn runtime(
    cmd: Command,
    wc: WatcherConfig,
    ext_cache: ExtensionCache,
) -> Result<RuntimeConfig, ServerError> {
    let mut config = RuntimeConfig::default();

    config.pathset([wc.base.clone()]);
    config.commands(vec![cmd]);

    config.filterer(create_filter(&wc.base, &wc.ignore_files, wc.ignore_changes).await?);

    config.action_throttle(Duration::from_secs(3));

    config.on_action(move |action: Action| {
        let signals: Vec<MainSignal> = action.events.iter().flat_map(|e| e.signals()).collect();
        let has_paths = action
            .events
            .iter()
            .flat_map(|e| e.paths())
            .next()
            .is_some();

        let empty_event = action
            .events
            .iter()
            .map(|e| e.is_empty())
            .next()
            .unwrap_or_default();

        debug!(
            ?action,
            ?signals,
            has_paths,
            empty_event,
            "watcher action received"
        );

        let ext_cache = ext_cache.clone();
        async move {
            if signals.contains(&MainSignal::Terminate) {
                let function_shutdown_delay = ext_cache.function_shutdown_delay().await;
                if let Some(delay) = function_shutdown_delay {
                    function_graceful_shutdown_or_else_sigkill(
                        action,
                        MainSignal::Terminate,
                        delay,
                    );
                    return Ok(());
                }
                action.outcome(Outcome::both(Outcome::Stop, Outcome::Exit));
                return Ok(());
            }
            if signals.contains(&MainSignal::Interrupt) {
                let function_shutdown_delay = ext_cache.function_shutdown_delay().await;
                if let Some(delay) = function_shutdown_delay {
                    function_graceful_shutdown_or_else_sigkill(
                        action,
                        MainSignal::Interrupt,
                        delay,
                    );
                    return Ok(());
                }
                action.outcome(Outcome::both(Outcome::Stop, Outcome::Exit));
                return Ok(());
            }

            if !has_paths {
                if !signals.is_empty() {
                    let mut out = Outcome::DoNothing;
                    for sig in signals {
                        out = Outcome::both(out, Outcome::Signal(sig));
                    }

                    action.outcome(out);
                    return Ok(());
                }

                let completion = action.events.iter().flat_map(|e| e.completions()).next();
                if let Some(status) = completion {
                    match status {
                        Some(ProcessEnd::ExitError(sig)) => {
                            error!(code = ?sig, "command exited");
                        }
                        Some(ProcessEnd::ExitSignal(sig)) => {
                            error!(code = ?sig, "command killed");
                        }
                        Some(ProcessEnd::ExitStop(sig)) => {
                            error!(code = ?sig, "command stopped");
                        }
                        Some(ProcessEnd::Exception(sig)) => {
                            error!(code = ?sig, "command ended by exception");
                        }
                        _ => {}
                    };

                    action.outcome(Outcome::DoNothing);
                    return Ok(());
                }
            }

            if !empty_event {
                let event = NextEvent::shutdown("recompiling function");
                ext_cache.send_event(event).await?;
            }
            let when_running = Outcome::both(Outcome::Stop, Outcome::Start);
            action.outcome(Outcome::if_running(when_running, Outcome::Start));

            Ok::<(), ServerError>(())
        }
    });

    config.on_pre_spawn(move |prespawn: PreSpawn| {
        let name = wc.name.clone();
        let runtime_api = wc.runtime_api.clone();
        let manifest_path = wc.manifest_path.clone();
        let bin_name = wc.bin_name.clone();
        let base_env = wc.env.clone();

        async move {
            trace!("loading watch environment metadata");

            let new_env = reload_env(&manifest_path, &bin_name);

            if let Some(mut command) = prespawn.command().await {
                command
                    .env("AWS_LAMBDA_FUNCTION_VERSION", "1")
                    .env("AWS_LAMBDA_FUNCTION_MEMORY_SIZE", "4096")
                    .envs(base_env)
                    .envs(new_env)
                    .env("AWS_LAMBDA_RUNTIME_API", &runtime_api)
                    .env("AWS_LAMBDA_FUNCTION_NAME", &name);
            }

            Ok::<(), Infallible>(())
        }
    });

    Ok(config)
}

fn function_graceful_shutdown_or_else_sigkill(
    action: Action,
    signal_type: MainSignal,
    max_delay: Duration,
) {
    tracing::debug!(
        ?signal_type,
        ?max_delay,
        "attempting graceful function shutdown"
    );
    action.outcome(Outcome::both(
        // send sigterm
        Outcome::Signal(signal_type),
        // race graceful shutdown against forced shutdown following max delay
        Outcome::race(
            // happy path, process exits then watchexec exits
            Outcome::both(Outcome::Wait, Outcome::Exit),
            // unhappy path, we sleep max delay then SIGKILL and exit watchexec
            Outcome::both(
                Outcome::Sleep(max_delay),
                Outcome::both(Outcome::Stop, Outcome::Exit),
            ),
        ),
    ));
}

fn reload_env(manifest_path: &PathBuf, bin_name: &Option<String>) -> HashMap<String, String> {
    let metadata = match load_metadata(manifest_path) {
        Ok(metadata) => metadata,
        Err(e) => {
            error!("failed to reload metadata: {}", e);
            return HashMap::new();
        }
    };

    let options = ConfigOptions {
        name: bin_name.clone(),
        ..Default::default()
    };
    let config = match load_config_without_cli_flags(&metadata, &options) {
        Ok(config) => config,
        Err(e) => {
            error!("failed to reload config: {}", e);
            return HashMap::new();
        }
    };

    match config.watch.lambda_environment(&config.env) {
        Ok(env) => env,
        Err(e) => {
            error!("failed to reload environment: {}", e);
            HashMap::new()
        }
    }
}
