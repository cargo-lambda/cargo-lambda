use axum::{
    body::Body,
    extract::{Extension, Path},
    http::{header::HeaderName, Request, StatusCode},
    response::Response,
    routing::{get, post},
    Router,
};
use clap::{Args, ValueHint};
use miette::{IntoDiagnostic, Result, WrapErr};
use opentelemetry::{
    global,
    trace::{TraceContextExt, Tracer},
    Context, KeyValue,
};
use std::{collections::HashMap, net::SocketAddr, path::PathBuf, process::Stdio};
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

mod requests;
use requests::*;
mod scheduler;
use scheduler::*;
mod trace;
use trace::*;

const AWS_XRAY_TRACE_HEADER: &str = "x-amzn-trace-id";
const LAMBDA_RUNTIME_XRAY_TRACE_HEADER: &str = "lambda-runtime-trace-id";
const LAMBDA_RUNTIME_COGNITO_IDENTITY: &str = "lambda-runtime-cognito-identity";
const LAMBDA_RUNTIME_CLIENT_CONTEXT: &str = "lambda-runtime-client-context";
const LAMBDA_RUNTIME_AWS_REQUEST_ID: &str = "lambda-runtime-aws-request-id";
const LAMBDA_RUNTIME_DEADLINE_MS: &str = "lambda-runtime-deadline-ms";
const LAMBDA_RUNTIME_FUNCTION_ARN: &str = "lambda-runtime-invoked-function-arn";

#[derive(Args, Clone, Debug)]
#[clap(name = "start")]
pub struct Start {
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
        let print_traces = self.print_traces;
        let manifest_path = self.manifest_path.clone();

        Toplevel::new()
            .start("Lambda server", move |s| {
                start_server(s, port, print_traces, manifest_path)
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
) -> Result<(), axum::Error> {
    init_tracing(print_traces);

    let addr = SocketAddr::from(([127, 0, 0, 1], invoke_port));
    let server_addr = format!("http://{addr}");

    let req_cache = RequestCache::new(server_addr);
    let req_tx = init_scheduler(&subsys, req_cache.clone(), manifest_path).await;
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
    let mut req = req;
    let headers = req.headers_mut();

    let span = global::tracer("cargo-lambda/emulator").start("invoke request");
    let cx = Context::current_with_span(span);

    let mut injector = HashMap::new();
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut injector);
    });
    let xray_header = injector
        .get(AWS_XRAY_TRACE_HEADER)
        .expect("x-amzn-trace-id header not injected by the propagator") // this is Infaliable
        .parse()
        .expect("x-amzn-trace-id header is not in the expected format"); // this is Infaliable
    headers.insert(LAMBDA_RUNTIME_XRAY_TRACE_HEADER, xray_header);

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

    let resp = resp_rx.await.map_err(ServerError::ReceiveFunctionMessage)?;

    cx.span().add_event(
        "function call completed",
        vec![KeyValue::new("status", resp.status().to_string())],
    );

    Ok(resp)
}

async fn next_request(
    Extension(req_cache): Extension<RequestCache>,
    Extension(resp_cache): Extension<ResponseCache>,
    Path(function_name): Path<String>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    let req_id = req
        .headers()
        .get(LAMBDA_RUNTIME_AWS_REQUEST_ID)
        .expect("missing request id");

    let mut builder = Response::builder()
        .header(LAMBDA_RUNTIME_AWS_REQUEST_ID, req_id)
        .header(LAMBDA_RUNTIME_DEADLINE_MS, 600_000_u32)
        .header(LAMBDA_RUNTIME_FUNCTION_ARN, "function-arn");

    let resp = match req_cache.pop(&function_name).await {
        None => builder.status(StatusCode::NO_CONTENT).body(Body::empty()),
        Some(invoke) => {
            let req_id = req_id
                .to_str()
                .map_err(ServerError::InvalidRequestIdHeader)?;

            debug!(req_id = ?req_id, "processing request");

            let (parts, body) = invoke.req.into_parts();

            let resp_tx = invoke.resp_tx;
            resp_cache.push(req_id, resp_tx).await;

            let headers = parts.headers;
            if let Some(h) = headers.get(LAMBDA_RUNTIME_CLIENT_CONTEXT) {
                builder = builder.header(LAMBDA_RUNTIME_CLIENT_CONTEXT, h);
            }
            if let Some(h) = headers.get(LAMBDA_RUNTIME_COGNITO_IDENTITY) {
                builder = builder.header(LAMBDA_RUNTIME_COGNITO_IDENTITY, h);
            }
            if let Some(h) = headers.get(LAMBDA_RUNTIME_XRAY_TRACE_HEADER) {
                builder = builder.header(LAMBDA_RUNTIME_XRAY_TRACE_HEADER, h);
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
