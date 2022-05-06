use crate::{error::ServerError, requests::*, state::ExtensionCache};
use axum::{
    body::Body,
    extract::{self, Extension},
    http::Request,
    response::Response,
    routing::{get, post},
    Json, Router,
};
use tokio::sync::mpsc;
use tracing::debug;

const EXTENSION_ID_HEADER: &str = "Lambda-Extension-Identifier";

pub(crate) fn routes() -> Router {
    Router::new()
        .route("/2020-01-01/extension/register", post(register_extension))
        .route(
            "/2020-01-01/extension/event/next",
            get(next_extension_event),
        )
}

async fn register_extension(
    Extension(ext_cache): Extension<ExtensionCache>,
    extract::Json(payload): extract::Json<EventsRequest>,
) -> Result<Response<Body>, ServerError> {
    debug!(?payload, "registering extension");

    let extension_id = ext_cache.register(payload.events).await;
    let resp = Response::builder()
        .status(200)
        .header(EXTENSION_ID_HEADER, extension_id)
        .body(Body::empty())?;

    Ok(resp)
}

async fn next_extension_event(
    Extension(ext_cache): Extension<ExtensionCache>,
    req: Request<Body>,
) -> Result<Json<NextEvent>, ServerError> {
    let extension_id = match req.headers().get(EXTENSION_ID_HEADER) {
        None => Err(ServerError::MissingExtensionIdHeader)?,
        Some(id) => id.to_str().unwrap(),
    };

    debug!(%extension_id, "extension waiting for next event");

    let (tx, mut rx) = mpsc::channel::<NextEvent>(100);
    ext_cache.set_senders(extension_id, tx).await;

    match rx.recv().await {
        None => Err(ServerError::NoExtensionEvent),
        Some(event) => {
            ext_cache.clear(extension_id).await;
            Ok(Json(event))
        }
    }
}
