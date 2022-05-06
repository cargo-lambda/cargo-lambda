use crate::error::ServerError;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use http_api_problem::ApiError;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot::Sender;

pub(crate) const AWS_XRAY_TRACE_HEADER: &str = "x-amzn-trace-id";

#[derive(Debug)]
pub struct InvokeRequest {
    pub function_name: String,
    pub req: Request<Body>,
    pub resp_tx: Sender<Response<Body>>,
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let api_error = ApiError::builder(StatusCode::INTERNAL_SERVER_ERROR)
            .message(self.to_string())
            .finish();

        (
            api_error.status(),
            api_error.into_http_api_problem().json_string(),
        )
            .into_response()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct EventsRequest {
    pub events: Vec<String>,
}

/// Request tracing information
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracing {
    /// The type of tracing exposed to the extension
    pub r#type: String,
    /// The span value
    pub value: String,
}
/// Event received when there is a new Lambda invocation.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InvokeEvent {
    /// The time that the function times out
    pub deadline_ms: u64,
    /// The ID assigned to the Lambda request
    pub request_id: String,
    /// The function's Amazon Resource Name
    pub invoked_function_arn: String,
    /// The request tracing information
    pub tracing: Tracing,
}

/// Event received when a Lambda function shuts down.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShutdownEvent {
    /// The reason why the function terminates
    /// It can be SPINDOWN, TIMEOUT, or FAILURE
    pub shutdown_reason: String,
    /// The time that the function times out
    pub deadline_ms: u64,
}

/// Event that the extension receives in
/// either the INVOKE or SHUTDOWN phase
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "UPPERCASE", tag = "eventType")]
pub enum NextEvent {
    /// Payload when the event happens in the INVOKE phase
    Invoke(InvokeEvent),
    /// Payload when the event happens in the SHUTDOWN phase
    Shutdown(ShutdownEvent),
}

impl NextEvent {
    pub fn type_queue(&self) -> &str {
        match self {
            Self::Invoke(_) => "INVOKE",
            Self::Shutdown(_) => "SHUTDOWN",
        }
    }
}

impl From<(&str, &InvokeRequest)> for NextEvent {
    fn from((id, req): (&str, &InvokeRequest)) -> NextEvent {
        let tracing_id = req
            .req
            .headers()
            .get(AWS_XRAY_TRACE_HEADER)
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();

        let e = InvokeEvent {
            request_id: id.to_string(),
            invoked_function_arn: req.function_name.clone(),
            tracing: Tracing {
                r#type: AWS_XRAY_TRACE_HEADER.to_string(),
                value: tracing_id.to_string(),
            },
            ..Default::default()
        };

        NextEvent::Invoke(e)
    }
}
