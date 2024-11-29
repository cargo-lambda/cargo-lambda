use std::collections::HashMap;

use cargo_metadata::Metadata as CargoMetadata;
use matchit::{InsertError, MatchError, MergeError, Router};
use serde::Deserialize;

use crate::{cargo::Metadata, error::MetadataError};

#[derive(Clone, Debug, Default, Deserialize)]
pub struct WatchConfig {
    pub router: Option<FunctionRouter>,
}

#[derive(Clone, Debug, Default)]
pub struct FunctionRouter {
    inner: Router<FunctionRoutes>,
}

impl FunctionRouter {
    pub fn at(&self, path: &str, method: &str) -> Result<&str, MatchError> {
        let matched = self.inner.at(path)?;
        matched.value.at(method).ok_or(MatchError::NotFound)
    }

    pub fn insert(&mut self, path: &str, routes: FunctionRoutes) -> Result<(), InsertError> {
        self.inner.insert(path, routes)
    }
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

impl<'de> Deserialize<'de> for FunctionRouter {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let routes = HashMap::<String, FunctionRoutes>::deserialize(deserializer)?;
        let mut inner = Router::new();

        for (path, route) in routes {
            inner.insert(&path, route).map_err(|e| {
                serde::de::Error::custom(format!("Failed to insert route {path}: {e}"))
            })?;
        }

        Ok(FunctionRouter { inner })
    }
}

impl<'de> Deserialize<'de> for FunctionRoutes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error;
        use serde_json::Value;

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

#[tracing::instrument(target = "cargo_lambda")]
pub fn watch_metadata(metadata: &CargoMetadata) -> Result<WatchConfig, MetadataError> {
    tracing::trace!(meta = ?metadata.workspace_metadata, "workspace metadata");

    let ws_metadata: Metadata =
        serde_json::from_value(metadata.workspace_metadata.clone()).unwrap_or_default();
    let mut config = ws_metadata.lambda.package.watch.unwrap_or_default();

    // Check package-specific metadata
    for pkg in &metadata.packages {
        for target in &pkg.targets {
            let target_matches =
                target.kind.iter().any(|kind| kind == "bin") && pkg.metadata.is_object();

            if target_matches {
                let package_metadata: Metadata = serde_json::from_value(pkg.metadata.clone())
                    .map_err(MetadataError::InvalidCargoMetadata)?;

                if let Some(package_watch) = package_metadata.lambda.package.watch {
                    merge_watch_config(&mut config, &package_watch)?;
                }
                break;
            }
        }
    }

    tracing::debug!(?config, "using watch configuration from metadata");
    Ok(config)
}

fn merge_watch_config(
    base: &mut WatchConfig,
    package_watch: &WatchConfig,
) -> Result<(), MergeError> {
    if let Some(router) = &package_watch.router {
        let mut base_router = base.router.take().unwrap_or_default();
        base_router.inner.merge(router.inner.clone())?;

        base.router = Some(base_router);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::cargo::{load_metadata, tests::fixture};

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
        let router = FunctionRouter { inner };
        assert_eq!(router.at("/api/v1/users", "GET"), Ok("user_handler"));
        assert_eq!(router.at("/api/v1/users", "POST"), Ok("user_handler"));

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
        let router = FunctionRouter { inner };
        assert_eq!(router.at("/api/v1/users", "GET"), Ok("get_user"));
        assert_eq!(router.at("/api/v1/users", "POST"), Ok("create_user"));
        assert_eq!(router.at("/api/v1/users", "PUT"), Err(MatchError::NotFound));
    }

    #[test]
    fn test_watch_config_metadata() {
        let metadata = load_metadata(fixture("workspace-package")).unwrap();

        let watch_config = watch_metadata(&metadata).unwrap();
        let router = watch_config.router.unwrap();
        assert_eq!(router.at("/foo", "GET"), Ok("crate-1"));
        assert_eq!(router.at("/foo", "POST"), Ok("crate-1"));
        assert_eq!(router.at("/bar", "GET"), Ok("crate-1"));
        assert_eq!(router.at("/bar", "POST"), Ok("crate-2"));
        assert_eq!(router.at("/baz", "GET"), Err(MatchError::NotFound));
        assert_eq!(router.at("/qux", "GET"), Ok("crate-3"));
    }
}
