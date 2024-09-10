use crate::error::ServerError;
use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use http_api_problem::ApiError;
use hyper::HeaderMap;
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot::Sender;

pub(crate) const AWS_XRAY_TRACE_HEADER: &str = "x-amzn-trace-id";
pub(crate) const AWS_INVOCATION_TYPE_HEADER: &str = "x-amz-invocation-type";

/// LambdaResponse is the data that the Lambda function sends
/// as the response to an invocation. Because Lambda uses a push
/// model, this response is represented as a HTTP Request data object.
pub type LambdaResponse = Request<Body>;

#[derive(Debug)]
pub enum Action {
    Invoke(InvokeRequest),
    Init,
}

#[derive(Debug)]
pub struct InvokeRequest {
    pub function_name: String,
    pub req: Request<Body>,
    pub resp_tx: Sender<LambdaResponse>,
}

#[derive(Debug, Deserialize)]
pub struct StreamingPrelude {
    #[serde(deserialize_with = "http_serde::status_code::deserialize")]
    #[serde(rename = "statusCode")]
    pub status_code: StatusCode,
    #[serde(deserialize_with = "http_serde::header_map::deserialize", default)]
    pub headers: HeaderMap,
    pub cookies: Vec<String>,
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
    pub fn invoke(id: &str, event: &InvokeRequest) -> NextEvent {
        let tracing_id = event
            .req
            .headers()
            .get(AWS_XRAY_TRACE_HEADER)
            .and_then(|h| h.to_str().ok())
            .unwrap_or_default();

        let e = InvokeEvent {
            request_id: id.to_string(),
            invoked_function_arn: event.function_name.clone(),
            tracing: Tracing {
                r#type: AWS_XRAY_TRACE_HEADER.to_string(),
                value: tracing_id.to_string(),
            },
            ..Default::default()
        };

        NextEvent::Invoke(e)
    }

    pub fn shutdown(reason: &str) -> NextEvent {
        NextEvent::Shutdown(ShutdownEvent {
            shutdown_reason: reason.into(),
            ..Default::default()
        })
    }

    pub fn type_queue(&self) -> &str {
        match self {
            Self::Invoke(_) => "INVOKE",
            Self::Shutdown(_) => "SHUTDOWN",
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LogBuffering {
    pub timeout_ms: usize,
    pub max_bytes: usize,
    pub max_items: usize,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct EventsDestination {
    pub protocol: String,
    #[serde(rename = "URI")]
    pub uri: String,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub(crate) struct SubcribeEvent {
    #[serde(rename = "schemaVersion")]
    pub schema_version: String,
    pub types: Vec<String>,
    pub buffering: Option<LogBuffering>,
    pub destination: EventsDestination,
}
