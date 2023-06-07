use crate::{error::ServerError, requests::NextEvent, state::ExtensionCache};
use cargo_lambda_metadata::cargo::function_environment_metadata;
use ignore_files::{IgnoreFile, IgnoreFilter};
use std::{collections::HashMap, convert::Infallible, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{
    mpsc::{Receiver, Sender},
    Mutex,
};
use tracing::{debug, error, info, trace};
use watchexec::{
    action::{Action, Outcome, PostSpawn, PreSpawn},
    command::{Command, SupervisorId},
    config::{InitConfig, RuntimeConfig},
    error::RuntimeError,
    event::{Event, Priority, ProcessEnd, Tag},
    handler::SyncFnHandler,
    signal::source::MainSignal,
    ErrorHook, Watchexec,
};

#[derive(Clone, Debug, Default)]
pub(crate) struct WatcherConfig {
    pub base: PathBuf,
    pub manifest_path: PathBuf,
    pub ignore_files: Vec<IgnoreFile>,
    pub ignore_changes: bool,
    pub only_lambda_apis: bool,
    pub env: HashMap<String, String>,
}

pub(crate) async fn new(
    wc: WatcherConfig,
    ext_cache: ExtensionCache,
    function_rx: Arc<Mutex<Receiver<FunctionData>>>,
    gc_tx: Sender<String>,
) -> Result<Arc<Watchexec>, ServerError> {
    let init = crate::watcher::init();
    let runtime = crate::watcher::runtime(wc, ext_cache, function_rx, gc_tx).await?;

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

#[derive(Debug)]
pub(crate) struct FunctionData {
    pub cmd: Command,
    pub name: String,
    pub runtime_api: String,
    pub bin_name: Option<String>,
}

async fn runtime(
    wc: WatcherConfig,
    ext_cache: ExtensionCache,
    function_rx: Arc<Mutex<Receiver<FunctionData>>>,
    gc_tx: Sender<String>,
) -> Result<RuntimeConfig, ServerError> {
    let mut config = RuntimeConfig::default();
    let function_cache: Arc<Mutex<HashMap<SupervisorId, FunctionData>>> =
        Arc::new(Mutex::new(HashMap::new()));

    debug!(ignore_files = ?wc.ignore_files, "creating watcher config");

    config.pathset([wc.base.clone()]);

    let mut filter = IgnoreFilter::new(&wc.base, &wc.ignore_files)
        .await
        .map_err(ServerError::InvalidIgnoreFiles)?;
    if wc.ignore_changes {
        filter
            .add_globs(&["**/*"], Some(&wc.base))
            .map_err(ServerError::InvalidIgnoreFiles)?;
    }
    config.filterer(Arc::new(watchexec_filterer_ignore::IgnoreFilterer(filter)));

    config.action_throttle(Duration::from_secs(3));

    {
        let function_cache = function_cache.clone();
        let gc_tx = gc_tx.clone();

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

            // TODO gc completed lambda functions
            let process_completions = action
                .events
                .iter()
                .filter_map(|e| {
                    e.tags.iter().find_map(|t| match t {
                        Tag::ProcessCompletion(data) => Some(data),
                        _ => None,
                    })
                })
                .cloned()
                .collect::<Vec<_>>();

            debug!(
                ?action,
                ?signals,
                has_paths,
                empty_event,
                "watcher action received"
            );

            let ext_cache = ext_cache.clone();
            let function_rx = function_rx.clone();
            let function_cache = function_cache.clone();
            let gc_tx = gc_tx.clone();
            async move {
                // TODO filter events
                // let function_events: HashMap<String, Vec<Event>> = HashMap::new();
                async fn apply_all(action: &Action, outcome: Outcome) {
                    for &function in action.list() {
                        action
                            .apply(outcome.clone(), function, watchexec::action::EventSet::All)
                            .await;
                    }
                }

                info!(signals = ?signals, "searching signals");
                if signals.contains(&MainSignal::Terminate) {
                    apply_all(&action, Outcome::both(Outcome::Stop, Outcome::Exit)).await;

                    return Ok(());
                }

                if signals.contains(&MainSignal::Interrupt) {
                    apply_all(&action, Outcome::both(Outcome::Stop, Outcome::Exit)).await;
                    return Ok(());
                }

                if !has_paths {
                    if !signals.is_empty() {
                        let mut out = Outcome::DoNothing;
                        for sig in signals {
                            out = Outcome::both(out, Outcome::Signal(sig));
                        }
                        apply_all(&action, out).await;

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

                        // `DoNothing` is equivalent to doing nothing here and applying no
                        // outcome.
                        return Ok(());
                    }
                }

                if !empty_event {
                    let event = NextEvent::shutdown("recompiling function");
                    ext_cache.send_event(event).await?;
                }

                let when_running = Outcome::both(Outcome::Stop, Outcome::Start);
                info!("setting outcome to running");
                apply_all(&action, Outcome::if_running(when_running, Outcome::Start)).await;

                // Start up new functions only if
                while let Ok(function) = function_rx.lock().await.try_recv() {
                    info!(function = ?function, "starting function process");
                    let process = action
                        .create(vec![function.cmd.clone()], watchexec::action::EventSet::All)
                        .await;
                    function_cache.lock().await.insert(process, function);
                }

                // TODO gc completed lambda functions
                for _process_end in process_completions {
                    /*
                    let Some(name) = function_cache.lock().await.remove(&e.).map(|f| f.name) else {
                        continue;
                    };

                    if let Err(err) = gc_tx.send(name.clone()).await {
                        error!(error = %err, function = ?name, "failed to send message to clean up dead function");
                    }
                    */
                }

                println!("action handler done!");

                Ok::<(), ServerError>(())
            }
        });
    }

    let fc_clone = function_cache.clone();
    config.on_pre_spawn(move |prespawn: PreSpawn| {
        let function_cache = fc_clone.clone();
        let manifest_path = wc.manifest_path.clone();
        let base_env = wc.env.clone();

        async move {
            let function_cache = function_cache.clone().lock_owned().await;
            let pid = prespawn.supervisor();
            let Some(FunctionData {
                cmd: _,
                name,
                runtime_api,
                bin_name,
            }) = function_cache.get(&pid) else { return Ok::<(), Infallible>(()); };

            trace!("loading watch environment metadata");

            let env = function_environment_metadata(manifest_path, bin_name.as_deref())
                .map_err(|err| {
                    error!(error = %err, "invalid function metadata");
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

                println!("command with ENV: {command:?}");
            }

            Ok::<(), Infallible>(())
        }
    });

    Ok(config)
}
