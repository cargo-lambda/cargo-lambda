use axum::{Router, extract::Extension, http::header::HeaderName};
use bytes::Bytes;
use cargo_lambda_metadata::{
    DEFAULT_PACKAGE_FUNCTION,
    cargo::{
        CargoMetadata, CargoPackage, filter_binary_targets_from_metadata, kind_bin_filter,
        selected_bin_filter, watch::Watch,
    },
    env::SystemEnvExtractor,
    lambda::Timeout,
};
use cargo_lambda_remote::tls::TlsOptions;
use cargo_options::Run as CargoOptions;
use http::StatusCode;
use http_body_util::{BodyExt, combinators::BoxBody};
use hyper::{Request, Response, body::Incoming, client::conn::http1, service::service_fn};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
};
use miette::{IntoDiagnostic, Result, WrapErr};
use opentelemetry::{
    global,
    sdk::{export::trace::stdout, trace, trace::Tracer},
};
use opentelemetry_aws::trace::XrayPropagator;
use rustls::ServerConfig;
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr},
    path::Path,
    str::FromStr,
    sync::Arc,
};
use tokio::{
    net::{TcpListener, TcpStream},
    pin,
    time::Duration,
};
use tokio_graceful_shutdown::{SubsystemBuilder, SubsystemHandle, Toplevel};
use tokio_rustls::TlsAcceptor;
use tokio_util::task::TaskTracker;
use tower_http::{
    catch_panic::CatchPanicLayer,
    cors::CorsLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    timeout::TimeoutLayer,
    trace::TraceLayer,
};
use tracing::{Subscriber, error, info};
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::registry::LookupSpan;

mod error;
pub mod eventstream;
mod instance_pool;
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

#[tracing::instrument(target = "cargo_lambda")]
pub async fn run(
    config: &Watch,
    base_env: &HashMap<String, String>,
    metadata: &CargoMetadata,
    color: &str,
) -> Result<()> {
    tracing::trace!("watching project");

    let manifest_path = config.manifest_path();

    let mut cargo_options = config.cargo_opts.clone();
    cargo_options.color = Some(color.into());
    if cargo_options.manifest_path.is_none() {
        cargo_options.manifest_path = Some(manifest_path.clone());
    }

    let base = dunce::canonicalize(".").into_diagnostic()?;
    let ignore_files = watcher::ignore::discover_files(&base, SystemEnvExtractor).await;

    let env = config.lambda_environment(base_env).into_diagnostic()?;

    let package_filter = if !cargo_options.packages.is_empty() {
        let packages = cargo_options.packages.clone();
        Some(move |p: &&CargoPackage| packages.contains(&p.name))
    } else {
        None
    };

    let binary_filter = if config.cargo_opts.bin.is_empty() {
        Box::new(kind_bin_filter)
    } else {
        selected_bin_filter(config.cargo_opts.bin.clone())
    };

    let binary_packages =
        filter_binary_targets_from_metadata(metadata, binary_filter, package_filter);

    if binary_packages.is_empty() {
        Err(ServerError::NoBinaryPackages)?;
    }

    let watcher_config = WatcherConfig {
        base,
        ignore_files,
        env,
        ignore_changes: config.ignore_changes,
        only_lambda_apis: config.only_lambda_apis,
        manifest_path: manifest_path.clone(),
        wait: config.wait,
        ..Default::default()
    };

    let runtime_state = build_runtime_state(config, &manifest_path, binary_packages)?;

    let disable_cors = config.disable_cors;
    let timeout = config.timeout.clone();
    let tls_options = config.tls_options.clone();

    let _ = Toplevel::new(move |s| async move {
        s.start(SubsystemBuilder::new("Lambda server", move |s| {
            start_server(
                s,
                runtime_state,
                cargo_options,
                watcher_config,
                tls_options,
                disable_cors,
                timeout,
            )
        }));
    })
    .catch_signals()
    .handle_shutdown_requests(Duration::from_secs(1))
    .await;

    Ok(())
}

pub fn xray_layer<S>(config: &Watch) -> OpenTelemetryLayer<S, Tracer>
where
    S: Subscriber + for<'span> LookupSpan<'span>,
{
    global::set_text_map_propagator(XrayPropagator::default());

    let builder = stdout::new_pipeline().with_trace_config(
        trace::config()
            .with_sampler(trace::Sampler::AlwaysOn)
            .with_id_generator(trace::XrayIdGenerator::default()),
    );
    let tracer = if config.print_traces {
        builder.install_simple()
    } else {
        builder.with_writer(std::io::sink()).install_simple()
    };
    tracing_opentelemetry::layer().with_tracer(tracer)
}

fn build_runtime_state(
    config: &Watch,
    manifest_path: &Path,
    binary_packages: HashSet<String>,
) -> Result<RuntimeState> {
    let ip = IpAddr::from_str(&config.invoke_address)
        .into_diagnostic()
        .wrap_err("invalid invoke address")?;
    let (runtime_port, proxy_addr) = if config.tls_options.is_secure() {
        (
            config.invoke_port + 1,
            Some(SocketAddr::from((ip, config.invoke_port))),
        )
    } else {
        (config.invoke_port, None)
    };
    let runtime_addr = SocketAddr::from((ip, runtime_port));

    Ok(RuntimeState::new(
        runtime_addr,
        proxy_addr,
        manifest_path.to_path_buf(),
        config.only_lambda_apis,
        binary_packages,
        config.router.clone(),
        config.concurrency,
    ))
}

async fn start_server(
    subsys: SubsystemHandle,
    runtime_state: RuntimeState,
    cargo_options: CargoOptions,
    watcher_config: WatcherConfig,
    tls_options: TlsOptions,
    disable_cors: bool,
    timeout: Option<Timeout>,
) -> Result<()> {
    let only_lambda_apis = watcher_config.only_lambda_apis;
    let init_default_function =
        runtime_state.is_default_function_enabled() && watcher_config.send_function_init();

    let (runtime_addr, proxy_addr, runtime_url) = runtime_state.addresses();

    let x_request_id = HeaderName::from_static("lambda-runtime-aws-request-id");
    let req_tx = init_scheduler(
        &subsys,
        runtime_state.clone(),
        cargo_options,
        watcher_config,
    );

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
        app = app.layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            timeout.duration(),
        ));
    }
    let app = app.with_state(state_ref);

    if only_lambda_apis {
        info!("");
        info!(
            "the flag --only_lambda_apis is active, the lambda function will not be started by Cargo Lambda"
        );
        info!("the lambda function will depend on the following environment variables");
        info!(
            "you MUST set these variables in the environment where you're running your function:"
        );
        info!("AWS_LAMBDA_FUNCTION_VERSION=1");
        info!("AWS_LAMBDA_FUNCTION_MEMORY_SIZE=4096");
        info!("AWS_LAMBDA_RUNTIME_API={}", runtime_url);
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

    let tls_config = tls_options.server_config()?;
    let tls_tracker = TaskTracker::new();

    if let (Some(tls_config), Some(proxy_addr)) = (tls_config, proxy_addr) {
        let tls_tracker = tls_tracker.clone();

        subsys.start(SubsystemBuilder::new("TLS proxy", move |s| {
            start_tls_proxy(s, tls_tracker, tls_config, proxy_addr, runtime_addr)
        }));
    }

    info!(?runtime_addr, "starting Runtime server");
    let out = axum::serve(
        TcpListener::bind(runtime_addr).await.into_diagnostic()?,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move {
        subsys.on_shutdown_requested().await;
    })
    .await;

    if let Err(error) = out {
        error!(error = ?error, "failed to serve HTTP requests");
    }

    tls_tracker.close();
    tls_tracker.wait().await;

    Ok(())
}

async fn start_tls_proxy(
    subsys: SubsystemHandle,
    connection_tracker: TaskTracker,
    tls_config: ServerConfig,
    proxy_addr: SocketAddr,
    runtime_addr: SocketAddr,
) -> Result<()> {
    info!(
        ?proxy_addr,
        "starting TLS server, use this address to send secure requests to the runtime"
    );

    let acceptor = TlsAcceptor::from(Arc::new(tls_config));

    let listener = TcpListener::bind(proxy_addr).await.into_diagnostic()?;

    let addr = Arc::new(runtime_addr);

    loop {
        let (stream, _) = listener.accept().await.into_diagnostic()?;
        let acceptor = acceptor.clone();

        let addr = addr.clone();

        connection_tracker.spawn({
            let cancellation_token = subsys.create_cancellation_token();
            let connection_tracker = connection_tracker.clone();

            async move {
                let hyper_service = service_fn(move |request: Request<Incoming>| {
                    proxy(connection_tracker.clone(), request, addr.clone())
                });

                let tls_stream = match acceptor.accept(stream).await {
                    Ok(tls_stream) => tls_stream,
                    Err(e) => {
                        error!(error = ?e, "Failed to accept TLS connection");
                        return Err(e).into_diagnostic();
                    }
                };

                let builder = Builder::new(TokioExecutor::new());
                let conn = builder.serve_connection(TokioIo::new(tls_stream), hyper_service);

                pin!(conn);

                let result = tokio::select! {
                    res = conn.as_mut() => res,
                    _ = cancellation_token.cancelled() => {
                        conn.as_mut().graceful_shutdown();
                        conn.await
                    }
                };

                if let Err(e) = result {
                    error!(error = ?e, "Failed to serve connection");
                }

                Ok(())
            }
        });
    }
}

async fn proxy(
    connection_tracker: TaskTracker,
    req: Request<Incoming>,
    addr: Arc<SocketAddr>,
) -> Result<Response<BoxBody<Bytes, hyper::Error>>, hyper::Error> {
    let stream = TcpStream::connect(&*addr).await.unwrap();
    let io = TokioIo::new(stream);

    let (mut sender, conn) = http1::Builder::new()
        .preserve_header_case(true)
        .title_case_headers(true)
        .handshake(io)
        .await?;

    connection_tracker.spawn(async move {
        if let Err(err) = conn.await {
            println!("Connection failed: {err:?}");
        }
    });

    let resp = sender.send_request(req).await?;
    Ok(resp.map(|b| b.boxed()))
}
