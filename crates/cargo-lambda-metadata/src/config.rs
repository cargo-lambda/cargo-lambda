use std::{collections::HashMap, path::PathBuf};

use crate::{
    cargo::{
        CargoMetadata, Metadata, PackageMetadata, binary_targets_from_metadata, build::Build,
        deploy::Deploy, watch::Watch,
    },
    error::MetadataError,
};
use cargo_metadata::{Package, Target};
use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use miette::{IntoDiagnostic, Result};
use serde::{Deserialize, Serialize};
use tracing::trace;

/// A function can be identified by its package name or by its binary name.
/// We need both to be able to load the config from the package metadata,
/// regardless of which one is provided by the user.
#[derive(Debug, Default)]
pub struct FunctionNames {
    package: Option<String>,
    binary: Option<String>,
}

impl FunctionNames {
    pub fn from_package(package: &str) -> Self {
        FunctionNames::new(Some(package.to_string()), None)
    }

    pub fn from_binary(binary: &str) -> Self {
        FunctionNames::new(None, Some(binary.to_string()))
    }

    pub fn new(package: Option<String>, binary: Option<String>) -> Self {
        FunctionNames { package, binary }
    }

    pub fn is_empty(&self) -> bool {
        self.package.is_none() && self.binary.is_none()
    }

    pub fn include(&self, name: &str) -> bool {
        self.package.as_ref().is_some_and(|p| p == name)
            || self.binary.as_ref().is_some_and(|b| b == name)
    }

    pub fn find_binary_metadata<'a>(
        &'a self,
        metadata: &'a HashMap<String, PackageMetadata>,
    ) -> Option<&'a PackageMetadata> {
        let bin_meta = self.binary.as_ref().and_then(|binary| metadata.get(binary));
        if bin_meta.is_some() {
            return bin_meta;
        }

        self.package
            .as_ref()
            .and_then(|package| metadata.get(package))
    }
}

impl From<(&str, &str)> for FunctionNames {
    fn from((package, binary): (&str, &str)) -> Self {
        FunctionNames::new(Some(package.to_string()), Some(binary.to_string()))
    }
}

#[derive(Debug, Default)]
pub struct ConfigOptions {
    pub names: FunctionNames,
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
    let (bin_metadata, package_metadata, mut figment) = general_config_figment(metadata, options)?;

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

pub fn general_config_figment(
    metadata: &CargoMetadata,
    options: &ConfigOptions,
) -> Result<(Option<Config>, Option<Config>, Figment)> {
    let (ws_metadata, bin_metadata) = workspace_metadata(metadata, &options.names)?;
    let package_metadata = package_metadata(metadata, &options.names)?;

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

    Ok((bin_metadata, package_metadata, figment))
}

fn workspace_metadata(
    metadata: &CargoMetadata,
    name: &FunctionNames,
) -> Result<(Config, Option<Config>)> {
    if metadata.workspace_metadata.is_null() || !metadata.workspace_metadata.is_object() {
        return Ok((Config::default(), None));
    }

    let meta: Metadata =
        serde_json::from_value(metadata.workspace_metadata.clone()).into_diagnostic()?;

    let ws_config = meta.lambda.package.into();
    if !name.is_empty() {
        if let Some(bin_config) = name.find_binary_metadata(&meta.lambda.bin) {
            return Ok((ws_config, Some(bin_config.clone().into())));
        }
    }

    Ok((ws_config, None))
}

fn package_metadata(metadata: &CargoMetadata, name: &FunctionNames) -> Result<Option<Config>> {
    let kind_condition = |pkg: &Package, target: &Target| {
        target.kind.iter().any(|kind| kind == "bin") && pkg.metadata.is_object()
    };

    if name.is_empty() {
        if metadata.packages.len() == 1 {
            return get_config_from_root(metadata);
        }

        let targets = binary_targets_from_metadata(metadata, false);
        trace!(
            ?targets,
            "inspecting targets for a command without package name"
        );
        if targets.len() == 1 {
            let name = targets
                .into_iter()
                .next()
                .ok_or(MetadataError::MissingBinaryInProject)?;
            return get_config_from_packages(
                metadata,
                kind_condition,
                &FunctionNames::from_package(&name),
            );
        }

        return Ok(None);
    };

    get_config_from_packages(metadata, kind_condition, name)
}

fn get_config_from_packages(
    metadata: &CargoMetadata,
    kind_condition: impl Fn(&Package, &Target) -> bool,
    name: &FunctionNames,
) -> Result<Option<Config>> {
    for pkg in &metadata.packages {
        for target in &pkg.targets {
            if kind_condition(pkg, target)
                && (name.include(&target.name) || name.include(&pkg.name))
            {
                let meta: Metadata =
                    serde_json::from_value(pkg.metadata.clone()).into_diagnostic()?;

                if let Some(bin_config) = name.find_binary_metadata(&meta.lambda.bin) {
                    return Ok(Some(bin_config.clone().into()));
                }

                return Ok(Some(meta.lambda.package.into()));
            }
        }
    }

    Ok(None)
}

pub fn get_config_from_all_packages(metadata: &CargoMetadata) -> Result<HashMap<String, Config>> {
    let kind_condition = |pkg: &Package, target: &Target| {
        target.kind.iter().any(|kind| kind == "bin") && pkg.metadata.is_object()
    };

    let mut configs = HashMap::new();
    for pkg in &metadata.packages {
        for target in &pkg.targets {
            if kind_condition(pkg, target) {
                let meta: Metadata =
                    serde_json::from_value(pkg.metadata.clone()).into_diagnostic()?;

                configs.insert(pkg.name.clone(), meta.lambda.package.into());
            }
        }
    }

    Ok(configs)
}

fn get_config_from_root(metadata: &CargoMetadata) -> Result<Option<Config>> {
    let Some(root) = metadata.root_package() else {
        return Ok(None);
    };

    get_config_from_package(root)
}

fn get_config_from_package(package: &Package) -> Result<Option<Config>> {
    if package.metadata.is_null() || !package.metadata.is_object() {
        return Ok(None);
    }

    let meta: Metadata = serde_json::from_value(package.metadata.clone()).into_diagnostic()?;
    Ok(Some(meta.lambda.package.into()))
}

#[cfg(test)]
mod tests {

    use matchit::MatchError;

    use super::*;
    use crate::{
        cargo::{
            build::{CompilerOptions, OutputFormat},
            load_metadata,
        },
        lambda::Tracing,
        tests::fixture_metadata,
    };

    #[test]
    fn test_load_env_from_metadata() {
        let metadata = load_metadata(fixture_metadata("single-binary-package"), None).unwrap();
        let config = load_config_without_cli_flags(&metadata, &ConfigOptions::default()).unwrap();

        assert_eq!(
            config.deploy.lambda_tags(),
            Some(HashMap::from([
                ("organization".to_string(), "aws".to_string()),
                ("team".to_string(), "lambda".to_string())
            ]))
        );

        assert_eq!(config.env.get("FOO"), Some(&"BAR".to_string()));
        assert_eq!(config.deploy.function_config.memory, Some(512.into()));
        assert_eq!(config.deploy.function_config.timeout, Some(60.into()));
        assert_eq!(config.deploy.merge_env, true);

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
            other => panic!("unexpected compiler: {other:?}"),
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
            names: FunctionNames::from_package("crate-3"),
            admerge: true,
            ..Default::default()
        };

        let metadata = load_metadata(fixture_metadata("workspace-package"), None).unwrap();
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
            names: FunctionNames::from_package("crate-3"),
            ..Default::default()
        };

        let metadata = load_metadata(fixture_metadata("workspace-package"), None).unwrap();
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
            names: FunctionNames::from_binary("basic-lambda-1"),
            admerge: true,
            ..Default::default()
        };

        let metadata = load_metadata(fixture_metadata("workspace-package"), None).unwrap();
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

        let metadata = load_metadata(manifest, None).unwrap();
        let config = load_config_without_cli_flags(&metadata, &options).unwrap();
        assert_eq!(config.deploy.function_config.memory, Some(1024.into()));

        let options = ConfigOptions {
            context: Some("development".to_string()),
            global: Some(global.clone()),
            ..Default::default()
        };

        let config = load_config_without_cli_flags(&metadata, &options).unwrap();
        assert_eq!(config.deploy.function_config.memory, Some(512.into()));

        let options = ConfigOptions {
            global: Some(global),
            ..Default::default()
        };

        let config = load_config_without_cli_flags(&metadata, &options).unwrap();
        assert_eq!(config.deploy.function_config.memory, Some(256.into()));
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
        deploy.function_config.memory = Some(2048.into());

        let args_config = Config {
            deploy,
            ..Default::default()
        };

        let metadata = load_metadata(manifest, None).unwrap();
        let config = load_config(&args_config, &metadata, &options).unwrap();
        assert_eq!(config.deploy.function_config.memory, Some(2048.into()));
    }

    #[test]
    fn test_cargo_toml_merge_env_not_overridden_by_cli() {
        // Test that merge_env from Cargo.toml is NOT overridden when CLI doesn't set it
        let metadata = load_metadata(fixture_metadata("single-binary-package"), None).unwrap();

        // CLI with no merge_env set (should be None)
        let args_config = Config {
            deploy: Deploy::default(),
            ..Default::default()
        };

        let config = load_config(&args_config, &metadata, &ConfigOptions::default()).unwrap();

        // Should load merge_env=true from Cargo.toml
        assert_eq!(
            config.deploy.merge_env, true,
            "merge_env from Cargo.toml should be preserved when CLI doesn't set it"
        );
    }

    #[test]
    fn test_load_metadata_from_package_workspace() {
        let options = ConfigOptions {
            names: FunctionNames::from_package("package-1"),
            ..Default::default()
        };

        let metadata =
            load_metadata(fixture_metadata("workspace-with-package-config"), None).unwrap();
        let config = load_config_without_cli_flags(&metadata, &options).unwrap();

        assert_eq!(
            config.build.cargo_opts.common.features,
            vec!["lol".to_string()]
        );
        assert_eq!(config.build.output_format, Some(OutputFormat::Zip));
    }

    #[test]
    fn test_load_concurrency_from_metadata() {
        let metadata = load_metadata(fixture_metadata("single-binary-package"), None).unwrap();
        let config = load_config_without_cli_flags(&metadata, &ConfigOptions::default()).unwrap();

        assert_eq!(
            config.watch.concurrency, 5,
            "concurrency should be loaded from package metadata"
        );
    }

    #[test]
    fn test_concurrency_cli_override() {
        let metadata = load_metadata(fixture_metadata("single-binary-package"), None).unwrap();

        let mut watch = Watch::default();
        watch.concurrency = 10;

        let args_config = Config {
            watch,
            ..Default::default()
        };

        let config = load_config(&args_config, &metadata, &ConfigOptions::default()).unwrap();

        assert_eq!(
            config.watch.concurrency, 10,
            "CLI concurrency should override Cargo.toml metadata"
        );
    }

    #[test]
    fn test_load_concurrency_from_workspace_metadata() {
        let options = ConfigOptions {
            names: FunctionNames::from_package("crate-1"),
            ..Default::default()
        };

        let metadata = load_metadata(fixture_metadata("workspace-package"), None).unwrap();
        let config = load_config_without_cli_flags(&metadata, &options).unwrap();

        assert_eq!(
            config.watch.concurrency, 3,
            "concurrency should be loaded from workspace metadata"
        );
    }
}
