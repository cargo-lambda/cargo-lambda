use miette::Diagnostic;
use thiserror::Error;

use crate::requests::{InvokeRequest, NextEvent};

#[derive(Debug, Diagnostic, Error)]
pub enum ServerError {
    #[error("failed to build a response")]
    #[diagnostic()]
    ResponseBuild(#[from] axum::http::Error),

    #[error("failed to decode a base64 encoded body")]
    #[diagnostic()]
    BodyDecodeError(#[from] base64::DecodeError),

    #[error("failed to send message to api")]
    #[diagnostic()]
    SendFunctionMessage,

    #[error("failed to send message to function")]
    #[diagnostic()]
    SendInvokeMessage(#[from] Box<tokio::sync::mpsc::error::SendError<InvokeRequest>>),

    #[error("failed to receive message from function")]
    #[diagnostic()]
    ReceiveFunctionMessage(#[from] tokio::sync::oneshot::error::RecvError),

    #[error("failed to start function process")]
    #[diagnostic()]
    SpawnCommand(#[from] std::io::Error),

    #[error("invalid request id header")]
    #[diagnostic()]
    InvalidRequestIdHeader(#[from] axum::http::header::ToStrError),

    #[error("failed to deserialize the request body")]
    #[diagnostic()]
    BodyDeserialization(#[from] hyper::Error),

    #[error("failed to deserialize the request body")]
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

    #[error("failed to send message to extension")]
    #[diagnostic()]
    SendEventMessage(#[from] Box<tokio::sync::mpsc::error::SendError<NextEvent>>),

    #[error("no extension event received")]
    #[diagnostic()]
    NoExtensionEvent,
}
