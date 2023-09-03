use cargo_lambda_metadata::error::MetadataError;
use miette::Diagnostic;
use thiserror::Error;

use crate::requests::{Action, InvokeRequest, NextEvent};

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
    DataDeserialization(#[from] hyper::Error),

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

    #[error("client context cannot be longer than 3583 bytes after base64 encoding, the current size is {0}")]
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
}
