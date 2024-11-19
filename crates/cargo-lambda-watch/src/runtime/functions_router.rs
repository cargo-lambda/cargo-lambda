use crate::{
    error::ServerError,
    requests::*,
    runtime::LAMBDA_RUNTIME_XRAY_TRACE_HEADER,
    state::{RequestCache, ResponseCache},
    RefRuntimeState,
};
use axum::{
    body::Body,
    extract::{Path, Request, State},
    http::StatusCode,
    response::Response,
};
use base64::{engine::general_purpose as b64, Engine as _};
use cargo_lambda_metadata::DEFAULT_PACKAGE_FUNCTION;
use http::request::Parts;
use tracing::debug;

use super::LAMBDA_RUNTIME_AWS_REQUEST_ID;

pub(crate) const LAMBDA_RUNTIME_CLIENT_CONTEXT: &str = "lambda-runtime-client-context";
pub(crate) const LAMBDA_RUNTIME_COGNITO_IDENTITY: &str = "lambda-runtime-cognito-identity";
pub(crate) const LAMBDA_RUNTIME_DEADLINE_MS: &str = "lambda-runtime-deadline-ms";
pub(crate) const LAMBDA_RUNTIME_FUNCTION_ARN: &str = "lambda-runtime-invoked-function-arn";

pub(crate) async fn next_request(
    State(state): State<RefRuntimeState>,
    Path(function_name): Path<String>,
    parts: Parts,
) -> Result<Response<Body>, ServerError> {
    process_next_request(&state, &function_name, parts).await
}

pub(crate) async fn bare_next_request(
    State(state): State<RefRuntimeState>,
    parts: Parts,
) -> Result<Response<Body>, ServerError> {
    process_next_request(&state, DEFAULT_PACKAGE_FUNCTION, parts).await
}

pub(crate) async fn process_next_request(
    state: &RefRuntimeState,
    function_name: &str,
    parts: Parts,
) -> Result<Response<Body>, ServerError> {
    let function_name = if function_name.is_empty() {
        DEFAULT_PACKAGE_FUNCTION
    } else {
        function_name
    };

    let req_id = parts
        .headers
        .get(LAMBDA_RUNTIME_AWS_REQUEST_ID)
        .expect("missing request id");

    let mut builder = Response::builder()
        .header(LAMBDA_RUNTIME_AWS_REQUEST_ID, req_id)
        .header(LAMBDA_RUNTIME_DEADLINE_MS, 600_000_u32)
        .header(LAMBDA_RUNTIME_FUNCTION_ARN, "function-arn");

    let resp = match state.req_cache.pop(function_name).await {
        None => builder.status(StatusCode::NO_CONTENT).body(Body::empty()),
        Some(invoke) => {
            let req_id = req_id
                .to_str()
                .map_err(ServerError::InvalidRequestIdHeader)?;

            debug!(req_id = ?req_id, function = ?function_name, "processing request");
            let next_event = NextEvent::invoke(req_id, &invoke);
            state.ext_cache.send_event(next_event).await?;

            let (parts, body) = invoke.req.into_parts();

            let resp_tx = invoke.resp_tx;
            state.res_cache.push(req_id, resp_tx).await;

            let headers = parts.headers;
            if let Some(h) = headers.get(LAMBDA_RUNTIME_CLIENT_CONTEXT) {
                let ctx = b64::STANDARD.encode(h.as_bytes());
                let ctx = ctx.as_bytes();
                if ctx.len() > 3583 {
                    return Err(ServerError::InvalidClientContext(ctx.len()));
                }
                builder = builder.header(LAMBDA_RUNTIME_CLIENT_CONTEXT, ctx);
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

pub(crate) async fn next_invocation_response(
    State(state): State<RefRuntimeState>,
    Path((_function_name, req_id)): Path<(String, String)>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    respond_to_next_invocation(&state.res_cache, &req_id, req, StatusCode::OK).await
}

pub(crate) async fn bare_next_invocation_response(
    State(state): State<RefRuntimeState>,
    Path(req_id): Path<String>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    respond_to_next_invocation(&state.res_cache, &req_id, req, StatusCode::OK).await
}

pub(crate) async fn next_invocation_error(
    State(state): State<RefRuntimeState>,
    Path((_function_name, req_id)): Path<(String, String)>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    respond_to_next_invocation(
        &state.res_cache,
        &req_id,
        req,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await
}

pub(crate) async fn bare_next_invocation_error(
    State(state): State<RefRuntimeState>,
    Path(req_id): Path<String>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    respond_to_next_invocation(
        &state.res_cache,
        &req_id,
        req,
        StatusCode::INTERNAL_SERVER_ERROR,
    )
    .await
}

async fn respond_to_next_invocation(
    cache: &ResponseCache,
    req_id: &str,
    mut req: Request<Body>,
    response_status: StatusCode,
) -> Result<Response<Body>, ServerError> {
    if let Some(resp_tx) = cache.pop(req_id).await {
        req.extensions_mut().insert(response_status);

        resp_tx
            .send(req)
            .map_err(|_| ServerError::SendFunctionMessage)?;
    }

    Ok(Response::new(Body::empty()))
}

pub(crate) async fn init_error(
    State(state): State<RefRuntimeState>,
    Path(_function_name): Path<String>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    respond_to_invocation(&state.req_cache, req, StatusCode::OK).await
}

pub(crate) async fn bare_init_error(
    State(state): State<RefRuntimeState>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    respond_to_invocation(&state.req_cache, req, StatusCode::OK).await
}

async fn respond_to_invocation(
    cache: &RequestCache,
    mut req: Request<Body>,
    response_status: StatusCode,
) -> Result<Response<Body>, ServerError> {
    let keys = cache.keys().await;

    if let Some(key) = keys.first() {
        if let Some(invoke_request) = cache.pop(key).await {
            req.extensions_mut().insert(response_status);

            invoke_request
                .resp_tx
                .send(req)
                .map_err(|_| ServerError::SendFunctionMessage)?;
        }
    }

    Ok(Response::new(Body::empty()))
}
