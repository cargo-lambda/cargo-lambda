use crate::RefRuntimeState;
use axum::{
    Router,
    routing::{get, post, put},
};

pub(crate) mod extensions_router;
use extensions_router::*;

mod functions_router;
use functions_router::*;

pub(crate) const LAMBDA_RUNTIME_AWS_REQUEST_ID: &str = "lambda-runtime-aws-request-id";
pub(crate) const LAMBDA_RUNTIME_XRAY_TRACE_HEADER: &str = "lambda-runtime-trace-id";

pub(crate) fn routes() -> Router<RefRuntimeState> {
    Router::new()
        .route("/2020-01-01/extension/register", post(register_extension))
        // secondary route is for internal extensions
        .route(
            "/:function_name/2020-01-01/extension/register",
            post(register_extension),
        )
        .route(
            "/2020-01-01/extension/event/next",
            get(next_extension_event),
        )
        // secondary route is for internal extensions
        .route(
            "/:function_name/2020-01-01/extension/event/next",
            get(next_extension_event),
        )
        .route("/2020-08-15/logs", put(subcribe_extension_events))
        .route("/2022-07-01/telemetry", put(subcribe_extension_events))
        .route(
            "/:function_name/2018-06-01/runtime/invocation/next",
            get(next_request),
        )
        .route(
            "/2018-06-01/runtime/invocation/next",
            get(bare_next_request),
        )
        .route(
            "/:function_name/2018-06-01/runtime/invocation/:req_id/response",
            post(next_invocation_response),
        )
        .route(
            "/2018-06-01/runtime/invocation/:req_id/response",
            post(bare_next_invocation_response),
        )
        .route(
            "/:function_name/2018-06-01/runtime/invocation/:req_id/error",
            post(next_invocation_error),
        )
        .route(
            "/2018-06-01/runtime/invocation/:req_id/error",
            post(bare_next_invocation_error),
        )
        .route(
            "/:function_name/2018-06-01/runtime/init/error",
            post(init_error),
        )
        .route("/2018-06-01/runtime/init/error", post(bare_init_error))
}
