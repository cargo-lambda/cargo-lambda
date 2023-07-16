use crate::{
    error::ServerError,
    requests::*,
    runtime::{LAMBDA_RUNTIME_AWS_REQUEST_ID, LAMBDA_RUNTIME_XRAY_TRACE_HEADER},
};
use aws_lambda_events::{
    apigw::{
        ApiGatewayV2httpRequest, ApiGatewayV2httpRequestContext,
        ApiGatewayV2httpRequestContextHttpDescription, ApiGatewayV2httpResponse,
    },
    encodings::Body as LambdaBody,
};
use axum::{
    body::Body,
    extract::{Extension, Path},
    handler::Handler,
    http::{HeaderValue, Request},
    response::Response,
    routing::{any, post},
    Router,
};
use base64::{engine::general_purpose as b64, Engine as _};
use cargo_lambda_invoke::DEFAULT_PACKAGE_FUNCTION;
use chrono::Utc;
use hyper::{body::to_bytes, header};
use miette::Result;
use opentelemetry::{
    global,
    trace::{TraceContextExt, Tracer},
    Context, KeyValue,
};
use query_map::QueryMap;
use std::collections::HashMap;
use tokio::sync::{mpsc::Sender, oneshot};

const LAMBDA_URL_PREFIX: &str = "lambda-url";

pub(crate) fn routes() -> Router {
    Router::new()
        .route(
            "/2015-03-31/functions/:function_name/invocations",
            post(invoke_handler),
        )
        .route("/lambda-url/:function_name/*path", any(furls_handler))
        .fallback(furls_handler.into_service())
}

async fn furls_handler(
    Extension(cmd_tx): Extension<Sender<InvokeRequest>>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    let (parts, body) = req.into_parts();
    let uri = &parts.uri;

    let (function_name, mut path) = extract_path_parameters(uri.path());

    let headers = &parts.headers;

    let body = to_bytes(body)
        .await
        .map_err(ServerError::BodyDeserialization)?;
    let text_content_type = match headers.get("content-type") {
        None => true,
        Some(c) => {
            let c = c.to_str().unwrap_or_default();
            c.starts_with("text/") || c.starts_with("application/json")
        }
    };

    let (body, is_base64_encoded) = if body.is_empty() {
        (None, false)
    } else if text_content_type {
        let body =
            String::from_utf8(body.into_iter().collect()).map_err(ServerError::StringBody)?;
        (Some(body), false)
    } else {
        let body = b64::STANDARD.encode(body.into_iter().collect::<Vec<u8>>());
        (Some(body), true)
    };

    let query_string_parameters = uri
        .query()
        .unwrap_or_default()
        .parse::<QueryMap>()
        .unwrap_or_default();

    let cookies = headers.get("cookie").map(|c| {
        c.to_str()
            .unwrap_or_default()
            .split("; ")
            .map(|s| s.trim().to_string())
            .collect()
    });

    let req_id = headers
        .get(LAMBDA_RUNTIME_AWS_REQUEST_ID)
        .expect("missing request id")
        .to_str()
        .expect("invalid request id format");

    let time = Utc::now();

    if !path.starts_with('/') {
        path = format!("/{path}");
    }

    let request_context = ApiGatewayV2httpRequestContext {
        stage: Some("$default".into()),
        route_key: Some("$default".into()),
        request_id: Some(req_id.into()),
        domain_name: Some("localhost".into()),
        domain_prefix: Some(function_name.clone()),
        http: ApiGatewayV2httpRequestContextHttpDescription {
            method: parts.method.clone(),
            path: Some(path.clone()),
            protocol: Some("http".into()),
            source_ip: Some("127.0.0.1".into()),
            user_agent: Some("cargo-lambda".into()),
        },
        time: Some(time.format("%d/%b/%Y:%T %z").to_string()),
        time_epoch: time.timestamp(),
        account_id: None,
        authorizer: None,
        authentication: None,
        apiid: None,
    };

    let event = ApiGatewayV2httpRequest {
        version: Some("2.0".into()),
        route_key: Some("$default".into()),
        raw_path: Some(path),
        raw_query_string: uri.query().map(String::from),
        path_parameters: HashMap::new(),
        stage_variables: HashMap::new(),
        headers: headers.clone(),
        body,
        request_context,
        cookies,
        query_string_parameters,
        is_base64_encoded,
    };
    let event = serde_json::to_string(&event).map_err(ServerError::SerializationError)?;

    let req = Request::from_parts(parts, event.into());
    let mut resp = schedule_invocation(&cmd_tx, function_name, req).await?;

    let body = to_bytes(resp.body_mut())
        .await
        .map_err(ServerError::BodyDeserialization)?;
    let resp_event: ApiGatewayV2httpResponse =
        serde_json::from_slice(&body).map_err(ServerError::SerializationError)?;

    let is_base64_encoded = resp_event.is_base64_encoded.unwrap_or(false);
    let resp_body = match resp_event.body.unwrap_or(LambdaBody::Empty) {
        LambdaBody::Empty => Body::empty(),
        b if is_base64_encoded => Body::from(
            b64::STANDARD
                .decode(b.as_ref())
                .map_err(ServerError::BodyDecodeError)?,
        ),
        LambdaBody::Text(s) => Body::from(s),
        LambdaBody::Binary(b) => Body::from(b),
    };
    let mut builder = Response::builder().status(resp_event.status_code as u16);
    if let Some(headers) = builder.headers_mut() {
        headers.extend(resp_event.headers);
        headers.extend(resp_event.multi_value_headers);

        resp_event.cookies.iter().try_for_each(|cookie| {
            let header_value =
                HeaderValue::try_from(cookie).map_err(|e| ServerError::ResponseBuild(e.into()))?;
            headers.append(header::SET_COOKIE, header_value);
            Ok::<(), ServerError>(())
        })?;
    }

    builder.body(resp_body).map_err(ServerError::ResponseBuild)
}

async fn invoke_handler(
    Extension(cmd_tx): Extension<Sender<InvokeRequest>>,
    Path(function_name): Path<String>,
    req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    schedule_invocation(&cmd_tx, function_name, req).await
}

async fn schedule_invocation(
    cmd_tx: &Sender<InvokeRequest>,
    function_name: String,
    mut req: Request<Body>,
) -> Result<Response<Body>, ServerError> {
    let headers = req.headers_mut();

    let span = global::tracer("cargo-lambda/emulator").start("invoke request");
    let cx = Context::current_with_span(span);

    let mut injector = HashMap::new();
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(&cx, &mut injector);
    });
    let xray_header = injector
        .get(AWS_XRAY_TRACE_HEADER)
        .expect("x-amzn-trace-id header not injected by the propagator") // this is Infaliable
        .parse()
        .expect("x-amzn-trace-id header is not in the expected format"); // this is Infaliable
    headers.insert(LAMBDA_RUNTIME_XRAY_TRACE_HEADER, xray_header);

    let (resp_tx, resp_rx) = oneshot::channel::<Response<Body>>();
    let function_name = if function_name.is_empty() {
        DEFAULT_PACKAGE_FUNCTION.into()
    } else {
        function_name
    };

    let req = InvokeRequest {
        function_name,
        req,
        resp_tx,
    };

    cmd_tx
        .send(req)
        .await
        .map_err(|e| ServerError::SendInvokeMessage(Box::new(e)))?;

    let resp = resp_rx.await.map_err(ServerError::ReceiveFunctionMessage)?;

    cx.span().add_event(
        "function call completed",
        vec![KeyValue::new("status", resp.status().to_string())],
    );

    Ok(resp)
}

fn extract_path_parameters(path: &str) -> (String, String) {
    let mut comp = path.split('/');

    comp.next();
    if let (Some(prefix), Some(fun_name)) = (comp.next(), comp.next()) {
        if prefix == LAMBDA_URL_PREFIX {
            let l = format!("/{prefix}/{fun_name}");
            let mut new_path = path.replace(&l, "");
            if !new_path.starts_with('/') {
                new_path = format!("/{new_path}");
            }
            return (fun_name.to_string(), new_path);
        }
    }

    (DEFAULT_PACKAGE_FUNCTION.to_string(), path.to_string())
}

#[cfg(test)]
mod test {
    use super::extract_path_parameters;
    use cargo_lambda_invoke::DEFAULT_PACKAGE_FUNCTION;

    #[test]
    fn test_extract_path_parameters() {
        let (func, path) = extract_path_parameters("");
        assert_eq!(DEFAULT_PACKAGE_FUNCTION, func);
        assert_eq!("", path);

        let (func, path) = extract_path_parameters("/");
        assert_eq!(DEFAULT_PACKAGE_FUNCTION, func);
        assert_eq!("/", path);

        let (func, path) = extract_path_parameters("/foo");
        assert_eq!(DEFAULT_PACKAGE_FUNCTION, func);
        assert_eq!("/foo", path);

        let (func, path) = extract_path_parameters("/foo/");
        assert_eq!(DEFAULT_PACKAGE_FUNCTION, func);
        assert_eq!("/foo/", path);

        let (func, path) = extract_path_parameters("/lambda-url/func-name");
        assert_eq!("func-name", func);
        assert_eq!("/", path);

        let (func, path) = extract_path_parameters("/lambda-url/func-name/");
        assert_eq!("func-name", func);
        assert_eq!("/", path);

        let (func, path) = extract_path_parameters("/lambda-url/func-name/foo");
        assert_eq!("func-name", func);
        assert_eq!("/foo", path);

        let (func, path) = extract_path_parameters("/lambda-url/func-name/foo/");
        assert_eq!("func-name", func);
        assert_eq!("/foo/", path);
    }
}
