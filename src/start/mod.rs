use axum::{
    body::Body,
    extract::{Extension, Path},
    http::{header::HeaderName, Request, StatusCode},
    response::Response,
    routing::{get, post},
    Router,
};
use clap::Args;
use miette::{IntoDiagnostic, Result, WrapErr};
use std::{net::SocketAddr, process::Stdio};
use tokio::{
    process::Command,
    sync::{mpsc::Sender, oneshot},
    time::Duration,
};
use tokio_graceful_shutdown::{SubsystemHandle, Toplevel};
use tower_http::{
    catch_panic::CatchPanicLayer,
    request_id::{PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::{debug, info};
use uuid::Uuid;

mod requests;
use requests::*;
mod scheduler;
use scheduler::*;
mod trace;
use trace::*;

#[derive(Args, Clone, Debug)]
#[clap(name = "start")]
pub struct Start {
    /// Address port where users send invoke requests
    #[clap(short = 'p', long, default_value = "9000")]
    invoke_port: u16,
}

impl Start {
    pub async fn run(&self) -> Result<()> {
        if which::which("cargo-watch").is_err() {
            let pb = crate::progress::Progress::start("Installing Cargo-watch...");
            let result = install_cargo_watch().await;
            let finish = if result.is_ok() {
                "Cargo-watch installed"
            } else {
                "Failed to install Cargo-watch"
            };
            pb.finish(finish);
            let _ = result?;
        }

        let port = self.invoke_port;

        Toplevel::new()
            .start("Lambda server", move |s| start_server(s, port))
            .catch_signals()
            .handle_shutdown_requests(Duration::from_millis(1000))
            .await
            .map_err(|e| miette::miette!("{}", e))
    }
}

async fn start_server(subsys: SubsystemHandle, invoke_port: u16) -> Result<(), axum::Error> {
    init_tracing();

    let addr = SocketAddr::from(([127, 0, 0, 1], invoke_port));
    let server_addr = format!("http://{addr}");

    let req_cache = RequestCache::new(server_addr);
    let req_tx = init_scheduler(&subsys, req_cache.clone()).await;
    let resp_cache = ResponseCache::new();
    let x_request_id = HeaderName::from_static("lambda-runtime-aws-request-id");

    let app = Router::new()
        .route(
            "/2015-03-31/functions/:function_name/invocations",
            post(invoke_handler),
        )
        .route(
            "/:function_name/2018-06-01/runtime/invocation/next",
            get(next_request),
        )
        .route(
            "/:function_name/2018-06-01/runtime/invocation/:req_id/response",
            post(next_invocation_response),
        )
        .route(
            "/:function_name/2018-06-01/runtime/invocation/:req_id/error",
            post(next_invocation_error),
        )
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

async fn invoke_handler(
    Extension(cmd_tx): Extension<Sender<InvokeRequest>>,
    Path(function_name): Path<String>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    let (resp_tx, resp_rx) = oneshot::channel::<Response<Body>>();

    let req = InvokeRequest {
        function_name,
        req,
        resp_tx,
    };

    cmd_tx
        .send(req)
        .await
        .map_err(|e| ServerError::SendInvokeMessage(Box::new(e)))?;

    resp_rx.await.map_err(ServerError::ReceiveFunctionMessage)
}

async fn next_request(
    Extension(req_cache): Extension<RequestCache>,
    Extension(resp_cache): Extension<ResponseCache>,
    Path(function_name): Path<String>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    let req_id = req
        .headers()
        .get("lambda-runtime-aws-request-id")
        .and_then(|h| h.to_str().ok())
        .map(|h| h.to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let mut builder = Response::builder()
        .header("lambda-runtime-aws-request-id", &req_id)
        .header("lambda-runtime-deadline-ms", 600_000_u32)
        .header("lambda-runtime-invoked-function-arn", "function-arn");

    let resp = match req_cache.pop(&function_name).await {
        None => builder.status(StatusCode::NO_CONTENT).body(Body::empty()),
        Some(invoke) => {
            debug!(req_id = ?req_id, "processing request");

            let (parts, body) = invoke.req.into_parts();

            let resp_tx = invoke.resp_tx;
            resp_cache.push(&req_id, resp_tx).await;

            let headers = parts.headers;
            if let Some(h) = headers.get("lambda-runtime-client-context") {
                builder = builder.header("lambda-runtime-client-context", h);
            }
            if let Some(h) = headers.get("lambda-runtime-cognito-identity") {
                builder = builder.header("lambda-runtime-cognito-identity", h);
            }

            builder.status(StatusCode::OK).body(body)
        }
    };

    resp.map_err(ServerError::ResponseBuild)
}

async fn next_invocation_response(
    Extension(cache): Extension<ResponseCache>,
    Path((_function_name, req_id)): Path<(String, String)>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    respond_to_next_invocation(&cache, &req_id, req, StatusCode::OK).await
}

async fn next_invocation_error(
    Extension(cache): Extension<ResponseCache>,
    Path((_function_name, req_id)): Path<(String, String)>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    respond_to_next_invocation(&cache, &req_id, req, StatusCode::INTERNAL_SERVER_ERROR).await
}

async fn respond_to_next_invocation(
    cache: &ResponseCache,
    req_id: &str,
    req: Request<Body>,
    response_status: StatusCode,
) -> Result<Response<Body>, ServerError> {
    if let Some(resp_tx) = cache.pop(req_id).await {
        let (_, body) = req.into_parts();

        let resp = Response::builder()
            .status(response_status)
            .header("lambda-runtime-aws-request-id", req_id)
            .body(body)
            .map_err(ServerError::ResponseBuild)?;

        resp_tx
            .send(resp)
            .map_err(|_| ServerError::SendFunctionMessage)?;
    }

    Ok(Response::new(Body::empty()))
}

async fn install_cargo_watch() -> Result<()> {
    let mut child = Command::new("cargo")
        .args(&["install", "cargo-watch"])
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .into_diagnostic()
        .wrap_err("Failed to run `cargo install cargo-watch`")?;

    let status = child
        .wait()
        .await
        .into_diagnostic()
        .wrap_err("Failed to wait on cargo process")?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
