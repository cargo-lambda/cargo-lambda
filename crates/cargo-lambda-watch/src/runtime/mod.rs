use axum::Router;

pub(crate) mod extensions_router;
mod functions_router;

pub(crate) const LAMBDA_RUNTIME_AWS_REQUEST_ID: &str = "lambda-runtime-aws-request-id";
pub(crate) const LAMBDA_RUNTIME_XRAY_TRACE_HEADER: &str = "lambda-runtime-trace-id";

pub(crate) fn routes() -> Router {
    Router::new()
        .merge(extensions_router::routes())
        .merge(functions_router::routes())
}
