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
use std::{
    collections::{hash_map::Entry, HashMap, VecDeque},
    net::SocketAddr,
    process::{Command, Stdio},
    sync::Arc,
};
use tokio::sync::{mpsc, mpsc::Sender, oneshot, Mutex};
use tower_http::{
    catch_panic::CatchPanicLayer,
    request_id::{MakeRequestId, PropagateRequestIdLayer, RequestId, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing::{debug, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

#[derive(Args, Clone, Debug)]
#[clap(name = "start")]
pub struct Start {
    /// Address port where users send invoke requests
    #[clap(short = 'p', long, default_value = "9000")]
    invoke_port: u16,
}

struct InvokeRequest {
    function_name: String,
    req: Request<Body>,
    resp_tx: oneshot::Sender<Response<Body>>,
}

type ServerAddr = String;

impl Start {
    pub async fn run(&self) -> Result<()> {
        if which::which("cargo-watch").is_err() {
            let pb = crate::progress::Progress::start("Installing Cargo-watch...");
            let result = install_cargo_watch();
            let finish = if result.is_ok() {
                "Cargo-watch installed"
            } else {
                "Failed to install Cargo-watch"
            };
            pb.finish(finish);
            let _ = result?;
        }

        self.start_server().await
    }

    async fn start_server(&self) -> Result<()> {
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::new(
                std::env::var("RUST_LOG")
                    .unwrap_or_else(|_| "cargo_lambda=info,tower_http=info".into()),
            ))
            .with(tracing_subscriber::fmt::layer())
            .init();

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<InvokeRequest>(100);
        let (gc_tx, mut gc_rx) = mpsc::channel::<String>(100);

        let addr = SocketAddr::from(([127, 0, 0, 1], self.invoke_port));
        let server_addr: ServerAddr = format!("http://{addr}");
        let scheduler = Scheduler::new(server_addr, gc_tx);

        let scheduler_gc = scheduler.clone();
        let scheduler_clone = scheduler.clone();

        tokio::spawn(async move {
            while let Some(function_name) = gc_rx.recv().await {
                scheduler_gc.clean(&function_name).await;
            }
        });

        tokio::spawn(async move {
            while let Some(req) = cmd_rx.recv().await {
                scheduler.call(req).await;
            }
        });

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
                RequestUuidService::default(),
            ))
            .layer(PropagateRequestIdLayer::new(x_request_id))
            .layer(Extension(cmd_tx.clone()))
            .layer(Extension(scheduler_clone))
            .layer(Extension(resp_cache))
            .layer(TraceLayer::new_for_http())
            .layer(CatchPanicLayer::new());

        info!("invoke server listening on {}", addr);
        axum::Server::bind(&addr)
            .serve(app.into_make_service())
            .await
            .into_diagnostic()
    }
}

async fn invoke_handler(
    Extension(cmd_tx): Extension<Sender<InvokeRequest>>,
    Path(function_name): Path<String>,
    req: Request<Body>,
) -> Response<Body> {
    let (resp_tx, resp_rx) = oneshot::channel::<Response<Body>>();

    let req = InvokeRequest {
        function_name,
        req,
        resp_tx,
    };

    cmd_tx.send(req).await.ok().unwrap();
    resp_rx.await.unwrap()
}

async fn next_request(
    Extension(scheduler): Extension<Scheduler>,
    Extension(cache): Extension<ResponseCache>,
    Path(function_name): Path<String>,
    req: Request<Body>,
) -> Response<Body> {
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

    match scheduler.pop(&function_name).await {
        None => builder
            .status(StatusCode::NO_CONTENT)
            .body(Body::empty())
            .unwrap(),
        Some(invoke) => {
            debug!("processing request -- {req_id}");

            let (parts, body) = invoke.req.into_parts();

            let resp_tx = invoke.resp_tx;
            cache.push(&req_id, resp_tx).await;

            let headers = parts.headers;
            if let Some(h) = headers.get("lambda-runtime-client-context") {
                builder = builder.header("lambda-runtime-client-context", h);
            }
            if let Some(h) = headers.get("lambda-runtime-cognito-identity") {
                builder = builder.header("lambda-runtime-cognito-identity", h);
            }

            builder.status(StatusCode::OK).body(body).unwrap()
        }
    }
}

async fn next_invocation_response(
    Extension(cache): Extension<ResponseCache>,
    Path((_function_name, req_id)): Path<(String, String)>,
    req: Request<Body>,
) -> Response<Body> {
    respond_to_next_invocation(&cache, &req_id, req, StatusCode::OK).await
}

async fn next_invocation_error(
    Extension(cache): Extension<ResponseCache>,
    Path((_function_name, req_id)): Path<(String, String)>,
    req: Request<Body>,
) -> Response<Body> {
    respond_to_next_invocation(&cache, &req_id, req, StatusCode::INTERNAL_SERVER_ERROR).await
}

async fn respond_to_next_invocation(
    cache: &ResponseCache,
    req_id: &str,
    req: Request<Body>,
    response_status: StatusCode,
) -> Response<Body> {
    if let Some(resp_tx) = cache.pop(req_id).await {
        let (_, body) = req.into_parts();

        let resp = Response::builder()
            .status(response_status)
            .header("lambda-runtime-aws-request-id", req_id)
            .body(body)
            .unwrap();

        resp_tx.send(resp).unwrap();
    }

    Response::new(Body::empty())
}

#[derive(Clone)]
struct RequestQueue {
    inner: Arc<Mutex<VecDeque<InvokeRequest>>>,
}

impl RequestQueue {
    fn new() -> RequestQueue {
        RequestQueue {
            inner: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    async fn pop(&self) -> Option<InvokeRequest> {
        let mut queue = self.inner.lock().await;
        queue.pop_front()
    }

    async fn push(&self, req: InvokeRequest) {
        let mut queue = self.inner.lock().await;
        queue.push_back(req);
    }
}

#[derive(Clone)]
struct ResponseCache {
    inner: Arc<Mutex<HashMap<String, oneshot::Sender<Response<Body>>>>>,
}

impl ResponseCache {
    fn new() -> ResponseCache {
        ResponseCache {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn pop(&self, req_id: &str) -> Option<oneshot::Sender<Response<Body>>> {
        let mut cache = self.inner.lock().await;
        cache.remove(req_id)
    }

    async fn push(&self, req_id: &str, resp_tx: oneshot::Sender<Response<Body>>) {
        let mut cache = self.inner.lock().await;
        cache.insert(req_id.into(), resp_tx);
    }
}

#[derive(Clone)]
struct Scheduler {
    server_addr: ServerAddr,
    gc_tx: Sender<String>,
    inner: Arc<Mutex<HashMap<String, RequestQueue>>>,
}

impl Scheduler {
    fn new(server_addr: ServerAddr, gc_tx: Sender<String>) -> Scheduler {
        Scheduler {
            server_addr,
            gc_tx,
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    async fn call(&self, req: InvokeRequest) {
        let mut inner = self.inner.lock().await;
        let name = req.function_name.clone();
        let runtime_api = format!("{}/{}", &self.server_addr, &name);

        match inner.entry(name) {
            Entry::Vacant(v) => {
                let name = req.function_name.clone();

                let stack = RequestQueue::new();
                stack.push(req).await;
                v.insert(stack);

                let gc = self.gc_tx.clone();
                tokio::spawn(async move {
                    let _ = watch_project(&name, &runtime_api);
                    gc.send(name).await.unwrap();
                });
            }
            Entry::Occupied(o) => {
                o.into_mut().push(req).await;
            }
        }
    }

    async fn pop(&self, function_name: &str) -> Option<InvokeRequest> {
        let inner = self.inner.lock().await;
        let stack = match inner.get(function_name) {
            None => return None,
            Some(s) => s,
        };

        stack.pop().await
    }

    async fn clean(&self, function_name: &str) {
        let mut inner = self.inner.lock().await;
        inner.remove(function_name);
    }
}

#[derive(Clone, Copy, Default)]
struct RequestUuidService;

impl MakeRequestId for RequestUuidService {
    fn make_request_id<B>(&mut self, _request: &Request<B>) -> Option<RequestId> {
        let request_id = Uuid::new_v4().to_string().parse().unwrap();
        Some(RequestId::new(request_id))
    }
}

fn watch_project(name: &str, runtime_api: &str) -> Result<std::process::ExitStatus> {
    info!("Starting lambda function {name}");

    let mut child = Command::new("cargo")
        .args(["watch", "--", "cargo", "run", "--bin", name])
        .env("RUST_LOG", std::env::var("RUST_LOG").unwrap_or_default())
        .env("AWS_LAMBDA_RUNTIME_API", runtime_api)
        .env("AWS_LAMBDA_FUNCTION_NAME", name)
        .env("AWS_LAMBDA_FUNCTION_VERSION", "1")
        .env("AWS_LAMBDA_FUNCTION_MEMORY_SIZE", "4096")
        .spawn()
        .into_diagnostic()
        .wrap_err("Failed to run `cargo-watch`")?;

    child
        .wait()
        .into_diagnostic()
        .wrap_err("Failed to wait on cargo-watch process")
}

fn install_cargo_watch() -> Result<()> {
    let mut child = Command::new("cargo")
        .args(&["install", "cargo-watch"])
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .into_diagnostic()
        .wrap_err("Failed to run `cargo install cargo-watch`")?;

    let status = child
        .wait()
        .into_diagnostic()
        .wrap_err("Failed to wait on cargo process")?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
