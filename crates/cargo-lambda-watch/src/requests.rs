use axum::{
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use http_api_problem::ApiError;
use miette::Diagnostic;
use thiserror::Error;
use tokio::sync::oneshot::Sender;

#[derive(Debug)]
pub struct InvokeRequest {
    pub function_name: String,
    pub req: Request<Body>,
    pub resp_tx: Sender<Response<Body>>,
}

#[derive(Error, Diagnostic, Debug)]
pub enum ServerError {
    #[error("failed to build a response")]
    #[diagnostic()]
    ResponseBuild(#[from] axum::http::Error),

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

    #[error("failed to serialize lambda-url event")]
    #[diagnostic()]
    SerializationError(#[from] serde_json::Error),
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
