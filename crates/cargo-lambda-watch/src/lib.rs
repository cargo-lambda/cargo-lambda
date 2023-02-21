use axum::{extract::Extension, http::header::HeaderName, Router};
use cargo_lambda_invoke::DEFAULT_PACKAGE_FUNCTION;
use cargo_lambda_metadata::env::EnvOptions;
use clap::{Args, ValueHint};
use miette::{IntoDiagnostic, Result, WrapErr};
use opentelemetry::{
    global,
    sdk::{export::trace::stdout, trace, trace::Tracer},
};
use opentelemetry_aws::trace::XrayPropagator;
use std::{
    net::{IpAddr, SocketAddr},
    path::{PathBuf, Path},
    str::FromStr,
};
use tokio::time::Duration;
use tokio_graceful_shutdown::{SubsystemHandle, Toplevel};
use tower_http::{
    catch_panic::CatchPanicLayer,
    cors::CorsLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};

use tracing::{info, Subscriber, trace};
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

const RUNTIME_EMULATOR_PATH: &str = "/.rt";

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
    #[arg(long)]
    features: Option<String>,

    /// Enable release mode when the emulator starts
    #[arg(long)]
    release: bool,
}

impl Watch {
    #[tracing::instrument(skip(self), target = "cargo_lambda")]
    pub async fn run(&self) -> Result<()> {
        tracing::trace!(options = ?self, "watching project");

        let ip = IpAddr::from_str(&self.invoke_address)
            .into_diagnostic()
            .wrap_err("invalid invoke address")?;
        let addr = SocketAddr::from((ip, self.invoke_port));
        let ignore_changes = self.ignore_changes;
        let only_lambda_apis = self.only_lambda_apis;
        let cargo_options = self.cargo_options.clone();

        let base = dunce::canonicalize(".").into_diagnostic()?;
        let ignore_files = discover_ignore_files(&base).await;

        let env = self.env_options.lambda_environment()?;

        let watcher_config = WatcherConfig {
            base,
            ignore_files,
            ignore_changes,
            only_lambda_apis,
            manifest_path: cargo_options.manifest_path.clone(),
            env: env.variables().cloned().unwrap_or_default(),
            ..Default::default()
        };

        Toplevel::new()
            .start("Lambda server", move |s| {
                start_server(s, addr, cargo_options, watcher_config)
            })
            .catch_signals()
            .handle_shutdown_requests(Duration::from_millis(1000))
            .await
            .map_err(|e| miette::miette!("{}", e))
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

    let (mut env_ignore, env_ignore_errs) = ignore_files::from_environment(Some("CARGO_LAMBDA")).await;
    trace!(ignore_files = ?env_ignore, errors = ?env_ignore_errs, "discovered ignore files from environment variable");
    ignore_files.append(&mut env_ignore);

    let (mut origin_ignore, origin_ignore_errs) = ignore_files::from_origin(base).await;
    trace!(ignore_files = ?origin_ignore, errors = ?origin_ignore_errs, "discovered ignore files from origin");
    ignore_files.append(&mut origin_ignore);

    for parent in project_origins::origins(base).await {
        let (mut parent_ignore, parent_ignore_errs) = ignore_files::from_origin(&parent).await;
        trace!(parent = ?parent, ignore_files = ?parent_ignore, errors = ?parent_ignore_errs, "discovered ignore files from parent origin");
        ignore_files.append(&mut parent_ignore);
    }

    ignore_files
}

async fn start_server(
    subsys: SubsystemHandle,
    addr: SocketAddr,
    cargo_options: CargoOptions,
    watcher_config: WatcherConfig,
) -> Result<(), axum::Error> {
    let runtime_addr = format!("http://{addr}{RUNTIME_EMULATOR_PATH}");

    if watcher_config.only_lambda_apis {
        info!("the flag --only_lambda_apis is active, the lambda function will not be started by Cargo Lambda");
        info!("the lambda function will depend on the following environment variables");
        info!(
            "you MUST set these variables in the environment where you're running your function:"
        );
        info!("AWS_LAMBDA_FUNCTION_VERSION=1");
        info!("AWS_LAMBDA_FUNCTION_MEMORY_SIZE=4096");
        info!("AWS_LAMBDA_RUNTIME_API={}", &runtime_addr);
        info!("AWS_LAMBDA_FUNCTION_NAME={DEFAULT_PACKAGE_FUNCTION}");
    }

    let ext_cache = ExtensionCache::default();
    let req_cache = RequestCache::new(runtime_addr);
    let runtime_state = RuntimeState {
        req_cache: req_cache.clone(),
        ext_cache: ext_cache.clone(),
    };

    let req_tx = init_scheduler(&subsys, runtime_state, cargo_options, watcher_config).await;
    let resp_cache = ResponseCache::new();
    let x_request_id = HeaderName::from_static("lambda-runtime-aws-request-id");

    let app = Router::new()
        .merge(trigger_router::routes())
        .nest(RUNTIME_EMULATOR_PATH, runtime::routes())
        .layer(SetRequestIdLayer::new(
            x_request_id.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(x_request_id))
        .layer(Extension(ext_cache))
        .layer(Extension(req_tx.clone()))
        .layer(Extension(req_cache))
        .layer(Extension(resp_cache))
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new())
        .layer(CorsLayer::permissive());

    info!("invoke server listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(subsys.on_shutdown_requested())
        .await
        .map_err(axum::Error::new)
}
