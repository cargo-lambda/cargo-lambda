use std::{collections::HashMap, path::PathBuf};

use crate::cargo::{
    build::Build, deploy::Deploy, watch::Watch, CargoMetadata, Metadata, PackageMetadata,
};
use cargo_metadata::{Package, Target};
use figment::{
    providers::{Env, Format, Serialized, Toml},
    Figment,
};
use miette::{IntoDiagnostic, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default)]
pub struct ConfigOptions {
    pub name: Option<String>,
    pub context: Option<String>,
    pub global: Option<PathBuf>,
    pub admerge: bool,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    pub build: Build,
    pub deploy: Deploy,
    pub watch: Watch,
}

impl From<PackageMetadata> for Config {
    fn from(meta: PackageMetadata) -> Self {
        Config {
            env: meta.env,
            build: meta.build.unwrap_or_default(),
            watch: meta.watch.unwrap_or_default(),
            deploy: meta.deploy.unwrap_or_default(),
        }
    }
}

pub fn load_config(
    args_config: &Config,
    metadata: &CargoMetadata,
    options: &ConfigOptions,
) -> Result<Config> {
    let mut figment = figment_from_metadata(metadata, options)?;

    let mut args_serialized = Serialized::defaults(args_config);
    if let Some(context) = &options.context {
        args_serialized = args_serialized.profile(context);
    }

    figment = if options.admerge {
        figment.admerge(args_serialized)
    } else {
        figment.merge(args_serialized)
    };

    figment.extract().into_diagnostic()
}

pub fn load_config_without_cli_flags(
    metadata: &CargoMetadata,
    options: &ConfigOptions,
) -> Result<Config> {
    let figment = figment_from_metadata(metadata, options)?;
    figment.extract().into_diagnostic()
}

fn figment_from_metadata(metadata: &CargoMetadata, options: &ConfigOptions) -> Result<Figment> {
    let (ws_metadata, bin_metadata) = workspace_metadata(metadata, options.name.as_deref())?;
    let package_metadata = package_metadata(metadata, options.name.as_deref())?;

    let mut config_file = options
        .global
        .as_ref()
        .map(Toml::file)
        .unwrap_or_else(|| Toml::file("CargoLambda.toml"));
    if options.context.is_some() {
        config_file = config_file.nested()
    }

    let mut figment = Figment::new();
    if let Some(context) = &options.context {
        figment = figment.select(context)
    }

    let mut env_serialized = Env::prefixed("CARGO_LAMBDA_");
    if let Some(context) = &options.context {
        env_serialized = env_serialized.profile(context);
    }
    figment = figment.merge(env_serialized);

    figment = if options.admerge {
        figment.admerge(config_file)
    } else {
        figment.merge(config_file)
    };

    let mut ws_serialized = Serialized::defaults(ws_metadata);
    if let Some(context) = &options.context {
        ws_serialized = ws_serialized.profile(context);
    }
    if options.admerge {
        figment = figment.admerge(ws_serialized);
    } else {
        figment = figment.merge(ws_serialized);
    }

    if let Some(bin_metadata) = bin_metadata {
        let mut bin_serialized = Serialized::defaults(bin_metadata);
        if let Some(context) = &options.context {
            bin_serialized = bin_serialized.profile(context);
        }

        if options.admerge {
            figment = figment.admerge(bin_serialized);
        } else {
            figment = figment.merge(bin_serialized);
        }
    }

    if let Some(package_metadata) = package_metadata {
        let mut package_serialized = Serialized::defaults(package_metadata);
        if let Some(context) = &options.context {
            package_serialized = package_serialized.profile(context);
        }

        if options.admerge {
            figment = figment.admerge(package_serialized);
        } else {
            figment = figment.merge(package_serialized);
        }
    }

    Ok(figment)
}

fn workspace_metadata(
    metadata: &CargoMetadata,
    name: Option<&str>,
) -> Result<(Config, Option<Config>)> {
    if metadata.workspace_metadata.is_null() || !metadata.workspace_metadata.is_object() {
        return Ok((Config::default(), None));
    }

    let meta: Metadata =
        serde_json::from_value(metadata.workspace_metadata.clone()).into_diagnostic()?;

    let ws_config = meta.lambda.package.into();
    if let Some(name) = name {
        if let Some(bin_config) = meta.lambda.bin.get(name) {
            return Ok((ws_config, Some(bin_config.clone().into())));
        }
    }

    Ok((ws_config, None))
}

fn package_metadata(metadata: &CargoMetadata, name: Option<&str>) -> Result<Option<Config>> {
    let Some(name) = name else {
        let Some(root) = metadata.root_package() else {
            return Ok(None);
        };

        if root.metadata.is_null() || !root.metadata.is_object() {
            return Ok(None);
        }

        let meta: Metadata = serde_json::from_value(root.metadata.clone()).into_diagnostic()?;
        return Ok(Some(meta.lambda.package.into()));
    };

    let kind_condition = |pkg: &Package, target: &Target| {
        target.kind.iter().any(|kind| kind == "bin") && pkg.metadata.is_object()
    };

    for pkg in &metadata.packages {
        for target in &pkg.targets {
            if kind_condition(pkg, target) && target.name == name {
                let meta: Metadata =
                    serde_json::from_value(pkg.metadata.clone()).into_diagnostic()?;

                if let Some(bin_config) = meta.lambda.bin.get(name) {
                    return Ok(Some(bin_config.clone().into()));
                }

                return Ok(Some(meta.lambda.package.into()));
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {

    use matchit::MatchError;

    use super::*;
    use crate::{
        cargo::{build::CompilerOptions, load_metadata},
        lambda::{Memory, Tracing},
        tests::fixture_metadata,
    };

    #[test]
    fn test_load_env_from_metadata() {
        let metadata = load_metadata(fixture_metadata("single-binary-package")).unwrap();
        let config = load_config_without_cli_flags(&metadata, &ConfigOptions::default()).unwrap();

        assert_eq!(
            config.deploy.lambda_tags(),
            Some(HashMap::from([
                ("organization".to_string(), "aws".to_string()),
                ("team".to_string(), "lambda".to_string())
            ]))
        );

        assert_eq!(config.env.get("FOO"), Some(&"BAR".to_string()));
        assert_eq!(config.deploy.function_config.memory, Some(Memory::Mb512));
        assert_eq!(config.deploy.function_config.timeout, Some(60.into()));

        assert_eq!(
            config.deploy.function_config.layer,
            Some(vec![
                "arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer1".to_string(),
                "arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer2".to_string()
            ])
        );

        let tracing = config.deploy.function_config.tracing.unwrap();
        assert_eq!(tracing, Tracing::Active);
        assert_eq!(
            config.deploy.function_config.role,
            Some("arn:aws:lambda:us-east-1:xxxxxxxx:iam:role1".to_string())
        );

        let env_options = config.deploy.function_config.env_options.unwrap();
        assert_eq!(env_options.env_var, Some(vec!["VAR1=VAL1".to_string()]));
        assert_eq!(env_options.env_file, Some(".env.production".into()));

        let compiler = config.build.compiler.unwrap();

        let cargo_compiler = match compiler {
            CompilerOptions::Cargo(opts) => opts,
            other => panic!("unexpected compiler: {:?}", other),
        };
        assert_eq!(
            cargo_compiler.subcommand,
            Some(vec!["brazil".to_string(), "build".to_string()])
        );
        assert_eq!(
            cargo_compiler.extra_args,
            Some(vec!["--release".to_string()])
        );
    }

    #[test]
    fn test_load_router_from_metadata_admerge() {
        let options = ConfigOptions {
            name: Some("crate-3".to_string()),
            admerge: true,
            ..Default::default()
        };

        let metadata = load_metadata(fixture_metadata("workspace-package")).unwrap();
        let config = load_config_without_cli_flags(&metadata, &options).unwrap();

        let router = config.watch.router.unwrap();
        assert_eq!(
            router.at("/foo", "GET"),
            Ok(("crate-1".to_string(), HashMap::new()))
        );
        assert_eq!(
            router.at("/bar", "GET"),
            Ok(("crate-1".to_string(), HashMap::new()))
        );
        assert_eq!(
            router.at("/bar", "POST"),
            Ok(("crate-2".to_string(), HashMap::new()))
        );
        assert_eq!(router.at("/baz", "GET"), Err(MatchError::NotFound));
        assert_eq!(
            router.at("/qux", "GET"),
            Ok(("crate-3".to_string(), HashMap::new()))
        );
    }

    #[test]
    fn test_load_router_from_metadata_strict() {
        let options = ConfigOptions {
            name: Some("crate-3".to_string()),
            ..Default::default()
        };

        let metadata = load_metadata(fixture_metadata("workspace-package")).unwrap();
        let config = load_config_without_cli_flags(&metadata, &options).unwrap();

        let router = config.watch.router.unwrap();
        assert_eq!(router.raw.len(), 1);
        assert_eq!(router.at("/foo", "GET"), Err(MatchError::NotFound));
        assert_eq!(router.at("/bar", "GET"), Err(MatchError::NotFound));
        assert_eq!(router.at("/bar", "POST"), Err(MatchError::NotFound));
        assert_eq!(router.at("/baz", "GET"), Err(MatchError::NotFound));
        assert_eq!(
            router.at("/qux", "GET"),
            Ok(("crate-3".to_string(), HashMap::new()))
        );
    }

    #[test]
    fn test_extend_env_from_workspace() {
        let options = ConfigOptions {
            name: Some("basic-lambda-1".to_string()),
            admerge: true,
            ..Default::default()
        };

        let metadata = load_metadata(fixture_metadata("workspace-package")).unwrap();
        let config = load_config_without_cli_flags(&metadata, &options).unwrap();

        assert_eq!(config.env.get("FOO"), Some(&"BAR".to_string()));
        assert_eq!(config.env.get("EXTRA"), Some(&"TRUE".to_string()));
        assert_eq!(config.env.get("AWS_REGION"), Some(&"us-west-2".to_string()));
    }

    #[test]
    fn test_config_with_context() {
        let manifest = fixture_metadata("config-with-context");
        let global = manifest.parent().unwrap().join("CargoLambda.toml");

        let options = ConfigOptions {
            context: Some("production".to_string()),
            global: Some(global.clone()),
            ..Default::default()
        };

        let metadata = load_metadata(manifest).unwrap();
        let config = load_config_without_cli_flags(&metadata, &options).unwrap();
        assert_eq!(config.deploy.function_config.memory, Some(Memory::Mb1024));

        let options = ConfigOptions {
            context: Some("development".to_string()),
            global: Some(global.clone()),
            ..Default::default()
        };

        let config = load_config_without_cli_flags(&metadata, &options).unwrap();
        assert_eq!(config.deploy.function_config.memory, Some(Memory::Mb512));

        let options = ConfigOptions {
            global: Some(global),
            ..Default::default()
        };

        let config = load_config_without_cli_flags(&metadata, &options).unwrap();
        assert_eq!(config.deploy.function_config.memory, Some(Memory::Mb256));
    }

    #[test]
    fn test_config_with_context_and_cli_flags() {
        let manifest = fixture_metadata("config-with-context");
        let global = manifest.parent().unwrap().join("CargoLambda.toml");

        let options = ConfigOptions {
            context: Some("production".to_string()),
            global: Some(global.clone()),
            ..Default::default()
        };

        let mut deploy = Deploy::default();
        deploy.function_config.memory = Some(Memory::Mb2048);

        let args_config = Config {
            deploy,
            ..Default::default()
        };

        let metadata = load_metadata(manifest).unwrap();
        let config = load_config(&args_config, &metadata, &options).unwrap();
        assert_eq!(config.deploy.function_config.memory, Some(Memory::Mb2048));
    }
}
