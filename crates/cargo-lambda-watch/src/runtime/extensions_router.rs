use crate::{RefRuntimeState, error::ServerError, requests::*, state::ExtensionType};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::Request,
    response::Response,
};
use http_body_util::BodyExt;
use hyper::HeaderMap;
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::mpsc;
use tracing::{debug, warn};

const EXTENSION_ID_HEADER: &str = "Lambda-Extension-Identifier";

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RegisterResponse {
    function_name: String,
    function_version: String,
    handler: String,
    account_id: Option<String>,
}

pub(crate) async fn register_extension(
    State(state): State<RefRuntimeState>,
    function_name: Option<Path<String>>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    let response_body = serde_json::to_vec(&RegisterResponse {
        function_name: extract_header_with_default(
            req.headers(),
            "cargo-lambda-extension-function-name",
            "function-name",
        ),
        function_version: extract_header_with_default(
            req.headers(),
            "cargo-lambda-extension-function-version",
            "function-version",
        ),
        handler: "bootstrap".to_string(),
        account_id: Some(extract_header_with_default(
            req.headers(),
            "cargo-lambda-extension-account-id",
            "account-id",
        )),
    })?;

    // we know that internal extensions are registered from within a function,
    // therefore have the function name prefixing their path
    let extension_type = match function_name {
        Some(_) => ExtensionType::Internal,
        _ => ExtensionType::External,
    };

    let payload: EventsRequest = extract_json(req).await?;
    debug!(?payload, "registering extension");

    let extension_id = state
        .ext_cache
        .register(payload.events, extension_type)
        .await;
    let resp = Response::builder()
        .status(200)
        .header(EXTENSION_ID_HEADER, extension_id)
        .body(response_body.into())?;

    Ok(resp)
}

pub(crate) async fn next_extension_event(
    State(state): State<RefRuntimeState>,
    req: Request<Body>,
) -> Result<Json<NextEvent>, ServerError> {
    let extension_id = match req.headers().get(EXTENSION_ID_HEADER) {
        None => Err(ServerError::MissingExtensionIdHeader)?,
        Some(id) => id.to_str().unwrap(),
    };

    debug!(%extension_id, "extension waiting for next event");

    let (tx, mut rx) = mpsc::channel::<NextEvent>(100);
    state.ext_cache.set_senders(extension_id, tx).await;

    match rx.recv().await {
        None => Err(ServerError::NoExtensionEvent),
        Some(event) => {
            state.ext_cache.clear(extension_id).await;
            Ok(Json(event))
        }
    }
}

pub(crate) async fn subcribe_extension_events(
    State(_state): State<RefRuntimeState>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    let extension_id = match req.headers().get(EXTENSION_ID_HEADER) {
        None => Err(ServerError::MissingExtensionIdHeader)?,
        Some(id) => id.to_str().unwrap().to_string(),
    };
    let payload: SubcribeEvent = extract_json(req).await?;

    debug!(%extension_id, ?payload.types, "received events subscription request");
    warn!(%extension_id, ?payload.types, "!!! Events subcription is not supported at the moment !!!");

    Ok(Response::new(Body::empty()))
}

/// Extract JSON manually instead of using Axum
/// because the extensions runtime doesn't send a Content-Type
async fn extract_json<T: DeserializeOwned>(req: Request<Body>) -> Result<T, ServerError> {
    let body = req.into_body();
    let bytes = body
        .collect()
        .await
        .map_err(ServerError::DataDeserialization)?
        .to_bytes();

    serde_json::from_slice(&bytes).map_err(ServerError::SerializationError)
}

fn extract_header_with_default(headers: &HeaderMap, name: &str, default: &str) -> String {
    headers
        .get(name)
        .and_then(|h| h.to_str().ok())
        .unwrap_or(default)
        .to_string()
}
