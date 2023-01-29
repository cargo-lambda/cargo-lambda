use crate::requests::ServerError;
use cargo_lambda_metadata::cargo::function_environment_metadata;
use ignore_files::{IgnoreFile, IgnoreFilter};
use std::{collections::HashMap, convert::Infallible, path::PathBuf, sync::Arc, time::Duration};
use tracing::{debug, error, trace, warn};
use watchexec::{
    action::{Action, Outcome, PreSpawn},
    command::Command,
    config::{InitConfig, RuntimeConfig},
    error::RuntimeError,
    event::{Event, Priority, ProcessEnd},
    handler::SyncFnHandler,
    signal::source::MainSignal,
    ErrorHook, Watchexec,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct WatcherConfig {
    pub runtime_api: String,
    pub name: String,
    pub bin_name: Option<String>,
    pub base: PathBuf,
    pub manifest_path: PathBuf,
    pub ignore_files: Vec<IgnoreFile>,
    pub no_reload: bool,
    pub no_start: bool,
    pub env: HashMap<String, String>,
}

pub(crate) async fn new(cmd: Command, wc: WatcherConfig) -> Result<Arc<Watchexec>, ServerError> {
    let init = crate::watcher::init();
    let runtime = crate::watcher::runtime(cmd, wc).await?;

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
            error!(error = ?err.error, "internal error watching your project");

            match err.error {
                RuntimeError::FsWatcher { .. } | RuntimeError::EventChannelTrySend { .. } => {
                    err.elevate()
                }
                _ => {}
            }

            Ok(())
        },
    ));

    config
}

async fn runtime(cmd: Command, wc: WatcherConfig) -> Result<RuntimeConfig, ServerError> {
    let mut config = RuntimeConfig::default();

    debug!(ignore_files = ?wc.ignore_files, "creating watcher config");

    config.pathset([wc.base.clone()]);
    config.commands(vec![cmd]);

    let mut filter = IgnoreFilter::new(&wc.base, &wc.ignore_files)
        .await
        .map_err(ServerError::InvalidIgnoreFiles)?;
    if wc.no_reload {
        filter
            .add_globs(&["**/*"], Some(wc.base))
            .await
            .map_err(ServerError::InvalidIgnoreFiles)?;
    }
    config.filterer(Arc::new(watchexec_filterer_ignore::IgnoreFilterer(filter)));

    config.action_throttle(Duration::from_secs(3));

    config.on_action(move |action: Action| {
        let fut = async { Ok::<(), Infallible>(()) };

        let signals: Vec<MainSignal> = action.events.iter().flat_map(|e| e.signals()).collect();
        let has_paths = action
            .events
            .iter()
            .flat_map(|e| e.paths())
            .next()
            .is_some();

        debug!(action = ?action, signals = ?signals, has_paths = has_paths, "watcher action received");

        if signals.contains(&MainSignal::Terminate) {
            action.outcome(Outcome::both(Outcome::Stop, Outcome::Exit));
            return fut;
        }

        if signals.contains(&MainSignal::Interrupt) {
            action.outcome(Outcome::both(Outcome::Stop, Outcome::Exit));
            return fut;
        }

        if !has_paths {
            if !signals.is_empty() {
                let mut out = Outcome::DoNothing;
                for sig in signals {
                    out = Outcome::both(out, Outcome::Signal(sig.into()));
                }

                action.outcome(out);
                return fut;
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
                return fut;
            }
        }

        let when_running = Outcome::both(Outcome::Stop, Outcome::Start);
        action.outcome(Outcome::if_running(when_running, Outcome::Start));

        fut
    });

    config.on_pre_spawn(move |prespawn: PreSpawn| {
        let name = wc.name.clone();
        let runtime_api = wc.runtime_api.clone();
        let manifest_path = wc.manifest_path.clone();
        let bin_name = wc.bin_name.clone();
        let base_env = wc.env.clone();

        async move {
            trace!("loading watch environment metadata");

            let env = function_environment_metadata(manifest_path, bin_name.as_deref())
                .map_err(|err| {
                    warn!(error = %err, "ignoring invalid function metadata");
                    err
                })
                .unwrap_or_default();

            if let Some(mut command) = prespawn.command().await {
                command
                    .env("AWS_LAMBDA_FUNCTION_VERSION", "1")
                    .env("AWS_LAMBDA_FUNCTION_MEMORY_SIZE", "4096")
                    .envs(base_env)
                    .envs(env)
                    .env("AWS_LAMBDA_RUNTIME_API", &runtime_api)
                    .env("AWS_LAMBDA_FUNCTION_NAME", &name);
            }

            Ok::<(), Infallible>(())
        }
    });

    Ok(config)
}
