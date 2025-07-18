use std::collections::HashMap;

use axum::response::{IntoResponse, Response};
use cargo_lambda_metadata::error::MetadataError;
use cargo_lambda_remote::tls::TlsError;
use http::StatusCode;
use miette::Diagnostic;
use serde::Serialize;
use thiserror::Error;

use crate::requests::{Action, InvokeRequest, NextEvent};

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Diagnostic, Error)]
pub enum ServerError {
    #[error("failed to build a response")]
    #[diagnostic()]
    ResponseBuild(#[from] axum::http::Error),

    #[error("failed to decode a base64 encoded body: {0}")]
    #[diagnostic()]
    BodyDecodeError(#[from] base64::DecodeError),

    #[error("failed to send message to api")]
    #[diagnostic()]
    SendFunctionMessage,

    #[error("failed to send message to function: {0}")]
    #[diagnostic()]
    SendActionMessage(#[from] Box<tokio::sync::mpsc::error::SendError<Action>>),

    #[error("failed to send message to function: {0}")]
    #[diagnostic()]
    SendInvokeMessage(#[from] Box<tokio::sync::mpsc::error::SendError<InvokeRequest>>),

    #[error("failed to receive message from function: {0}")]
    #[diagnostic()]
    ReceiveFunctionMessage(#[from] tokio::sync::oneshot::error::RecvError),

    #[error("failed to start function process")]
    #[diagnostic()]
    SpawnCommand(#[from] std::io::Error),

    #[error("invalid request id header: {0}")]
    #[diagnostic()]
    InvalidRequestIdHeader(#[from] axum::http::header::ToStrError),

    #[error("failed to deserialize data {0}")]
    #[diagnostic()]
    DataDeserialization(#[from] axum::Error),

    #[error("failed to deserialize streaming prelude")]
    #[diagnostic()]
    StreamingPreludeDeserialization,

    #[error("failed to deserialize the request body: {0}")]
    #[diagnostic()]
    StringBody(#[from] std::string::FromUtf8Error),

    #[error(transparent)]
    #[diagnostic()]
    SerializationError(#[from] serde_json::Error),

    #[error("failed to run watcher")]
    #[diagnostic()]
    WatcherError(#[from] watchexec::error::CriticalError),

    #[error("failed to load ignore files")]
    #[diagnostic()]
    InvalidIgnoreFiles(#[from] ignore_files::Error),

    #[error("missing extension id header")]
    #[diagnostic()]
    MissingExtensionIdHeader,

    #[error("failed to send message to extension: {0}")]
    #[diagnostic()]
    SendEventMessage(#[from] Box<tokio::sync::mpsc::error::SendError<NextEvent>>),

    #[error("no extension event received")]
    #[diagnostic()]
    NoExtensionEvent,

    #[error(
        "client context cannot be longer than 3583 bytes after base64 encoding, the current size is {0}"
    )]
    #[diagnostic()]
    InvalidClientContext(usize),

    #[error(transparent)]
    #[diagnostic()]
    FailedToReadMetadata(#[from] MetadataError),

    #[error("the project doesn't include any binary packages")]
    #[diagnostic()]
    NoBinaryPackages,

    #[error("the streaming prelude is missing from the Lambda response")]
    #[diagnostic()]
    MissingStreamingPrelude,

    #[error(transparent)]
    #[diagnostic()]
    InvalidStatusCode(#[from] hyper::http::status::InvalidStatusCode),

    #[error(transparent)]
    #[diagnostic()]
    TlsError(#[from] TlsError),
}

// Explicitly implement Send + Sync
unsafe impl Send for ServerError {}
unsafe impl Sync for ServerError {}

#[derive(Clone, Debug, Default, Serialize)]
#[cfg_attr(test, derive(PartialEq))]
pub struct HttpApiProblem {
    /// A URI reference [RFC3986](https://tools.ietf.org/html/rfc3986) that identifies the
    /// problem type.  This specification encourages that, when
    /// dereferenced, it provide human-readable documentation for the
    /// problem type (e.g., using HTML [W3C.REC-html5-20141028]).  When
    /// this member is not present, its value is assumed to be
    /// "about:blank".
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_url: Option<String>,
    /// The HTTP status code [RFC7231, Section 6](https://tools.ietf.org/html/rfc7231#section-6)
    /// generated by the origin server for this occurrence of the problem.
    #[serde(default)]
    #[serde(with = "custom_http_status_serialization")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<StatusCode>,
    /// A short, human-readable summary of the problem
    /// type. It SHOULD NOT change from occurrence to occurrence of the
    /// problem, except for purposes of localization (e.g., using
    /// proactive content negotiation;
    /// see [RFC7231, Section 3.4](https://tools.ietf.org/html/rfc7231#section-3.4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A human-readable explanation specific to this
    /// occurrence of the problem.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// A URI reference that identifies the specific
    /// occurrence of the problem.  It may or may not yield further
    /// information if dereferenced.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,

    /// Additional fields that must be JSON values
    #[serde(flatten)]
    additional_fields: HashMap<String, serde_json::Value>,
}

impl HttpApiProblem {
    fn with_title<T: Into<StatusCode>>(status: T) -> Self {
        let status = status.into();
        Self {
            status: Some(status),
            title: Some(
                status
                    .canonical_reason()
                    .unwrap_or("<unknown status code>")
                    .to_string(),
            ),
            ..Default::default()
        }
    }

    fn type_url<T: Into<String>>(mut self, type_url: T) -> Self {
        self.type_url = Some(type_url.into());
        self
    }

    fn with_title_and_type<T: Into<StatusCode>>(status: T) -> Self {
        let status = status.into();
        Self::with_title(status).type_url(format!("https://httpstatuses.com/{}", status.as_u16()))
    }

    fn json_string(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}

mod custom_http_status_serialization {
    use http::StatusCode;
    use serde::Serializer;

    pub fn serialize<S>(status: &Option<StatusCode>, s: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        if let Some(ref status_code) = *status {
            return s.serialize_u16(status_code.as_u16());
        }
        s.serialize_none()
    }
}

impl IntoResponse for ServerError {
    fn into_response(self) -> Response {
        let status = StatusCode::INTERNAL_SERVER_ERROR;
        let api_error = HttpApiProblem::with_title_and_type(status);

        (status, api_error.json_string()).into_response()
    }
}
