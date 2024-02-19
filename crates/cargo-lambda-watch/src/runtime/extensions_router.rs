use crate::{error::ServerError, requests::*, RefRuntimeState};
use axum::{body::Body, extract::State, http::Request, response::Response, Json};
use serde::de::DeserializeOwned;
use tokio::sync::mpsc;
use tracing::{debug, warn};

const EXTENSION_ID_HEADER: &str = "Lambda-Extension-Identifier";

pub(crate) async fn register_extension(
    State(state): State<RefRuntimeState>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    let payload: EventsRequest = extract_json(req).await?;

    debug!(?payload, "registering extension");

    let extension_id = state.ext_cache.register(payload.events).await;
    let resp = Response::builder()
        .status(200)
        .header(EXTENSION_ID_HEADER, extension_id)
        .body(Body::empty())?;

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
    let bytes = hyper::body::to_bytes(body)
        .await
        .map_err(ServerError::DataDeserialization)?;

    serde_json::from_slice(&bytes).map_err(ServerError::SerializationError)
}
