pub use cargo_metadata::{
    Metadata as CargoMetadata, Package as CargoPackage, Target as CargoTarget,
};
use miette::Result;
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    fs::{metadata, read_to_string},
    path::{Path, PathBuf},
};
use tracing::{debug, enabled, trace, Level};
use urlencoding::encode;

use crate::{
    env::{lambda_environment, Environment},
    error::MetadataError,
    lambda::{Memory, Timeout, Tracing},
};

const STRIP_CONFIG: &str = "profile.release.strip=\"symbols\"";
const LTO_CONFIG: &str = "profile.release.lto=\"thin\"";
const CODEGEN_CONFIG: &str = "profile.release.codegen-units=1";
const PANIC_CONFIG: &str = "profile.release.panic=\"abort\"";

#[derive(Debug, Default, Deserialize)]
#[non_exhaustive]
pub struct Metadata {
    #[serde(default)]
    pub lambda: LambdaMetadata,
    #[serde(default)]
    profile: Option<CargoProfile>,
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
    pub deploy: Option<DeployConfig>,
    #[serde(default)]
    pub build: BuildConfig,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct BuildConfig {
    pub compiler: Option<CompilerOptions>,
    pub target: Option<String>,
    #[serde(default)]
    pub include: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CompilerOptions {
    #[default]
    CargoZigbuild,
    Cargo(CargoCompilerOptions),
    Cross,
}

impl From<String> for CompilerOptions {
    fn from(s: String) -> Self {
        match s.to_lowercase().as_str() {
            "cargo" => Self::Cargo(CargoCompilerOptions::default()),
            "cross" => Self::Cross,
            _ => Self::CargoZigbuild,
        }
    }
}

impl CompilerOptions {
    pub fn is_local_cargo(&self) -> bool {
        matches!(self, CompilerOptions::Cargo(_))
    }

    pub fn is_cargo_zigbuild(&self) -> bool {
        matches!(self, CompilerOptions::CargoZigbuild)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
pub struct CargoCompilerOptions {
    #[serde(default)]
    pub subcommand: Option<Vec<String>>,
    #[serde(default)]
    pub extra_args: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct CargoProfile {
    pub release: Option<CargoProfileRelease>,
}

#[derive(Debug, Default, Deserialize)]
struct CargoProfileRelease {
    strip: Option<toml::Value>,
    lto: Option<toml::Value>,
    #[serde(rename = "codegen-units")]
    codegen_units: Option<toml::Value>,
    panic: Option<toml::Value>,
    #[serde(default = "default_cargo_bool")]
    debug: CargoBool,
}

impl CargoProfileRelease {
    fn debug_enabled(&self) -> bool {
        !(self.debug == CargoBool::Str("none".to_string())
            || self.debug == CargoBool::Num(0)
            || self.debug == CargoBool::Bool(false))
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq)]
#[serde(untagged)]
enum CargoBool {
    Bool(bool),
    Num(u8),
    Str(String),
}

impl Default for CargoBool {
    fn default() -> Self {
        default_cargo_bool()
    }
}

fn default_cargo_bool() -> CargoBool {
    CargoBool::Bool(false)
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DeployConfig {
    #[serde(default)]
    pub memory: Option<Memory>,
    #[serde(default)]
    pub timeout: Option<Timeout>,
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
    #[serde(default)]
    pub tags: Option<HashMap<String, String>>,
    #[serde(skip)]
    pub use_for_update: bool,
    #[serde(default)]
    pub subnet_ids: Option<Vec<String>>,
    #[serde(default)]
    pub security_group_ids: Option<Vec<String>>,
    #[serde(default = "default_runtime")]
    pub runtime: String,
    #[serde(default)]
    pub include: Option<Vec<String>>,
    #[serde(default)]
    pub s3_bucket: Option<String>,
    #[serde(default)]
    pub s3_key: Option<String>,
}

fn default_runtime() -> String {
    "provided.al2023".to_string()
}

impl DeployConfig {
    pub fn append_tags(&mut self, tags: HashMap<String, String>) {
        self.tags = match &self.tags {
            None => Some(tags),
            Some(base) => {
                let mut new_tags = base.clone();
                new_tags.extend(tags);
                Some(new_tags)
            }
        }
    }

    pub fn s3_tags(&self) -> Option<String> {
        match &self.tags {
            None => None,
            Some(tags) if tags.is_empty() => None,
            Some(tags) => {
                let mut vec = Vec::new();
                for (k, v) in tags {
                    vec.push(format!("{}={}", encode(k), encode(v)));
                }
                Some(vec.join("&"))
            }
        }
    }

    pub fn lambda_environment(&self) -> Result<Environment, MetadataError> {
        let base = if self.env.is_empty() {
            None
        } else {
            Some(&self.env)
        };
        lambda_environment(base, &self.env_file, None)
    }

    pub fn extend_environment(
        &mut self,
        extra: &HashMap<String, String>,
    ) -> Result<Environment, MetadataError> {
        let mut env = lambda_environment(Some(&self.env), &self.env_file, None)?;
        for (key, value) in extra {
            env.insert(key.clone(), value.clone());
        }
        Ok(env)
    }
}

/// Extract all the binary target names from a Cargo.toml file
pub fn binary_targets<P: AsRef<Path> + Debug>(
    manifest_path: P,
    build_examples: bool,
) -> Result<HashSet<String>, MetadataError> {
    let metadata = load_metadata(manifest_path)?;
    Ok(binary_targets_from_metadata(&metadata, build_examples))
}

pub fn binary_targets_from_metadata(
    metadata: &CargoMetadata,
    build_examples: bool,
) -> HashSet<String> {
    let condition = if build_examples {
        kind_example_filter
    } else {
        kind_bin_filter
    };

    let package_filter: Option<fn(&&CargoPackage) -> bool> = None;
    filter_binary_targets_from_metadata(metadata, condition, package_filter)
}

pub fn kind_bin_filter(target: &CargoTarget) -> bool {
    target.kind.iter().any(|k| k == "bin")
}

// Several targets can have `crate_type` be `bin`, we're only
// interested in the ones which `kind` is `bin` or `example`.
// See https://doc.rust-lang.org/cargo/commands/cargo-metadata.html?highlight=targets%20metadata#json-format
pub fn kind_example_filter(target: &CargoTarget) -> bool {
    target.kind.iter().any(|k| k == "example") && target.crate_types.iter().any(|t| t == "bin")
}

/// Extract all the binary target names from a Cargo.toml file
pub fn filter_binary_targets<P, F, K>(
    manifest_path: P,
    target_filter: F,
    package_filter: Option<K>,
) -> Result<HashSet<String>, MetadataError>
where
    P: AsRef<Path> + Debug,
    F: FnMut(&CargoTarget) -> bool,
    K: FnMut(&&CargoPackage) -> bool,
{
    let metadata = load_metadata(manifest_path)?;
    Ok(filter_binary_targets_from_metadata(
        &metadata,
        target_filter,
        package_filter,
    ))
}

pub fn filter_binary_targets_from_metadata<F, P>(
    metadata: &CargoMetadata,
    target_filter: F,
    package_filter: Option<P>,
) -> HashSet<String>
where
    F: FnMut(&CargoTarget) -> bool,
    P: FnMut(&&CargoPackage) -> bool,
{
    let packages = metadata.packages.iter();
    let targets = if let Some(filter) = package_filter {
        packages
            .filter(filter)
            .flat_map(|p| p.targets.clone())
            .collect::<Vec<_>>()
    } else {
        packages.flat_map(|p| p.targets.clone()).collect::<Vec<_>>()
    };

    targets
        .into_iter()
        .filter(target_filter)
        .map(|target| target.name.clone())
        .collect::<_>()
}

/// Extract target directory information
///
/// This fetches the target directory from `cargo metadata`, resolving the
/// user and project configuration and the environment variables in the right
/// way.
pub fn target_dir<P: AsRef<Path> + Debug>(manifest_path: P) -> Result<PathBuf> {
    let metadata = load_metadata(manifest_path)?;
    Ok(metadata.target_directory.into_std_path_buf())
}

pub fn target_dir_from_metadata(metadata: &CargoMetadata) -> Result<PathBuf> {
    Ok(metadata.target_directory.clone().into_std_path_buf())
}

/// Attempt to read the releaes profile section in the Cargo manifest.
/// Cargo metadata doesn't expose profile information, so we try
/// to read it from the Cargo.toml file directly.
pub fn cargo_release_profile_config<'a, P: AsRef<Path> + Debug>(
    manifest_path: P,
) -> Result<HashSet<&'a str>, MetadataError> {
    let path = manifest_path.as_ref();
    let file = read_to_string(path)
        .map_err(|e| MetadataError::InvalidManifestFile(path.to_path_buf(), e))?;

    let metadata: Metadata = toml::from_str(&file).map_err(MetadataError::InvalidTomlManifest)?;

    Ok(cargo_release_profile_config_from_metadata(metadata))
}

fn cargo_release_profile_config_from_metadata(metadata: Metadata) -> HashSet<&'static str> {
    let mut config = HashSet::from([STRIP_CONFIG, LTO_CONFIG, CODEGEN_CONFIG, PANIC_CONFIG]);

    let Some(profile) = &metadata.profile else {
        return config;
    };
    let Some(release) = &profile.release else {
        return config;
    };

    if release.strip.is_some() || release.debug_enabled() {
        config.remove(STRIP_CONFIG);
    }
    if release.lto.is_some() {
        config.remove(LTO_CONFIG);
    }
    if release.codegen_units.is_some() {
        config.remove(CODEGEN_CONFIG);
    }
    if release.panic.is_some() {
        config.remove(PANIC_CONFIG);
    }

    config
}

/// Create metadata about the root package in the Cargo manifest, without any dependencies.
#[tracing::instrument(target = "cargo_lambda")]
pub fn load_metadata<P: AsRef<Path> + Debug>(
    manifest_path: P,
) -> Result<CargoMetadata, MetadataError> {
    trace!("loading Cargo metadata");
    let mut metadata_cmd = cargo_metadata::MetadataCommand::new();
    metadata_cmd
        .no_deps()
        .verbose(enabled!(target: "cargo_lambda", Level::TRACE));

    // try to split manifest path and assign current_dir to enable parsing a project-specific
    // cargo config
    let manifest_ref = manifest_path.as_ref();

    match (manifest_ref.parent(), manifest_ref.file_name()) {
        (Some(project), Some(manifest)) if is_project_metadata_ok(project) => {
            metadata_cmd.current_dir(project);
            metadata_cmd.manifest_path(manifest);
        }
        _ => {
            // fall back to using the manifest_path without changing the dir
            // this means there will not be any project-specific config parsing
            metadata_cmd.manifest_path(manifest_ref);
        }
    }

    trace!(metadata = ?metadata_cmd, "loading cargo metadata");
    let meta = metadata_cmd
        .exec()
        .map_err(MetadataError::FailedCmdExecution)?;
    trace!(metadata = ?meta, "loaded cargo metadata");
    Ok(meta)
}

/// Create a HashMap of environment varibales from the package and workspace manifest
/// See the documentation to learn about how we use this metadata:
/// https://www.cargo-lambda.info/commands/watch.html#environment-variables
#[tracing::instrument(target = "cargo_lambda")]
pub fn function_environment_metadata<P: AsRef<Path> + Debug>(
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

            debug!(
                name = name,
                target_name = ?target.name,
                target_kind = ?target.kind,
                metadata_object = pkg.metadata.is_object(),
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

    debug!(env = ?env, "using environment variables from metadata");
    Ok(env)
}

/// Create a `DeployConfig` struct from Cargo metadata.
/// This configuration can be overwritten by flags from the cli.
#[tracing::instrument(target = "cargo_lambda")]
pub fn function_deploy_metadata<P: AsRef<Path> + Debug>(
    manifest_path: P,
    name: &str,
    tags: &Option<Vec<String>>,
    s3_bucket: &Option<String>,
    s3_key: &Option<String>,
    default: DeployConfig,
) -> Result<DeployConfig, MetadataError> {
    let metadata = load_metadata(manifest_path)?;
    let ws_metadata: LambdaMetadata =
        serde_json::from_value(metadata.workspace_metadata).unwrap_or_default();

    let mut config = ws_metadata.package.deploy.unwrap_or(default);

    if let Some(package_metadata) = ws_metadata.bin.get(name) {
        merge_deploy_config(&mut config, &package_metadata.deploy);
    }

    for pkg in &metadata.packages {
        for target in &pkg.targets {
            let target_matches = target.name == name
                && target.kind.iter().any(|kind| kind == "bin")
                && pkg.metadata.is_object();

            tracing::debug!(name, target_matches, "searching package metadata");

            if target_matches {
                let package_metadata: Metadata = serde_json::from_value(pkg.metadata.clone())
                    .map_err(MetadataError::InvalidCargoMetadata)?;
                let package_config = package_metadata.lambda.package.deploy;
                merge_deploy_config(&mut config, &package_config);

                break;
            }
        }
    }

    if let Some(tags) = tags {
        config.append_tags(extract_tags(tags));
    }

    if config.s3_bucket.is_none() {
        config.s3_bucket.clone_from(s3_bucket);
    }

    if config.s3_key.is_none() {
        config.s3_key.clone_from(s3_key);
    }

    tracing::debug!(?config, "using deploy configuration from metadata");
    Ok(config)
}

/// Create a `BuildConfig` struct from Cargo metadata.
/// This configuration can be overwritten by flags from the cli.
/// This function loads the workspace configuration that's merged
/// with the configuration from the first binary target in the project.
/// It assumes that all functions in the workspace will use the same compiler configuration.
pub fn function_build_metadata(metadata: &CargoMetadata) -> Result<BuildConfig, MetadataError> {
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

/// Load the main binary in the project.
/// It returns an error if the project includes from than one binary.
/// Use this function when the user didn't provide any funcion name
/// assuming that there is only one binary in the project
pub fn main_binary<P: AsRef<Path> + Debug>(manifest_path: P) -> Result<String, MetadataError> {
    let targets = binary_targets(manifest_path, false)?;
    if targets.len() > 1 {
        let mut vec = targets.into_iter().collect::<Vec<_>>();
        vec.sort();
        Err(MetadataError::MultipleBinariesInProject(vec.join(", ")))
    } else if targets.is_empty() {
        Err(MetadataError::MissingBinaryInProject)
    } else {
        targets
            .into_iter()
            .next()
            .ok_or_else(|| MetadataError::MissingBinaryInProject)
    }
}

fn merge_deploy_config(base: &mut DeployConfig, package_deploy: &Option<DeployConfig>) {
    let Some(package_deploy) = package_deploy else {
        return;
    };

    if package_deploy.memory.is_some() {
        base.memory.clone_from(&package_deploy.memory);
    }
    if let Some(package_timeout) = &package_deploy.timeout {
        if !package_timeout.is_zero() {
            base.timeout = Some(package_timeout.clone());
        }
    }
    base.env.extend(package_deploy.env.clone());
    if package_deploy.env_file.is_some() && base.env_file.is_none() {
        base.env_file.clone_from(&package_deploy.env_file);
    }
    if package_deploy.tracing != Tracing::default() {
        base.tracing = package_deploy.tracing.clone();
    }
    if package_deploy.iam_role.is_some() {
        base.iam_role.clone_from(&package_deploy.iam_role);
    }
    if package_deploy.layers.is_some() {
        base.layers.clone_from(&package_deploy.layers);
    }
    if package_deploy.subnet_ids.is_some() {
        base.subnet_ids.clone_from(&package_deploy.subnet_ids);
    }
    if package_deploy.security_group_ids.is_some() {
        base.security_group_ids
            .clone_from(&package_deploy.security_group_ids);
    }
    base.runtime.clone_from(&package_deploy.runtime);
    if let Some(package_include) = &package_deploy.include {
        let mut include = base.include.clone().unwrap_or_default();
        include.extend(package_include.clone());
        base.include = Some(include);
    }
    if package_deploy.s3_bucket.is_some() {
        base.s3_bucket.clone_from(&package_deploy.s3_bucket);
    }
    if let Some(package_tags) = &package_deploy.tags {
        let mut tags = base.tags.clone().unwrap_or_default();
        tags.extend(package_tags.clone());
        base.tags = Some(tags);
    }

    tracing::debug!(ws_metadata = ?base, package_metadata = ?package_deploy, "finished merging deploy metadata");
}

fn merge_build_config(base: &mut BuildConfig, package_build: &BuildConfig) {
    if package_build.compiler != base.compiler {
        base.compiler.clone_from(&package_build.compiler);
    }
    if package_build.target != base.target {
        base.target.clone_from(&package_build.target);
    }
    if package_build.include != base.include {
        base.include.clone_from(&package_build.include);
    }
    tracing::debug!(ws_metadata = ?base, package_metadata = ?package_build, "finished merging build metadata");
}

fn is_project_metadata_ok(path: &Path) -> bool {
    path.is_dir() && metadata(path).is_ok()
}

pub(crate) fn extract_tags(tags: &Vec<String>) -> HashMap<String, String> {
    let mut map = HashMap::new();

    for var in tags {
        let mut split = var.splitn(2, '=');
        if let (Some(k), Some(v)) = (split.next(), split.next()) {
            map.insert(k.to_string(), v.to_string());
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        format!("../../tests/fixtures/{name}/Cargo.toml").into()
    }

    #[test]
    fn test_binary_packages() {
        let bins = binary_targets(fixture("single-binary-package"), false).unwrap();
        assert_eq!(1, bins.len());
        assert!(bins.contains("basic-lambda"));
    }

    #[test]
    fn test_binary_packages_with_mutiple_bin_entries() {
        let bins = binary_targets(fixture("multi-binary-package"), false).unwrap();
        assert_eq!(5, bins.len());
        assert!(bins.contains("delete-product"));
        assert!(bins.contains("get-product"));
        assert!(bins.contains("get-products"));
        assert!(bins.contains("put-product"));
        assert!(bins.contains("dynamodb-streams"));
    }

    #[test]
    fn test_binary_packages_with_workspace() {
        let bins = binary_targets(fixture("workspace-package"), false).unwrap();
        assert_eq!(2, bins.len());
        assert!(bins.contains("basic-lambda-1"));
        assert!(bins.contains("basic-lambda-2"));
    }

    #[test]
    fn test_binary_packages_with_mixed_workspace() {
        let bins = binary_targets(fixture("mixed-workspace-package"), false).unwrap();
        assert_eq!(1, bins.len());
        assert!(bins.contains("function-crate"), "{:?}", bins);
    }

    #[test]
    fn test_binary_packages_with_missing_binary_info() {
        let err = binary_targets(fixture("missing-binary-package"), false).unwrap_err();
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
        let env = function_deploy_metadata(
            fixture("single-binary-package"),
            "basic-lambda",
            &None,
            &None,
            &None,
            DeployConfig::default(),
        )
        .unwrap();

        let layers = [
            "arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer1".to_string(),
            "arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer2".to_string(),
        ];

        let mut vars = HashMap::new();
        vars.insert("VAR1".to_string(), "VAL1".to_string());

        assert_eq!(Some(Memory::Mb512), env.memory);
        assert_eq!(Some(Timeout::new(60)), env.timeout);
        assert_eq!(Some(Path::new(".env.production")), env.env_file.as_deref());
        assert_eq!(Some(layers.to_vec()), env.layers);
        assert_eq!(Tracing::Active, env.tracing);
        assert_eq!(vars, env.env);
        assert_eq!(
            Some("arn:aws:lambda:us-east-1:xxxxxxxx:iam:role1".to_string()),
            env.iam_role
        );

        let mut tags = HashMap::new();
        tags.insert("organization".to_string(), "aws".to_string());
        tags.insert("team".to_string(), "lambda".to_string());

        assert_eq!(Some(tags), env.tags);
        let s3_tags = env.s3_tags().unwrap();
        assert_eq!(2, s3_tags.split('&').collect::<Vec<_>>().len());
        assert!(s3_tags.contains("organization=aws"), "{s3_tags}");
        assert!(s3_tags.contains("team=lambda"), "{s3_tags}");
    }

    #[test]
    fn test_deploy_metadata_packages_with_tags() {
        let tags = vec!["FOO=bar".into()];
        let env = function_deploy_metadata(
            fixture("single-binary-package"),
            "basic-lambda",
            &Some(tags),
            &None,
            &None,
            DeployConfig::default(),
        )
        .unwrap();

        let mut tags = HashMap::new();
        tags.insert("organization".to_string(), "aws".to_string());
        tags.insert("team".to_string(), "lambda".to_string());
        tags.insert("FOO".to_string(), "bar".to_string());

        assert_eq!(Some(tags), env.tags);
    }

    #[test]
    fn test_deploy_metadata_packages_with_s3_bucket() {
        let env = function_deploy_metadata(
            fixture("single-binary-package"),
            "basic-lambda",
            &None,
            &Some("deploy-bucket".into()),
            &None,
            DeployConfig::default(),
        )
        .unwrap();

        assert_eq!(Some("deploy-bucket".to_string()), env.s3_bucket);
    }

    #[test]
    fn test_deploy_metadata_packages_with_s3_bucket_and_key() {
        let env = function_deploy_metadata(
            fixture("single-binary-package"),
            "basic-lambda",
            &None,
            &Some("deploy-bucket".into()),
            &Some("prefix/name".into()),
            DeployConfig::default(),
        )
        .unwrap();

        assert_eq!(Some("deploy-bucket".to_string()), env.s3_bucket);
        assert_eq!(Some("prefix/name".to_string()), env.s3_key);
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
    fn test_invalid_metadata() {
        let result =
            function_environment_metadata(fixture("missing-binary-package"), Some("get-products"));
        assert!(result.is_err());
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
            target_dir.ends_with("tests/fixtures/single-binary-package/target"),
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

        let opts = match env.compiler.unwrap() {
            CompilerOptions::Cargo(opts) => opts,
            other => panic!("unexpected compiler: {:?}", other),
        };

        let subcommand = opts.subcommand.unwrap();
        assert_eq!(vec!["brazil".to_string(), "build".to_string()], subcommand);
    }

    #[test]
    fn test_build_config_metadata_include() {
        let manifest_path = fixture("single-binary-package-build-include");
        let metadata = load_metadata(manifest_path).unwrap();

        let env = function_build_metadata(&metadata).unwrap();
        assert_eq!(Some(vec!["Cargo.toml".into()]), env.include);
    }

    #[test]
    fn test_deploy_lambda_env() {
        let mut d = DeployConfig::default();
        let env = d.lambda_environment().unwrap();
        assert!(env.is_empty());

        let mut extra = HashMap::new();
        extra.insert("FOO".to_string(), "BAR".to_string());

        let vars = d.extend_environment(&extra).unwrap();
        assert_eq!(1, vars.len());
        assert_eq!("BAR", vars["FOO"]);

        let mut base = HashMap::new();
        base.insert("BAZ".to_string(), "QUX".to_string());
        d.env = base;

        let env = d.extend_environment(&extra).unwrap();
        assert_eq!(2, env.len());
        assert_eq!("BAR", env["FOO"]);
        assert_eq!("QUX", env["BAZ"]);
    }

    #[test]
    fn test_main_binary_with_package_name() {
        let manifest_path = fixture("single-binary-package");
        let name = main_binary(manifest_path).unwrap();
        assert_eq!("basic-lambda", name);
    }

    #[test]
    fn test_main_binary_with_binary_name() {
        let manifest_path = fixture("single-binary-different-name");
        let name = main_binary(manifest_path).unwrap();
        assert_eq!("basic-lambda-binary", name);
    }

    #[test]
    fn test_main_binary_multi_binaries() {
        let manifest_path = fixture("multi-binary-package");
        let err = main_binary(manifest_path).unwrap_err();
        assert_eq!(
            "there are more than one binary in the project, please specify a binary name with --binary-name or --binary-path. This is the list of binaries I found: delete-product, dynamodb-streams, get-product, get-products, put-product",
            err.to_string()
        );
    }

    #[test]
    fn test_s3_tags_encoding() {
        let mut tags = HashMap::new();
        tags.insert(
            "organization".to_string(),
            "Amazon Web Services".to_string(),
        );
        tags.insert("team".to_string(), "Simple Storage Service".to_string());

        let config = DeployConfig {
            tags: Some(tags),
            ..Default::default()
        };

        let s3_tags = config.s3_tags().unwrap();
        assert_eq!(2, s3_tags.split('&').collect::<Vec<_>>().len());
        assert!(
            s3_tags.contains("organization=Amazon%20Web%20Services"),
            "{s3_tags}"
        );
        assert!(
            s3_tags.contains("team=Simple%20Storage%20Service"),
            "{s3_tags}"
        );
    }

    #[test]
    fn test_example_packages() {
        let bins = binary_targets(fixture("examples-package"), true).unwrap();
        assert_eq!(1, bins.len());
        assert!(bins.contains("example-lambda"));
    }

    #[test]
    fn test_release_config() {
        let config = cargo_release_profile_config_from_metadata(Metadata::default());
        assert!(config.contains(STRIP_CONFIG));
        assert!(config.contains(LTO_CONFIG));
        assert!(config.contains(CODEGEN_CONFIG));
        assert!(config.contains(PANIC_CONFIG));
    }

    #[test]
    fn test_release_config_exclude_strip() {
        let meta = Metadata {
            profile: Some(CargoProfile {
                release: Some(CargoProfileRelease {
                    strip: Some(toml::Value::String("none".into())),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let config = cargo_release_profile_config_from_metadata(meta);
        assert!(!config.contains(STRIP_CONFIG));

        let meta = Metadata {
            profile: Some(CargoProfile {
                release: Some(CargoProfileRelease {
                    debug: CargoBool::Bool(true),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let config = cargo_release_profile_config_from_metadata(meta);
        assert!(!config.contains(STRIP_CONFIG));
    }

    #[test]
    fn test_release_config_exclude_lto() {
        let meta = Metadata {
            profile: Some(CargoProfile {
                release: Some(CargoProfileRelease {
                    lto: Some(toml::Value::String("none".into())),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let config = cargo_release_profile_config_from_metadata(meta);
        assert!(!config.contains(LTO_CONFIG));
    }

    #[test]
    fn test_release_config_exclude_codegen() {
        let meta = Metadata {
            profile: Some(CargoProfile {
                release: Some(CargoProfileRelease {
                    codegen_units: Some(toml::Value::Integer(2)),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let config = cargo_release_profile_config_from_metadata(meta);
        assert!(!config.contains(CODEGEN_CONFIG));
    }

    #[test]
    fn test_release_config_exclude_panic() {
        let meta = Metadata {
            profile: Some(CargoProfile {
                release: Some(CargoProfileRelease {
                    panic: Some(toml::Value::String("none".into())),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let config = cargo_release_profile_config_from_metadata(meta);
        assert!(!config.contains(PANIC_CONFIG));
    }

    #[test]
    fn test_release_debug_info() {
        let data = r#"
        [profile.release]
        overflow-checks = true
        debug = 1
        debug-assertions = false
        panic = "abort"
        lto = true
        "#;
        let metadata: Metadata = toml::from_str(data).unwrap();
        let profile = metadata.profile.unwrap().release.unwrap();
        assert!(profile.debug_enabled());
    }
}
