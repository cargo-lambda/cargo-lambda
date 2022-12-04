use cargo_metadata::{Metadata as CargoMetadata, Package};
use miette::{IntoDiagnostic, Result};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use crate::{
    error::MetadataError,
    lambda::{Memory, Timeout, Tracing},
};

#[derive(Default, Deserialize)]
#[non_exhaustive]
pub struct Metadata {
    #[serde(default)]
    pub lambda: LambdaMetadata,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[non_exhaustive]
pub struct LambdaMetadata {
    #[serde(flatten)]
    pub package: PackageMetadata,
    #[serde(default)]
    pub bin: HashMap<String, PackageMetadata>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[non_exhaustive]
pub struct PackageMetadata {
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub deploy: DeployConfig,
    #[serde(default)]
    pub build: BuildConfig,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct BuildConfig {
    pub compiler: CompilerOptions,
}

impl BuildConfig {
    pub fn is_zig_enabled(&self) -> bool {
        self.compiler == CompilerOptions::CargoZigbuild
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompilerOptions {
    #[default]
    CargoZigbuild,
    Cargo(CargoCompilerOptions),
}

impl From<String> for CompilerOptions {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            "cargo" => Self::Cargo(CargoCompilerOptions::default()),
            _ => Self::CargoZigbuild,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct CargoCompilerOptions {
    #[serde(default)]
    pub subcommand: Option<Vec<String>>,
    #[serde(default)]
    pub extra_args: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DeployConfig {
    #[serde(default)]
    pub memory: Option<Memory>,
    #[serde(default)]
    pub timeout: Timeout,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub env_file: Option<PathBuf>,
    #[serde(default)]
    pub tracing: Tracing,
    #[serde(default, alias = "role")]
    pub iam_role: Option<String>,
    #[serde(default)]
    pub layers: Option<Vec<String>>,
}

/// Extract all the binary target names from a Cargo.toml file
pub fn binary_targets<P: AsRef<Path>>(manifest_path: P) -> Result<HashSet<String>> {
    let metadata = load_metadata(manifest_path)?;
    let bins = metadata
        .packages
        .iter()
        .flat_map(|p| {
            p.targets
                .iter()
                .filter(|target| target.kind.iter().any(|k| k == "bin"))
        })
        .map(|target| target.name.clone())
        .collect::<_>();
    Ok(bins)
}

pub fn binary_targets_from_metadata(metadata: &CargoMetadata) -> Result<HashSet<String>> {
    let bins = metadata
        .packages
        .iter()
        .flat_map(|p| {
            p.targets
                .iter()
                .filter(|target| target.kind.iter().any(|k| k == "bin"))
        })
        .map(|target| target.name.clone())
        .collect::<_>();
    Ok(bins)
}

/// Extract target directory information
///
/// This fetches the target directory from `cargo metadata`, resolving the
/// user and project configuration and the environment variables in the right
/// way.
pub fn target_dir<P: AsRef<Path>>(manifest_path: P) -> Result<PathBuf> {
    let metadata = load_metadata(manifest_path)?;
    Ok(metadata.target_directory.into_std_path_buf())
}

pub fn target_dir_from_metadata(metadata: &CargoMetadata) -> Result<PathBuf> {
    Ok(metadata.target_directory.clone().into_std_path_buf())
}

/// Create metadata about the root package in the Cargo manifest, without any dependencies.
pub fn load_metadata<P: AsRef<Path>>(manifest_path: P) -> Result<CargoMetadata> {
    let mut metadata_cmd = cargo_metadata::MetadataCommand::new();
    metadata_cmd.no_deps();

    // try to split manifest path and assign current_dir to enable parsing a project-specific
    // cargo config
    let manifest_ref = manifest_path.as_ref();
    let parent = manifest_ref.parent().map(|p| p.to_path_buf());
    match (parent, manifest_ref.file_name()) {
        (Some(mut project_dir), Some(manifest_file)) => {
            if !project_dir.is_dir() {
                project_dir = std::env::current_dir().into_diagnostic()?;
            }
            metadata_cmd.current_dir(project_dir);
            metadata_cmd.manifest_path(manifest_file);
        }
        _ => {
            // fall back to using the manifest_path without changing the dir
            // this means there will not be any proejct-specific config parsing
            metadata_cmd.manifest_path(manifest_ref);
        }
    }

    metadata_cmd.exec().into_diagnostic()
}

/// Create a HashMap of environment varibales from the package and workspace manifest
/// See the documentation to learn about how we use this metadata:
/// https://www.cargo-lambda.info/commands/watch.html#environment-variables
pub fn function_environment_metadata<P: AsRef<Path>>(
    manifest_path: P,
    name: Option<&str>,
) -> Result<HashMap<String, String>> {
    let metadata = load_metadata(manifest_path)?;
    let ws_metadata: LambdaMetadata =
        serde_json::from_value(metadata.workspace_metadata).unwrap_or_default();

    let mut env = HashMap::new();
    env.extend(ws_metadata.package.env);

    if let Some(name) = name {
        if let Some(res) = ws_metadata.bin.get(name) {
            env.extend(res.env.clone());
        }
    }

    for pkg in &metadata.packages {
        let name = name.unwrap_or(&pkg.name);

        for target in &pkg.targets {
            let target_matches = target.name == name
                && target.kind.iter().any(|kind| kind == "bin")
                && pkg.metadata.is_object();

            tracing::debug!(
                name = name,
                target_matches = target_matches,
                "searching package metadata"
            );

            if target_matches {
                let package_metadata: Metadata = serde_json::from_value(pkg.metadata.clone())
                    .map_err(MetadataError::InvalidCargoMetadata)?;

                env.extend(package_metadata.lambda.package.env);
                if let Some(res) = package_metadata.lambda.bin.get(name) {
                    env.extend(res.env.clone());
                }
            }
        }
    }

    if !env.is_empty() {
        tracing::debug!(env = ?env, "using environment variables from metadata");
    }
    Ok(env)
}

/// Create a `DeployConfig` struct from Cargo metadata.
/// This configuration can be overwritten by flags from the cli.
pub fn function_deploy_metadata<P: AsRef<Path>>(
    manifest_path: P,
    name: &str,
) -> Result<DeployConfig> {
    let metadata = load_metadata(manifest_path)?;
    let ws_metadata: LambdaMetadata =
        serde_json::from_value(metadata.workspace_metadata).unwrap_or_default();

    let mut config = ws_metadata.package.deploy;

    if let Some(package_metadata) = ws_metadata.bin.get(name) {
        merge_deploy_config(&mut config, &package_metadata.deploy);
    }

    for pkg in &metadata.packages {
        for target in &pkg.targets {
            let target_matches = target.name == name
                && target.kind.iter().any(|kind| kind == "bin")
                && pkg.metadata.is_object();

            tracing::debug!(
                name = name,
                target_matches = target_matches,
                "searching package metadata"
            );

            if target_matches {
                let package_metadata: Metadata = serde_json::from_value(pkg.metadata.clone())
                    .map_err(MetadataError::InvalidCargoMetadata)?;
                let package_deploy = package_metadata.lambda.package.deploy;

                merge_deploy_config(&mut config, &package_deploy);
            }
        }
    }

    tracing::debug!(config = ?config, "using deploy configuration from metadata");
    Ok(config)
}

/// Create a `BuildConfig` struct from Cargo metadata.
/// This configuration can be overwritten by flags from the cli.
/// This function loads the workspace configuration that's merged
/// with the configuration from the first binary target in the project.
/// It assumes that all functions in the workspace will use the same compiler configuration.
pub fn function_build_metadata(metadata: &CargoMetadata) -> Result<BuildConfig> {
    let ws_metadata: LambdaMetadata =
        serde_json::from_value(metadata.workspace_metadata.clone()).unwrap_or_default();

    let mut config = ws_metadata.package.build;

    'outer: for pkg in &metadata.packages {
        for target in &pkg.targets {
            if target.kind.iter().any(|kind| kind == "bin") && pkg.metadata.is_object() {
                let package_metadata: Metadata = serde_json::from_value(pkg.metadata.clone())
                    .map_err(MetadataError::InvalidCargoMetadata)?;
                let package_build = package_metadata.lambda.package.build;

                merge_build_config(&mut config, &package_build);
                break 'outer;
            }
        }
    }

    tracing::debug!(config = ?config, "using build compiler configuration from metadata");
    Ok(config)
}

/// Load the main package in the project.
/// It returns an error if the project includes from than one package.
/// Use this function when the user didn't provide any funcion name
/// assuming that there is only one package in the project
pub fn root_package<P: AsRef<Path>>(manifest_path: P) -> Result<Package> {
    let metadata = load_metadata(manifest_path)?;
    if metadata.packages.len() > 1 {
        Err(MetadataError::MultiplePackagesInProject)?;
    } else if metadata.packages.is_empty() {
        Err(MetadataError::MissingPackageInProject)?;
    }

    Ok(metadata
        .packages
        .into_iter()
        .next()
        .expect("failed to extract the root package from the metadata"))
}

fn merge_deploy_config(base: &mut DeployConfig, package_deploy: &DeployConfig) {
    if package_deploy.memory.is_some() {
        base.memory = package_deploy.memory.clone();
    }
    if !package_deploy.timeout.is_zero() {
        base.timeout = package_deploy.timeout.clone();
    }
    base.env.extend(package_deploy.env.clone());
    if package_deploy.env_file.is_some() && base.env_file.is_none() {
        base.env_file = package_deploy.env_file.clone();
    }
    if package_deploy.tracing != Tracing::default() {
        base.tracing = package_deploy.tracing.clone();
    }
    if package_deploy.iam_role.is_some() {
        base.iam_role = package_deploy.iam_role.clone();
    }
    if package_deploy.layers.is_some() {
        base.layers = package_deploy.layers.clone();
    }
    tracing::debug!(ws_metadata = ?base, package_metadata = ?package_deploy, "finished merging deploy metadata");
}

fn merge_build_config(base: &mut BuildConfig, package_build: &BuildConfig) {
    if package_build.compiler != base.compiler {
        base.compiler = package_build.compiler.clone();
    }
    tracing::debug!(ws_metadata = ?base, package_metadata = ?package_build, "finished merging build metadata");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        format!("../../test/fixtures/{name}/Cargo.toml").into()
    }

    #[test]
    fn test_binary_packages() {
        let bins = binary_targets(fixture("single-binary-package")).unwrap();
        assert_eq!(1, bins.len());
        assert!(bins.contains("basic-lambda"));
    }

    #[test]
    fn test_binary_packages_with_mutiple_bin_entries() {
        let bins = binary_targets(fixture("multi-binary-package")).unwrap();
        assert_eq!(5, bins.len());
        assert!(bins.contains("delete-product"));
        assert!(bins.contains("get-product"));
        assert!(bins.contains("get-products"));
        assert!(bins.contains("put-product"));
        assert!(bins.contains("dynamodb-streams"));
    }

    #[test]
    fn test_binary_packages_with_workspace() {
        let bins = binary_targets(fixture("workspace-package")).unwrap();
        assert_eq!(2, bins.len());
        assert!(bins.contains("basic-lambda-1"));
        assert!(bins.contains("basic-lambda-2"));
    }

    #[test]
    fn test_binary_packages_with_mixed_workspace() {
        let bins = binary_targets(fixture("mixed-workspace-package")).unwrap();
        assert_eq!(1, bins.len());
        assert!(bins.contains("function-crate"), "{:?}", bins);
    }

    #[test]
    fn test_binary_packages_with_missing_binary_info() {
        let err = binary_targets(fixture("missing-binary-package")).unwrap_err();
        assert!(err
            .to_string()
            .contains("a [lib] section, or [[bin]] section must be present"));
    }

    #[test]
    fn test_metadata_packages() {
        let env =
            function_environment_metadata(fixture("single-binary-package"), Some("basic-lambda"))
                .unwrap();

        assert_eq!(env.get("FOO").unwrap(), "BAR");
    }

    #[test]
    fn test_deploy_metadata_packages() {
        let env =
            function_deploy_metadata(fixture("single-binary-package"), "basic-lambda").unwrap();

        let layers = [
            "arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer1".to_string(),
            "arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer2".to_string(),
        ];

        let mut vars = HashMap::new();
        vars.insert("VAR1".to_string(), "VAL1".to_string());

        assert_eq!(Some(Memory::Mb512), env.memory);
        assert_eq!(Timeout::new(60), env.timeout);
        assert_eq!(Some(Path::new(".env.production")), env.env_file.as_deref());
        assert_eq!(Some(layers.to_vec()), env.layers);
        assert_eq!(Tracing::Active, env.tracing);
        assert_eq!(vars, env.env);
        assert_eq!(
            Some("arn:aws:lambda:us-east-1:xxxxxxxx:iam:role1".to_string()),
            env.iam_role
        );
    }

    #[test]
    fn test_metadata_multi_packages() {
        let env =
            function_environment_metadata(fixture("multi-binary-package"), Some("get-product"))
                .unwrap();

        assert_eq!(env.get("FOO").unwrap(), "BAR");

        let env =
            function_environment_metadata(fixture("multi-binary-package"), Some("delete-product"))
                .unwrap();

        assert_eq!(env.get("BAZ").unwrap(), "QUX");
    }

    #[test]
    fn test_metadata_workspace_packages() {
        let env =
            function_environment_metadata(fixture("workspace-package"), Some("basic-lambda-1"))
                .unwrap();

        assert_eq!(env.get("FOO").unwrap(), "BAR");

        let env =
            function_environment_metadata(fixture("workspace-package"), Some("basic-lambda-2"))
                .unwrap();

        assert_eq!(env.get("FOO").unwrap(), "BAR");
    }

    #[test]
    fn test_metadata_packages_without_name() {
        let env = function_environment_metadata(fixture("single-binary-package"), None).unwrap();

        assert_eq!(env.get("FOO").unwrap(), "BAR");
    }

    #[test]
    #[ignore = "changing the environment is not reliable"]
    fn test_target_dir_non_set() {
        std::env::remove_var("CARGO_TARGET_DIR");
        let target_dir = target_dir(fixture("single-binary-package")).unwrap();
        assert!(
            target_dir.ends_with("test/fixtures/single-binary-package/target"),
            "unexpected directory {:?}",
            target_dir
        );
    }

    #[test]
    #[ignore = "changing the environment is not reliable"]
    fn test_target_dir_from_project_config() {
        std::env::remove_var("CARGO_TARGET_DIR");
        let target_dir = target_dir(fixture("target-dir-set-in-project")).unwrap();
        assert!(
            target_dir.ends_with("project_specific_target"),
            "unexpected directory {:?}",
            target_dir
        );
    }

    #[test]
    #[ignore = "changing the environment is not reliable"]
    fn test_target_dir_from_env() {
        std::env::set_var("CARGO_TARGET_DIR", "/tmp/exotic_path");
        let target_dir = target_dir(fixture("single-binary-package")).unwrap();
        assert!(
            target_dir.ends_with("/tmp/exotic_path"),
            "unexpected directory {:?}",
            target_dir
        );
    }

    #[test]
    fn test_build_config_metadata() {
        let manifest_path = fixture("single-binary-package");
        let metadata = load_metadata(manifest_path).unwrap();

        let env = function_build_metadata(&metadata).unwrap();

        let opts = match env.compiler {
            CompilerOptions::Cargo(opts) => opts,
            other => panic!("unexpected compiler: {:?}", other),
        };

        let subcommand = opts.subcommand.unwrap();
        assert_eq!(vec!["brazil".to_string(), "build".to_string()], subcommand);
    }
}
