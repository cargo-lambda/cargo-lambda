use axum::{extract::Extension, http::header::HeaderName, Router};
use clap::{Args, ValueHint};
use miette::Result;
use std::{net::SocketAddr, path::PathBuf};
use tokio::time::Duration;
use tokio_graceful_shutdown::{SubsystemHandle, Toplevel};
use tower_http::{
    catch_panic::CatchPanicLayer,
    request_id::{PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::info;

mod requests;
mod runtime_router;

mod scheduler;
use scheduler::*;
mod trace;
use trace::*;
mod trigger_router;
mod watch_installer;

const RUNTIME_EMULATOR_PATH: &str = "/.rt";

#[derive(Args, Clone, Debug)]
#[clap(name = "watch", visible_alias = "start")]
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
    pub manifest_path: PathBuf,
}

impl Watch {
    pub async fn run(&self) -> Result<()> {
        if !self.no_reload && which::which("cargo-watch").is_err() {
            watch_installer::install().await?;
        }

        let port = self.invoke_port;
        let print_traces = self.print_traces;
        let manifest_path = self.manifest_path.clone();
        let no_reload = self.no_reload;

        Toplevel::new()
            .start("Lambda server", move |s| {
                start_server(s, port, print_traces, manifest_path, no_reload)
            })
            .catch_signals()
            .handle_shutdown_requests(Duration::from_millis(1000))
            .await
            .map_err(|e| miette::miette!("{}", e))
    }
}

async fn start_server(
    subsys: SubsystemHandle,
    invoke_port: u16,
    print_traces: bool,
    manifest_path: PathBuf,
    no_reload: bool,
) -> Result<(), axum::Error> {
    init_tracing(print_traces);

    let addr = SocketAddr::from(([127, 0, 0, 1], invoke_port));
    let runtime_addr = format!("http://{addr}{RUNTIME_EMULATOR_PATH}");

    let req_cache = RequestCache::new(runtime_addr);
    let req_tx = init_scheduler(&subsys, req_cache.clone(), manifest_path, no_reload).await;
    let resp_cache = ResponseCache::new();
    let x_request_id = HeaderName::from_static("lambda-runtime-aws-request-id");

    let app = Router::new()
        .merge(trigger_router::routes())
        .nest(RUNTIME_EMULATOR_PATH, runtime_router::routes())
        .layer(SetRequestIdLayer::new(
            x_request_id.clone(),
            RequestUuidService,
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
