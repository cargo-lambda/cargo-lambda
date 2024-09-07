use axum::{extract::Extension, http::header::HeaderName, Router};
use cargo_lambda_invoke::DEFAULT_PACKAGE_FUNCTION;
use cargo_lambda_metadata::{cargo::binary_targets, env::EnvOptions, lambda::Timeout};
use clap::{Args, ValueHint};
use miette::{IntoDiagnostic, Result, WrapErr};
use opentelemetry::{
    global,
    sdk::{export::trace::stdout, trace, trace::Tracer},
};
use opentelemetry_aws::trace::XrayPropagator;
use std::{
    collections::HashSet,
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};
use tokio::time::Duration;
use tokio_graceful_shutdown::{SubsystemBuilder, SubsystemHandle, Toplevel};
use tower_http::{
    catch_panic::CatchPanicLayer,
    cors::CorsLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};

use tracing::{info, trace, Subscriber};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::LookupSpan;

mod error;
mod requests;
mod runtime;

mod scheduler;
use scheduler::*;
mod state;
use state::*;
mod trigger_router;
mod watcher;
use watcher::WatcherConfig;

use crate::{error::ServerError, requests::Action};

pub(crate) const RUNTIME_EMULATOR_PATH: &str = "/.rt";

#[derive(Args, Clone, Debug)]
#[command(
    name = "watch",
    visible_alias = "start",
    after_help = "Full command documentation: https://www.cargo-lambda.info/commands/watch.html"
)]
pub struct Watch {
    /// Ignore any code changes, and don't reload the function automatically
    #[arg(long, visible_alias = "no-reload")]
    ignore_changes: bool,

    /// Start the Lambda runtime APIs without starting the function.
    /// This is useful if you start (and debug) your function in your IDE.
    #[arg(long)]
    only_lambda_apis: bool,

    #[cfg_attr(
        target_os = "windows",
        arg(short = 'a', long, default_value = "127.0.0.1")
    )]
    #[cfg_attr(
        not(target_os = "windows"),
        arg(short = 'a', long, default_value = "::")
    )]
    /// Address where users send invoke requests
    invoke_address: String,

    /// Address port where users send invoke requests
    #[arg(short = 'p', long, default_value = "9000")]
    invoke_port: u16,

    /// Print OpenTelemetry traces after each function invocation
    #[arg(long)]
    print_traces: bool,

    /// Wait for the first invocation to run the function
    #[arg(long, short)]
    wait: bool,

    /// Disable the default CORS configuration
    #[arg(long)]
    disable_cors: bool,

    /// How long the invoke request waits for a response
    #[arg(long)]
    timeout: Option<Timeout>,

    #[command(flatten)]
    cargo_options: CargoOptions,

    #[command(flatten)]
    env_options: EnvOptions,
}

#[derive(Args, Clone, Debug)]
struct CargoOptions {
    /// Path to Cargo.toml
    #[arg(long, value_name = "PATH", value_hint = ValueHint::FilePath)]
    #[arg(default_value = "Cargo.toml")]
    manifest_path: PathBuf,

    /// Features to pass to `cargo run`, separated by comma
    #[arg(long, short = 'F')]
    features: Option<String>,

    /// Enable release mode when the emulator starts
    #[arg(long, short = 'r')]
    release: bool,

    /// Change the color options in the `cargo run` command
    #[arg(skip)]
    color: String,

    /// Ignore all default features
    #[arg(long)]
    no_default_features: bool,
}

impl Watch {
    #[tracing::instrument(skip(self), target = "cargo_lambda")]
    pub async fn run(&self, color: &str) -> Result<()> {
        tracing::trace!(options = ?self, "watching project");

        let ip = IpAddr::from_str(&self.invoke_address)
            .into_diagnostic()
            .wrap_err("invalid invoke address")?;
        let addr = SocketAddr::from((ip, self.invoke_port));

        let mut cargo_options = self.cargo_options.clone();
        cargo_options.color = color.into();

        let base = dunce::canonicalize(".").into_diagnostic()?;
        let ignore_files = discover_ignore_files(&base).await;

        let env = self.env_options.lambda_environment().into_diagnostic()?;

        let binary_packages = binary_targets(&cargo_options.manifest_path, false)
            .map_err(ServerError::FailedToReadMetadata)?;

        if binary_packages.is_empty() {
            Err(ServerError::NoBinaryPackages)?;
        }

        let watcher_config = WatcherConfig {
            base,
            ignore_files,
            ignore_changes: self.ignore_changes,
            only_lambda_apis: self.only_lambda_apis,
            manifest_path: cargo_options.manifest_path.clone(),
            env: env.variables().cloned().unwrap_or_default(),
            wait: self.wait,
            ..Default::default()
        };

        let runtime_state =
            RuntimeState::new(addr, cargo_options.manifest_path.clone(), binary_packages);

        let disable_cors = self.disable_cors;
        let timeout = self.timeout.clone();

        Toplevel::new(move |s| async move {
            s.start(SubsystemBuilder::new("Lambda server", move |s| {
                start_server(
                    s,
                    runtime_state,
                    cargo_options,
                    watcher_config,
                    disable_cors,
                    timeout,
                )
            }));
        })
        .catch_signals()
        .handle_shutdown_requests(Duration::from_millis(1000))
        .await
        .into_diagnostic()
    }

    pub fn xray_layer<S>(&self) -> OpenTelemetryLayer<S, Tracer>
    where
        S: Subscriber + for<'span> LookupSpan<'span>,
    {
        global::set_text_map_propagator(XrayPropagator::default());

        let builder = stdout::new_pipeline().with_trace_config(
            trace::config()
                .with_sampler(trace::Sampler::AlwaysOn)
                .with_id_generator(trace::XrayIdGenerator::default()),
        );
        let tracer = if self.print_traces {
            builder.install_simple()
        } else {
            builder.with_writer(std::io::sink()).install_simple()
        };
        tracing_opentelemetry::layer().with_tracer(tracer)
    }
}

/// we discover ignore files from the `CARGO_LAMBDA_IGNORE_FILES` environment variable,
/// the current directory, and any parent directories that represent project roots
async fn discover_ignore_files(base: &Path) -> Vec<ignore_files::IgnoreFile> {
    let mut ignore_files = Vec::new();

    let (mut env_ignore, env_ignore_errs) =
        ignore_files::from_environment(Some("CARGO_LAMBDA")).await;
    trace!(ignore_files = ?env_ignore, errors = ?env_ignore_errs, "discovered ignore files from environment variable");
    ignore_files.append(&mut env_ignore);

    let (mut origin_ignore, origin_ignore_errs) = ignore_files::from_origin(base).await;
    trace!(ignore_files = ?origin_ignore, errors = ?origin_ignore_errs, "discovered ignore files from origin");
    ignore_files.append(&mut origin_ignore);

    let mut origins = HashSet::new();
    let mut current = base;
    if base.is_dir() && base.join("Cargo.toml").is_file() {
        origins.insert(base.to_owned());
    }

    while let Some(parent) = current.parent() {
        current = parent;
        if current.is_dir() && current.join("Cargo.toml").is_file() {
            origins.insert(current.to_owned());
        } else {
            break;
        }
    }

    for parent in origins {
        let (mut parent_ignore, parent_ignore_errs) = ignore_files::from_origin(&parent).await;
        trace!(parent = ?parent, ignore_files = ?parent_ignore, errors = ?parent_ignore_errs, "discovered ignore files from parent origin");
        ignore_files.append(&mut parent_ignore);
    }

    ignore_files
}

async fn start_server(
    subsys: SubsystemHandle,
    runtime_state: RuntimeState,
    cargo_options: CargoOptions,
    watcher_config: WatcherConfig,
    disable_cors: bool,
    timeout: Option<Timeout>,
) -> Result<(), axum::Error> {
    let only_lambda_apis = watcher_config.only_lambda_apis;
    let init_default_function =
        runtime_state.is_default_function_enabled() && watcher_config.send_function_init();

    let (socket_addr, runtime_addr) = runtime_state.addresses();

    let x_request_id = HeaderName::from_static("lambda-runtime-aws-request-id");
    let req_tx = init_scheduler(
        &subsys,
        runtime_state.clone(),
        cargo_options,
        watcher_config,
    )
    .await;

    let state_ref = Arc::new(runtime_state);
    let mut app = Router::new()
        .merge(trigger_router::routes().with_state(state_ref.clone()))
        .nest(
            RUNTIME_EMULATOR_PATH,
            runtime::routes().with_state(state_ref.clone()),
        )
        .layer(SetRequestIdLayer::new(
            x_request_id.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(x_request_id))
        .layer(Extension(req_tx.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new());
    if !disable_cors {
        app = app.layer(CorsLayer::very_permissive());
    }
    if let Some(timeout) = timeout {
        app = app.layer(TimeoutLayer::new(timeout.duration()));
    }
    let app = app.with_state(state_ref);

    info!(?socket_addr, "invoke server waiting for requests");
    if only_lambda_apis {
        info!("");
        info!("the flag --only_lambda_apis is active, the lambda function will not be started by Cargo Lambda");
        info!("the lambda function will depend on the following environment variables");
        info!(
            "you MUST set these variables in the environment where you're running your function:"
        );
        info!("AWS_LAMBDA_FUNCTION_VERSION=1");
        info!("AWS_LAMBDA_FUNCTION_MEMORY_SIZE=4096");
        info!("AWS_LAMBDA_RUNTIME_API={}", runtime_addr);
        info!("AWS_LAMBDA_FUNCTION_NAME={DEFAULT_PACKAGE_FUNCTION}");
    } else {
        let print_start_info = if init_default_function {
            // This call ignores any error sending the action.
            // The function can still be lazy loaded later if there is any error.
            req_tx.send(Action::Init).await.is_err()
        } else {
            false
        };

        if print_start_info {
            info!("");
            info!("your function will start running when you send the first invoke request");
            info!("read the invoke guide if you don't know how to continue:");
            info!("https://www.cargo-lambda.info/commands/invoke.html");
        }
    }

    if let Err(error) = axum::Server::bind(&socket_addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(subsys.on_shutdown_requested())
        .await
    {
        if !error.is_incomplete_message() {
            return Err(axum::Error::new(error));
        }
    }

    Ok(())
}
