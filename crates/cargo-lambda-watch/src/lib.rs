use axum::{extract::Extension, http::header::HeaderName, Router};
use clap::{Args, ValueHint};
use miette::Result;
use opentelemetry::{
    global,
    sdk::{export::trace::stdout, trace, trace::Tracer},
};
use opentelemetry_aws::trace::XrayPropagator;
use std::{net::SocketAddr, path::PathBuf};
use tokio::time::Duration;
use tokio_graceful_shutdown::{SubsystemHandle, Toplevel};
use tower_http::{
    catch_panic::CatchPanicLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::{info, Subscriber};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::LookupSpan;

mod requests;
mod runtime_router;

mod scheduler;
use scheduler::*;
mod trigger_router;
mod watch_installer;

const RUNTIME_EMULATOR_PATH: &str = "/.rt";

#[derive(Args, Clone, Debug)]
#[clap(name = "watch", visible_alias = "start", trailing_var_arg = true)]
pub struct Watch {
    /// Avoid hot-reload
    #[clap(long)]
    no_reload: bool,

    /// Address port where users send invoke requests
    #[clap(short = 'p', long, default_value = "9000")]
    invoke_port: u16,

    /// Print OpenTelemetry traces after each function invocation
    #[clap(long)]
    print_traces: bool,

    /// Path to Cargo.toml
    #[clap(long, value_name = "PATH", parse(from_os_str), value_hint = ValueHint::FilePath)]
    #[clap(default_value = "Cargo.toml")]
    manifest_path: PathBuf,

    /// Features to pass to `cargo run`, separated by comma
    #[clap(long)]
    features: Option<String>,

    /// Arguments and flags to pass to `cargo watch`
    #[clap(value_hint = ValueHint::CommandWithArguments)]
    watch_args: Vec<String>,
}

impl Watch {
    #[tracing::instrument(skip(self), target = "cargo_lambda")]
    pub async fn run(&self) -> Result<()> {
        tracing::trace!(options = ?self, "watching project");

        if !self.no_reload && which::which("cargo-watch").is_err() {
            watch_installer::install().await?;
        }

        let port = self.invoke_port;
        let manifest_path = self.manifest_path.clone();
        let no_reload = self.no_reload;
        let watch_args = self.watch_args.clone();
        let features = self.features.clone();

        Toplevel::new()
            .start("Lambda server", move |s| {
                start_server(s, port, manifest_path, watch_args, features, no_reload)
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

async fn start_server(
    subsys: SubsystemHandle,
    invoke_port: u16,
    manifest_path: PathBuf,
    watch_args: Vec<String>,
    features: Option<String>,
    no_reload: bool,
) -> Result<(), axum::Error> {
    let addr = SocketAddr::from(([127, 0, 0, 1], invoke_port));
    let runtime_addr = format!("http://{addr}{RUNTIME_EMULATOR_PATH}");

    let req_cache = RequestCache::new(runtime_addr);
    let req_tx = init_scheduler(
        &subsys,
        req_cache.clone(),
        manifest_path,
        watch_args,
        features,
        no_reload,
    )
    .await;
    let resp_cache = ResponseCache::new();
    let x_request_id = HeaderName::from_static("lambda-runtime-aws-request-id");

    let app = Router::new()
        .merge(trigger_router::routes())
        .nest(RUNTIME_EMULATOR_PATH, runtime_router::routes())
        .layer(SetRequestIdLayer::new(
            x_request_id.clone(),
            MakeRequestUuid,
        ))
        .layer(PropagateRequestIdLayer::new(x_request_id))
        .layer(Extension(req_tx.clone()))
        .layer(Extension(req_cache))
        .layer(Extension(resp_cache))
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new());

    info!("invoke server listening on {}", addr);
    axum::Server::bind(&addr)
        .serve(app.into_make_service())
        .with_graceful_shutdown(subsys.on_shutdown_requested())
        .await
        .map_err(axum::Error::new)
}
