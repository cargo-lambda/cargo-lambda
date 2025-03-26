use cargo_options::Run;
use clap::Args;
use matchit::{InsertError, MatchError, Router};
use serde::{
    Deserialize, Serialize,
    de::{Error, Visitor},
    ser::SerializeSeq,
};
use serde_json::{Value, json};
use std::{collections::HashMap, path::PathBuf};

use crate::{
    cargo::{count_common_options, serialize_common_options},
    env::{EnvOptions, Environment},
    error::MetadataError,
    lambda::Timeout,
};

use cargo_lambda_remote::tls::TlsOptions;

#[cfg(windows)]
const DEFAULT_INVOKE_ADDRESS: &str = "127.0.0.1";

#[cfg(not(windows))]
const DEFAULT_INVOKE_ADDRESS: &str = "::";

const DEFAULT_INVOKE_PORT: u16 = 9000;

#[derive(Args, Clone, Debug, Default, Deserialize)]
#[command(
    name = "watch",
    visible_alias = "start",
    after_help = "Full command documentation: https://www.cargo-lambda.info/commands/watch.html"
)]
pub struct Watch {
    /// Ignore any code changes, and don't reload the function automatically
    #[arg(long, visible_alias = "no-reload")]
    #[serde(default)]
    pub ignore_changes: bool,

    /// Start the Lambda runtime APIs without starting the function.
    /// This is useful if you start (and debug) your function in your IDE.
    #[arg(long)]
    #[serde(default)]
    pub only_lambda_apis: bool,

    #[arg(short = 'a', long, default_value = DEFAULT_INVOKE_ADDRESS)]
    #[serde(default = "default_invoke_address")]
    /// Address where users send invoke requests
    pub invoke_address: String,

    /// Address port where users send invoke requests
    #[arg(short = 'P', long, default_value_t = DEFAULT_INVOKE_PORT)]
    #[serde(default = "default_invoke_port")]
    pub invoke_port: u16,

    /// Print OpenTelemetry traces after each function invocation
    #[arg(long)]
    #[serde(default)]
    pub print_traces: bool,

    /// Wait for the first invocation to compile the function
    #[arg(long, short)]
    #[serde(default)]
    pub wait: bool,

    /// Disable the default CORS configuration
    #[arg(long)]
    #[serde(default)]
    pub disable_cors: bool,

    /// How long the invoke request waits for a response
    #[arg(long)]
    #[serde(default)]
    pub timeout: Option<Timeout>,

    #[command(flatten)]
    #[serde(flatten)]
    pub cargo_opts: Run,

    #[command(flatten)]
    #[serde(flatten)]
    pub env_options: EnvOptions,

    #[command(flatten)]
    #[serde(flatten)]
    pub tls_options: TlsOptions,

    #[arg(skip)]
    #[serde(default, skip_serializing_if = "is_empty_router")]
    pub router: Option<FunctionRouter>,
}

impl Watch {
    pub fn manifest_path(&self) -> PathBuf {
        self.cargo_opts
            .manifest_path
            .clone()
            .unwrap_or_else(|| "Cargo.toml".into())
    }

    /// Returns the package name if there is only one package in the list of `packages`,
    /// otherwise None.
    pub fn package(&self) -> Option<String> {
        if self.cargo_opts.packages.len() > 1 {
            return None;
        }
        self.cargo_opts.packages.first().map(|s| s.to_string())
    }

    pub fn lambda_environment(
        &self,
        base: &HashMap<String, String>,
    ) -> Result<Environment, MetadataError> {
        self.env_options.lambda_environment(base)
    }
}

impl Serialize for Watch {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        // Count non-empty fields
        let field_count = self.ignore_changes as usize
            + self.only_lambda_apis as usize
            + !self.invoke_address.is_empty() as usize
            + (self.invoke_port != 0) as usize
            + self.print_traces as usize
            + self.wait as usize
            + self.disable_cors as usize
            + self.timeout.is_some() as usize
            + self.router.is_some() as usize
            + self.cargo_opts.manifest_path.is_some() as usize
            + self.cargo_opts.release as usize
            + self.cargo_opts.ignore_rust_version as usize
            + self.cargo_opts.unit_graph as usize
            + !self.cargo_opts.packages.is_empty() as usize
            + !self.cargo_opts.bin.is_empty() as usize
            + !self.cargo_opts.example.is_empty() as usize
            + !self.cargo_opts.args.is_empty() as usize
            + count_common_options(&self.cargo_opts.common)
            + self.env_options.count_fields()
            + self.tls_options.count_fields();

        let mut state = serializer.serialize_struct("Watch", field_count)?;

        // Only serialize bool fields that are true
        if self.ignore_changes {
            state.serialize_field("ignore_changes", &true)?;
        }
        if self.only_lambda_apis {
            state.serialize_field("only_lambda_apis", &true)?;
        }
        if !self.invoke_address.is_empty() {
            state.serialize_field("invoke_address", &self.invoke_address)?;
        }
        if self.invoke_port != 0 {
            state.serialize_field("invoke_port", &self.invoke_port)?;
        }
        if self.print_traces {
            state.serialize_field("print_traces", &true)?;
        }
        if self.wait {
            state.serialize_field("wait", &true)?;
        }
        if self.disable_cors {
            state.serialize_field("disable_cors", &true)?;
        }

        // Only serialize Some values for Options
        if let Some(timeout) = &self.timeout {
            state.serialize_field("timeout", timeout)?;
        }
        if let Some(router) = &self.router {
            state.serialize_field("router", router)?;
        }

        // Flatten the fields from cargo_opts and env_options
        self.env_options.serialize_fields::<S>(&mut state)?;
        self.tls_options.serialize_fields::<S>(&mut state)?;

        if let Some(manifest_path) = &self.cargo_opts.manifest_path {
            state.serialize_field("manifest_path", manifest_path)?;
        }
        if self.cargo_opts.release {
            state.serialize_field("release", &true)?;
        }
        if self.cargo_opts.ignore_rust_version {
            state.serialize_field("ignore_rust_version", &true)?;
        }
        if self.cargo_opts.unit_graph {
            state.serialize_field("unit_graph", &true)?;
        }
        if !self.cargo_opts.packages.is_empty() {
            state.serialize_field("packages", &self.cargo_opts.packages)?;
        }
        if !self.cargo_opts.bin.is_empty() {
            state.serialize_field("bin", &self.cargo_opts.bin)?;
        }
        if !self.cargo_opts.example.is_empty() {
            state.serialize_field("example", &self.cargo_opts.example)?;
        }
        if !self.cargo_opts.args.is_empty() {
            state.serialize_field("args", &self.cargo_opts.args)?;
        }
        serialize_common_options::<S>(&mut state, &self.cargo_opts.common)?;

        state.end()
    }
}

fn default_invoke_address() -> String {
    DEFAULT_INVOKE_ADDRESS.to_string()
}

fn default_invoke_port() -> u16 {
    DEFAULT_INVOKE_PORT
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WatchConfig {
    pub router: Option<FunctionRouter>,
}

#[derive(Clone, Debug, Default)]
pub struct FunctionRouter {
    inner: Router<FunctionRoutes>,
    pub(crate) raw: Vec<Route>,
}

impl FunctionRouter {
    pub fn at(
        &self,
        path: &str,
        method: &str,
    ) -> Result<(String, HashMap<String, String>), MatchError> {
        let matched = self.inner.at(path)?;
        let function = matched.value.at(method).ok_or(MatchError::NotFound)?;

        let params = matched
            .params
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();

        Ok((function.to_string(), params))
    }

    pub fn insert(&mut self, path: &str, routes: FunctionRoutes) -> Result<(), InsertError> {
        self.inner.insert(path, routes)
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }
}

#[allow(dead_code)]
fn is_empty_router(router: &Option<FunctionRouter>) -> bool {
    router.is_none() || router.as_ref().is_some_and(|r| r.is_empty())
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Route {
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    methods: Option<Vec<String>>,
    function: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FunctionRoutes {
    Single(String),
    Multiple(HashMap<String, String>),
}

impl FunctionRoutes {
    pub fn at(&self, method: &str) -> Option<&str> {
        match self {
            FunctionRoutes::Single(function) => Some(function),
            FunctionRoutes::Multiple(routes) => routes.get(method).map(|s| s.as_str()),
        }
    }
}

struct FunctionRouterVisitor;

impl<'de> Visitor<'de> for FunctionRouterVisitor {
    type Value = FunctionRouter;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a map or sequence of function routes")
    }

    fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::MapAccess<'de>,
    {
        let routes: HashMap<String, FunctionRoutes> =
            Deserialize::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;

        let mut inner = Router::new();
        let mut raw = Vec::new();

        let mut inverse = HashMap::new();

        for (path, route) in &routes {
            inner.insert(path, route.clone()).map_err(|e| {
                serde::de::Error::custom(format!("Failed to insert route {path}: {e}"))
            })?;

            match route {
                FunctionRoutes::Single(function) => {
                    raw.push(Route {
                        path: path.clone(),
                        methods: None,
                        function: function.clone(),
                    });
                }
                FunctionRoutes::Multiple(routes) => {
                    for (method, function) in routes {
                        inverse
                            .entry((path.clone(), function.clone()))
                            .and_modify(|route: &mut Route| {
                                let mut methods = route.methods.clone().unwrap_or_default();
                                methods.push(method.clone());
                                route.methods = Some(methods);
                            })
                            .or_insert_with(|| Route {
                                path: path.clone(),
                                methods: Some(vec![method.clone()]),
                                function: function.clone(),
                            });
                    }
                }
            }
        }

        for (_, route) in inverse {
            raw.push(route);
        }

        Ok(FunctionRouter { inner, raw })
    }

    fn visit_seq<A>(self, seq: A) -> Result<Self::Value, A::Error>
    where
        A: serde::de::SeqAccess<'de>,
    {
        let routes: Vec<Route> =
            Deserialize::deserialize(serde::de::value::SeqAccessDeserializer::new(seq))?;

        let mut inner = Router::new();
        let mut raw = Vec::new();

        let mut routes_by_path = HashMap::new();

        for route in &routes {
            routes_by_path
                .entry(route.path.clone())
                .and_modify(|routes| merge_routes(routes, route))
                .or_insert_with(|| decode_route(route));

            raw.push(route.clone());
        }

        for (path, route) in &routes_by_path {
            inner.insert(path, route.clone()).map_err(|e| {
                serde::de::Error::custom(format!("Failed to insert route {path}: {e}"))
            })?;
        }

        Ok(FunctionRouter { inner, raw })
    }
}

fn merge_routes(routes: &mut FunctionRoutes, route: &Route) {
    let methods = route.methods.clone().unwrap_or_default();
    match routes {
        FunctionRoutes::Single(function) if !methods.is_empty() => {
            let mut tmp = HashMap::new();
            for method in methods {
                tmp.insert(method.clone(), function.clone());
            }
            *routes = FunctionRoutes::Multiple(tmp);
        }
        FunctionRoutes::Multiple(_) if methods.is_empty() => {
            *routes = FunctionRoutes::Single(route.function.clone());
        }
        FunctionRoutes::Multiple(routes) => {
            for method in methods {
                routes.insert(method.clone(), route.function.clone());
            }
        }
        FunctionRoutes::Single(_) => {
            *routes = FunctionRoutes::Single(route.function.clone());
        }
    }
}

fn decode_route(route: &Route) -> FunctionRoutes {
    match &route.methods {
        Some(methods) => {
            let mut routes = HashMap::new();
            for method in methods {
                routes.insert(method.clone(), route.function.clone());
            }
            FunctionRoutes::Multiple(routes)
        }
        None => FunctionRoutes::Single(route.function.clone()),
    }
}

impl<'de> Deserialize<'de> for FunctionRouter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(FunctionRouterVisitor)
    }
}

impl Serialize for FunctionRouter {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.raw.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for FunctionRoutes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::String(s) => Ok(FunctionRoutes::Single(s)),
            Value::Array(arr) => {
                let mut routes = HashMap::new();
                for item in arr {
                    let obj = item.as_object().ok_or_else(|| {
                        Error::custom("Array items must be objects with method and function fields")
                    })?;

                    let method = obj
                        .get("method")
                        .and_then(|m| m.as_str())
                        .ok_or_else(|| Error::custom("Missing or invalid method field"))?;

                    let function = obj
                        .get("function")
                        .and_then(|f| f.as_str())
                        .ok_or_else(|| Error::custom("Missing or invalid function field"))?;

                    routes.insert(method.to_string(), function.to_string());
                }
                Ok(FunctionRoutes::Multiple(routes))
            }
            _ => Err(Error::custom(
                "Function routes must be either a string or an array of objects with method and function fields",
            )),
        }
    }
}

impl Serialize for FunctionRoutes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            FunctionRoutes::Single(function) => function.serialize(serializer),
            FunctionRoutes::Multiple(routes) => {
                let mut seq = serializer.serialize_seq(Some(routes.len()))?;
                for (method, function) in routes {
                    let mut map = serde_json::Map::new();
                    map.insert("method".to_string(), json!(method));
                    map.insert("function".to_string(), json!(function));
                    seq.serialize_element(&Value::Object(map))?;
                }
                seq.end()
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use cargo_options::CommonOptions;
    use serde_json::{Value, json};
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn test_router_deserialize() {
        let router: FunctionRouter = toml::from_str(
            r#"
            "/api/v1/users" = [
                { function = "get_user", method = "GET" },
                { function = "create_user", method = "POST" }
            ]
            "/api/v1/all_methods" = "all_methods"
        "#,
        )
        .unwrap();

        assert_eq!(
            router.inner.at("/api/v1/users").unwrap().value,
            &FunctionRoutes::Multiple(HashMap::from([
                ("GET".to_string(), "get_user".to_string()),
                ("POST".to_string(), "create_user".to_string()),
            ]))
        );

        assert_eq!(
            router.inner.at("/api/v1/all_methods").unwrap().value,
            &FunctionRoutes::Single("all_methods".to_string())
        );
    }

    #[test]
    fn test_router_get() {
        let router = FunctionRouter::default();
        assert_eq!(router.at("/api/v1/users", "GET"), Err(MatchError::NotFound));

        let mut inner = Router::new();
        inner
            .insert(
                "/api/v1/users",
                FunctionRoutes::Single("user_handler".to_string()),
            )
            .unwrap();
        let router = FunctionRouter {
            inner,
            ..Default::default()
        };
        assert_eq!(
            router.at("/api/v1/users", "GET"),
            Ok(("user_handler".to_string(), HashMap::new()))
        );
        assert_eq!(
            router.at("/api/v1/users", "POST"),
            Ok(("user_handler".to_string(), HashMap::new()))
        );

        let mut inner = Router::new();
        inner
            .insert(
                "/api/v1/users",
                FunctionRoutes::Multiple(HashMap::from([
                    ("GET".to_string(), "get_user".to_string()),
                    ("POST".to_string(), "create_user".to_string()),
                ])),
            )
            .unwrap();
        let router = FunctionRouter {
            inner,
            ..Default::default()
        };
        assert_eq!(
            router.at("/api/v1/users", "GET"),
            Ok(("get_user".to_string(), HashMap::new()))
        );
        assert_eq!(
            router.at("/api/v1/users", "POST"),
            Ok(("create_user".to_string(), HashMap::new()))
        );
        assert_eq!(router.at("/api/v1/users", "PUT"), Err(MatchError::NotFound));

        let mut inner = Router::new();
        inner
            .insert(
                "/api/v1/users/{id}",
                FunctionRoutes::Single("user_handler".to_string()),
            )
            .unwrap();
        let router = FunctionRouter {
            inner,
            ..Default::default()
        };

        let (function, params) = router.at("/api/v1/users/1", "GET").unwrap();
        assert_eq!(function, "user_handler");
        assert_eq!(params, HashMap::from([("id".to_string(), "1".to_string())]));
    }

    #[test]
    fn test_router_serialize() {
        let config = r#"
            "/api/v1/users" = [
                { function = "get_user", method = "GET" },
                { function = "create_user", method = "POST" }
            ]
            "/api/v1/all_methods" = "all_methods"
        "#;
        let router: FunctionRouter = toml::from_str(config).unwrap();

        let json = serde_json::to_value(&router).unwrap();

        let new_router: FunctionRouter = serde_json::from_value(json).unwrap();
        assert_eq!(new_router.raw, router.raw);

        assert_eq!(
            new_router.inner.at("/api/v1/users").unwrap().value,
            &FunctionRoutes::Multiple(HashMap::from([
                ("GET".to_string(), "get_user".to_string()),
                ("POST".to_string(), "create_user".to_string()),
            ]))
        );

        assert_eq!(
            new_router.inner.at("/api/v1/all_methods").unwrap().value,
            &FunctionRoutes::Single("all_methods".to_string())
        );
    }

    #[test]
    fn test_watch_serialization() {
        let watch = Watch {
            invoke_address: "127.0.0.1".to_string(),
            invoke_port: 9000,
            env_options: EnvOptions {
                env_file: Some(PathBuf::from("/tmp/env")),
                env_var: Some(vec!["FOO=BAR".to_string()]),
            },
            tls_options: TlsOptions::new(
                Some(PathBuf::from("/tmp/cert.pem")),
                Some(PathBuf::from("/tmp/key.pem")),
                Some(PathBuf::from("/tmp/ca.pem")),
            ),
            cargo_opts: Run {
                common: CommonOptions {
                    quiet: false,
                    jobs: None,
                    keep_going: false,
                    profile: None,
                    features: vec!["feature1".to_string()],
                    all_features: false,
                    no_default_features: true,
                    target: vec!["x86_64-unknown-linux-gnu".to_string()],
                    target_dir: Some(PathBuf::from("/tmp/target")),
                    message_format: vec!["json".to_string()],
                    verbose: 1,
                    color: Some("auto".to_string()),
                    frozen: true,
                    locked: true,
                    offline: true,
                    config: vec!["config.toml".to_string()],
                    unstable_flags: vec!["flag1".to_string()],
                    timings: None,
                },
                manifest_path: None,
                release: false,
                ignore_rust_version: false,
                unit_graph: false,
                packages: vec![],
                bin: vec![],
                example: vec![],
                args: vec![],
            },
            ..Default::default()
        };

        let json = serde_json::to_value(&watch).unwrap();
        assert_eq!(json["invoke_address"], "127.0.0.1");
        assert_eq!(json["invoke_port"], 9000);
        assert_eq!(json["env_file"], "/tmp/env");
        assert_eq!(json["env_var"], json!(["FOO=BAR"]));
        assert_eq!(json["tls_cert"], "/tmp/cert.pem");
        assert_eq!(json["tls_key"], "/tmp/key.pem");
        assert_eq!(json["tls_ca"], "/tmp/ca.pem");
        assert_eq!(json["features"], json!(["feature1"]));
        assert_eq!(json["no_default_features"], true);
        assert_eq!(json["target"], json!(["x86_64-unknown-linux-gnu"]));
        assert_eq!(json["target_dir"], "/tmp/target");
        assert_eq!(json["message_format"], json!(["json"]));
        assert_eq!(json["verbose"], 1);
        assert_eq!(json["color"], "auto");
        assert_eq!(json["frozen"], true);
        assert_eq!(json["locked"], true);
        assert_eq!(json["offline"], true);
        assert_eq!(json["config"], json!(["config.toml"]));
        assert_eq!(json["unstable_flags"], json!(["flag1"]));
        assert_eq!(json["timings"], Value::Null);

        let deserialized: Watch = serde_json::from_value(json).unwrap();

        assert_eq!(deserialized.invoke_address, watch.invoke_address);
        assert_eq!(deserialized.invoke_port, watch.invoke_port);
        assert_eq!(
            deserialized.env_options.env_file,
            watch.env_options.env_file
        );
        assert_eq!(deserialized.env_options.env_var, watch.env_options.env_var);
        assert_eq!(
            deserialized.tls_options.tls_cert,
            watch.tls_options.tls_cert
        );
        assert_eq!(deserialized.tls_options.tls_key, watch.tls_options.tls_key);
        assert_eq!(deserialized.tls_options.tls_ca, watch.tls_options.tls_ca);
        assert_eq!(
            deserialized.cargo_opts.common.features,
            watch.cargo_opts.common.features
        );
        assert_eq!(
            deserialized.cargo_opts.common.no_default_features,
            watch.cargo_opts.common.no_default_features
        );
        assert_eq!(
            deserialized.cargo_opts.common.target,
            watch.cargo_opts.common.target
        );
        assert_eq!(
            deserialized.cargo_opts.common.target_dir,
            watch.cargo_opts.common.target_dir
        );
        assert_eq!(
            deserialized.cargo_opts.common.message_format,
            watch.cargo_opts.common.message_format
        );
        assert_eq!(
            deserialized.cargo_opts.common.verbose,
            watch.cargo_opts.common.verbose
        );
        assert_eq!(
            deserialized.cargo_opts.common.color,
            watch.cargo_opts.common.color
        );
        assert_eq!(
            deserialized.cargo_opts.common.frozen,
            watch.cargo_opts.common.frozen
        );
        assert_eq!(
            deserialized.cargo_opts.common.locked,
            watch.cargo_opts.common.locked
        );
        assert_eq!(
            deserialized.cargo_opts.common.offline,
            watch.cargo_opts.common.offline
        );
        assert_eq!(
            deserialized.cargo_opts.common.config,
            watch.cargo_opts.common.config
        );
        assert_eq!(
            deserialized.cargo_opts.common.unstable_flags,
            watch.cargo_opts.common.unstable_flags
        );
        assert_eq!(
            deserialized.cargo_opts.common.timings,
            watch.cargo_opts.common.timings
        );
    }
}
