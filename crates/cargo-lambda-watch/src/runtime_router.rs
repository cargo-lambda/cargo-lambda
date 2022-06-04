use crate::{requests::*, scheduler::*};
use axum::{
    body::Body,
    extract::{Extension, Path},
    http::{Request, StatusCode},
    response::Response,
    routing::{get, post},
    Router,
};
use tracing::debug;

pub(crate) const LAMBDA_RUNTIME_AWS_REQUEST_ID: &str = "lambda-runtime-aws-request-id";
pub(crate) const LAMBDA_RUNTIME_CLIENT_CONTEXT: &str = "lambda-runtime-client-context";
pub(crate) const LAMBDA_RUNTIME_COGNITO_IDENTITY: &str = "lambda-runtime-cognito-identity";
pub(crate) const LAMBDA_RUNTIME_DEADLINE_MS: &str = "lambda-runtime-deadline-ms";
pub(crate) const LAMBDA_RUNTIME_FUNCTION_ARN: &str = "lambda-runtime-invoked-function-arn";
pub(crate) const LAMBDA_RUNTIME_XRAY_TRACE_HEADER: &str = "lambda-runtime-trace-id";

pub(crate) fn routes() -> Router {
    Router::new()
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
            .header(LAMBDA_RUNTIME_AWS_REQUEST_ID, req_id)
            .body(body)
            .map_err(ServerError::ResponseBuild)?;

        resp_tx
            .send(resp)
            .map_err(|_| ServerError::SendFunctionMessage)?;
    }

    Ok(Response::new(Body::empty()))
}
