use crate::{error::ServerError, requests::NextEvent, state::ExtensionCache};
use cargo_lambda_metadata::cargo::function_environment_metadata;
use ignore_files::{IgnoreFile, IgnoreFilter};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error, info, trace};
use watchexec::{
    action::{Action, Outcome, PreSpawn},
    changeable::ChangeableFn,
    command::{Command, SupervisorId},
    error::RuntimeError,
    ErrorHook, Watchexec,
};
use watchexec_events::{Event, Priority, ProcessEnd, Tag};
use watchexec_signals::Signal;

#[derive(Clone, Debug, Default)]
pub(crate) struct WatcherConfig {
    pub base: PathBuf,
    pub manifest_path: PathBuf,
    pub ignore_files: Vec<IgnoreFile>,
    pub ignore_changes: bool,
    pub only_lambda_apis: bool,
    pub env: HashMap<String, String>,
}

impl WatcherConfig {
    pub(crate) fn start_function(&self) -> bool {
        !self.only_lambda_apis
    }
}

pub(crate) async fn new(
    wc: WatcherConfig,
    ext_cache: ExtensionCache,
    function_rx: Arc<Mutex<Receiver<FunctionData>>>,
    gc_tx: Sender<String>,
) -> Result<Arc<Watchexec>, ServerError> {
    debug!(ignore_files = ?wc.ignore_files, "creating watcher config");

    // instantiate the watcher with its handlers
    let wx = handlers(wc.clone(), ext_cache, function_rx, gc_tx).await?;
    // set error handler
    wx.config.on_error(|err: ErrorHook| {
        match err.error {
            RuntimeError::IoError {
                // according to wx's documentation, this errors can be ignored.
                // see: https://github.com/wx/wx/blob/e06dc0dd16f8aa88a1556583eafbd985ca2c4eea/crates/lib/src/error/runtime.rs#L13-L15
                about: "waiting on process group",
                ..
            } => {}
            RuntimeError::FsWatcher { .. } | RuntimeError::EventChannelTrySend { .. } => {
                err.elevate()
            }
            e => {
                error!(error = ?e, "internal error watching your project");
            }
        };
    });

    // set pathset
    wx.config.pathset([wc.base.clone()]);

    // set filters
    let mut filter = IgnoreFilter::new(&wc.base, &wc.ignore_files)
        .await
        .map_err(ServerError::InvalidIgnoreFiles)?;
    filter
        .add_globs(&["target/*", "target*"], Some(&wc.base))
        .map_err(ServerError::InvalidIgnoreFiles)?;

    if wc.ignore_changes {
        filter
            .add_globs(&["**/*"], Some(&wc.base))
            .map_err(ServerError::InvalidIgnoreFiles)?;
    }
    wx.config
        .filterer(Arc::new(watchexec_filterer_ignore::IgnoreFilterer(filter)));

    wx.send_event(Event::default(), Priority::Urgent)
        .await
        .map_err(ServerError::WatcherError)?;

    Ok(wx)
}

#[derive(Debug, Clone)]
pub(crate) struct FunctionData {
    pub cmd: Command,
    pub name: String,
    pub runtime_api: String,
    pub bin_name: Option<String>,
}

async fn handlers(
    wc: WatcherConfig,
    ext_cache: ExtensionCache,
    function_rx: Arc<Mutex<Receiver<FunctionData>>>,
    gc_tx: Sender<String>,
) -> Result<Arc<Watchexec>, ServerError> {
    let function_cache: Arc<Mutex<HashMap<SupervisorId, FunctionData>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Closure creates function specific `pre_spawn_hook` based on `SupervisorId`
    let create_prespawn = {
        let function_cache = function_cache.clone();
        let wc = wc.clone();
        move |pid: SupervisorId| {
            let pre_spawn_hook = ChangeableFn::default();
            {
                let function_cache = function_cache.clone();
                let wc = wc.clone();
                pre_spawn_hook.replace(move |prespawn: PreSpawn| {
                    let function_cache = function_cache.clone();
                    let manifest_path = wc.manifest_path.clone();
                    let base_env = wc.env.clone();

                    let Some(FunctionData {
                        cmd: _,
                        name,
                        runtime_api,
                        bin_name,
                    }) = function_cache
                        .try_lock()
                        .ok()
                        .and_then(|guard| guard.get(&pid).cloned())
                    else {
                        return;
                    };

                    trace!("loading watch environment metadata");

                    let env = function_environment_metadata(manifest_path, bin_name.as_deref())
                        .map_err(|err| {
                            error!(error = %err, "invalid function metadata");
                            err
                        })
                        .unwrap_or_default();

                    let mut command = prespawn.command();
                    command
                        .env("AWS_LAMBDA_FUNCTION_VERSION", "1")
                        .env("AWS_LAMBDA_FUNCTION_MEMORY_SIZE", "4096")
                        .envs(base_env)
                        .envs(env)
                        .env("AWS_LAMBDA_RUNTIME_API", &runtime_api)
                        .env("AWS_LAMBDA_FUNCTION_NAME", &name);
                });
            }

            pre_spawn_hook
        }
    };

    // Main action handler
    let handler = {
        let function_cache = function_cache.clone();
        let gc_tx = gc_tx.clone();

        move |action: Action| {
            let signals: Vec<Signal> = action.events.iter().flat_map(|e| e.signals()).collect();
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
            let _gc_tx = gc_tx.clone();
            // TODO filter events
            // let function_events: HashMap<String, Vec<Event>> = HashMap::new();
            let apply_all = |action: &Action, outcome: Outcome| {
                let Ok(fc) = function_cache.lock() else {
                    return;
                };

                for &process_id in fc.keys() {
                    action.apply(
                        process_id,
                        outcome.clone(),
                        watchexec::action::EventSet::All,
                    );
                }
            };

            info!(signals = ?signals, "searching signals");
            if signals.contains(&Signal::Terminate) {
                apply_all(&action, Outcome::both(Outcome::Stop, Outcome::Exit));

                return;
            }

            if signals.contains(&Signal::Interrupt) {
                apply_all(&action, Outcome::both(Outcome::Stop, Outcome::Exit));
                return;
            }

            if !has_paths {
                if !signals.is_empty() {
                    let mut out = Outcome::DoNothing;
                    for sig in signals {
                        out = Outcome::both(out, Outcome::Signal(sig));
                    }
                    apply_all(&action, out);

                    return;
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
                    return;
                }
            }

            if !empty_event {
                let event = NextEvent::shutdown("recompiling function");
                if let Err(err) = ext_cache.try_send_event(event) {
                    error!(err = ?err, "failed to try send event");
                };
            }

            let when_running = Outcome::both(Outcome::Stop, Outcome::Start);
            info!("setting outcome to running");
            apply_all(&action, Outcome::if_running(when_running, Outcome::Start));

            // Start functions queued up by scheduler.
            while let Some(function) = function_rx
                .lock()
                .ok()
                .and_then(|mut guard| guard.try_recv().ok())
            {
                let name = &function.name;
                info!( %name, "starting function process");
                let process = action.create(function.cmd.clone());
                action.apply(
                    process,
                    Outcome::StartHook(create_prespawn(process)),
                    watchexec::action::EventSet::All,
                );
                if let Ok(mut fc) = function_cache.lock() {
                    fc.insert(process, function);
                } else {
                    error!(function = ?function, "function cache lock could not be aquired");
                }
            }

            // Clean up dead functions.
            // TODO gc completed lambda functions
            // Iterate over all completed processes, sending dead functions to the gc by mapping
            // to them via their SupervisorId.
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
        }
    };

    let wx = Watchexec::new(handler).map_err(ServerError::WatcherError)?;

    Ok(wx)
}
