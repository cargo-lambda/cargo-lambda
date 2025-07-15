pub use cargo_metadata::{
    Metadata as CargoMetadata, Package as CargoPackage, Target as CargoTarget,
};
use cargo_options::CommonOptions;
use miette::Result;
use serde::{Deserialize, Serialize, ser::SerializeStruct};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    fs::{metadata, read_to_string},
    path::{Path, PathBuf},
};
use tracing::{Level, enabled, trace};

use crate::error::MetadataError;

pub mod build;
use build::Build;

pub mod deploy;
use deploy::Deploy;

pub mod profile;
use profile::CargoProfile;

pub mod watch;
use watch::Watch;
const STRIP_CONFIG: &str = "profile.release.strip=\"symbols\"";
const LTO_CONFIG: &str = "profile.release.lto=\"thin\"";
const CODEGEN_CONFIG: &str = "profile.release.codegen-units=1";
const PANIC_CONFIG: &str = "profile.release.panic=\"abort\"";

#[derive(Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Metadata {
    #[serde(default)]
    pub lambda: LambdaMetadata,
    #[serde(default)]
    profile: Option<CargoProfile>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct LambdaMetadata {
    #[serde(flatten)]
    pub package: PackageMetadata,
    #[serde(default)]
    pub bin: HashMap<String, PackageMetadata>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct PackageMetadata {
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub deploy: Option<Deploy>,
    #[serde(default)]
    pub build: Option<Build>,
    #[serde(default)]
    pub watch: Option<Watch>,
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

pub fn selected_bin_filter(selected_bins: Vec<String>) -> Box<dyn Fn(&CargoTarget) -> bool> {
    let bins: HashSet<String> = selected_bins.into_iter().collect();
    Box::new(move |t: &CargoTarget| kind_bin_filter(t) && bins.contains(&t.name))
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
pub fn target_dir_from_metadata(metadata: &CargoMetadata) -> Result<PathBuf> {
    Ok(metadata.target_directory.clone().into_std_path_buf())
}

/// Attempt to read the release profile section in the Cargo manifest.
/// Cargo metadata doesn't expose profile information, so we try
/// to read it from the Cargo.toml file directly.
pub fn cargo_release_profile_config<'a>(
    metadata: &CargoMetadata,
) -> Result<HashSet<&'a str>, MetadataError> {
    let path = metadata.workspace_root.join("Cargo.toml");
    let file =
        read_to_string(&path).map_err(|e| MetadataError::InvalidManifestFile(path.into(), e))?;

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

/// Load the main binary in the project.
/// It returns an error if the project includes from than one binary.
/// Use this function when the user didn't provide any funcion name
/// assuming that there is only one binary in the project
pub fn main_binary_from_metadata(metadata: &CargoMetadata) -> Result<String, MetadataError> {
    let targets = binary_targets_from_metadata(metadata, false);
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
            .ok_or(MetadataError::MissingBinaryInProject)
    }
}

fn is_project_metadata_ok(path: &Path) -> bool {
    path.is_dir() && metadata(path).is_ok()
}

pub(crate) fn serialize_common_options<S>(
    state: &mut <S as serde::Serializer>::SerializeStruct,
    opts: &CommonOptions,
) -> Result<(), S::Error>
where
    S: serde::Serializer,
{
    if opts.quiet {
        state.serialize_field("quiet", &true)?;
    }
    if let Some(jobs) = opts.jobs {
        state.serialize_field("jobs", &jobs)?;
    }
    if opts.keep_going {
        state.serialize_field("keep_going", &true)?;
    }
    if let Some(profile) = &opts.profile {
        state.serialize_field("profile", profile)?;
    }
    if !opts.features.is_empty() {
        state.serialize_field("features", &opts.features)?;
    }
    if opts.all_features {
        state.serialize_field("all_features", &true)?;
    }
    if opts.no_default_features {
        state.serialize_field("no_default_features", &true)?;
    }
    if !opts.target.is_empty() {
        state.serialize_field("target", &opts.target)?;
    }
    if let Some(target_dir) = &opts.target_dir {
        state.serialize_field("target_dir", target_dir)?;
    }
    if !opts.message_format.is_empty() {
        state.serialize_field("message_format", &opts.message_format)?;
    }
    if opts.verbose > 0 {
        state.serialize_field("verbose", &opts.verbose)?;
    }
    if let Some(color) = &opts.color {
        state.serialize_field("color", color)?;
    }
    if opts.frozen {
        state.serialize_field("frozen", &true)?;
    }
    if opts.locked {
        state.serialize_field("locked", &true)?;
    }
    if opts.offline {
        state.serialize_field("offline", &true)?;
    }
    if !opts.config.is_empty() {
        state.serialize_field("config", &opts.config)?;
    }
    if !opts.unstable_flags.is_empty() {
        state.serialize_field("unstable_flags", &opts.unstable_flags)?;
    }
    if let Some(timings) = &opts.timings {
        state.serialize_field("timings", timings)?;
    }

    Ok(())
}

pub(crate) fn count_common_options(opts: &CommonOptions) -> usize {
    opts.quiet as usize
        + opts.jobs.is_some() as usize
        + opts.keep_going as usize
        + opts.profile.is_some() as usize
        + !opts.features.is_empty() as usize
        + opts.all_features as usize
        + opts.no_default_features as usize
        + !opts.target.is_empty() as usize
        + opts.target_dir.is_some() as usize
        + !opts.message_format.is_empty() as usize
        + (opts.verbose > 0) as usize
        + opts.color.is_some() as usize
        + opts.frozen as usize
        + opts.locked as usize
        + opts.offline as usize
        + !opts.config.is_empty() as usize
        + !opts.unstable_flags.is_empty() as usize
        + opts.timings.is_some() as usize
}

pub(crate) fn deserialize_vec_or_map<'de, D>(
    deserializer: D,
) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;

    match value {
        Value::Array(arr) => {
            let el = arr
                .into_iter()
                .map(|v| v.as_str().map(String::from))
                .collect::<Option<Vec<_>>>();
            Ok(el)
        }
        Value::Object(map) => {
            let el = map
                .into_iter()
                .map(|(k, v)| format!("{}={}", k, v.as_str().unwrap_or("")))
                .collect();
            Ok(Some(el))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::fixture_metadata;

    use super::*;

    #[test]
    fn test_binary_packages() {
        let bins = binary_targets(fixture_metadata("single-binary-package"), false).unwrap();
        assert_eq!(1, bins.len());
        assert!(bins.contains("basic-lambda"));
    }

    #[test]
    fn test_binary_packages_with_mutiple_bin_entries() {
        let bins = binary_targets(fixture_metadata("multi-binary-package"), false).unwrap();
        assert_eq!(5, bins.len());
        assert!(bins.contains("delete-product"));
        assert!(bins.contains("get-product"));
        assert!(bins.contains("get-products"));
        assert!(bins.contains("put-product"));
        assert!(bins.contains("dynamodb-streams"));
    }

    #[test]
    fn test_binary_packages_with_workspace() {
        let bins = binary_targets(fixture_metadata("workspace-package"), false).unwrap();
        assert_eq!(3, bins.len());
        assert!(bins.contains("basic-lambda-1"));
        assert!(bins.contains("basic-lambda-2"));
        assert!(bins.contains("crate-3"));
    }

    #[test]
    fn test_binary_packages_with_mixed_workspace() {
        let bins = binary_targets(fixture_metadata("mixed-workspace-package"), false).unwrap();
        assert_eq!(1, bins.len());
        assert!(bins.contains("function-crate"), "{bins:?}");
    }

    #[test]
    fn test_binary_packages_with_missing_binary_info() {
        let err = binary_targets(fixture_metadata("missing-binary-package"), false).unwrap_err();
        assert!(
            err.to_string()
                .contains("a [lib] section, or [[bin]] section must be present")
        );
    }

    #[test]
    fn test_main_binary_with_package_name() {
        let manifest_path = fixture_metadata("single-binary-package");
        let metadata = load_metadata(manifest_path).unwrap();
        let name = main_binary_from_metadata(&metadata).unwrap();
        assert_eq!("basic-lambda", name);
    }

    #[test]
    fn test_main_binary_with_binary_name() {
        let manifest_path = fixture_metadata("single-binary-different-name");
        let metadata = load_metadata(manifest_path).unwrap();
        let name = main_binary_from_metadata(&metadata).unwrap();
        assert_eq!("basic-lambda-binary", name);
    }

    #[test]
    fn test_main_binary_multi_binaries() {
        let manifest_path = fixture_metadata("multi-binary-package");
        let metadata = load_metadata(manifest_path).unwrap();
        let err = main_binary_from_metadata(&metadata).unwrap_err();
        assert_eq!(
            "there are more than one binary in the project, please specify a binary name with --binary-name or --binary-path. This is the list of binaries I found: delete-product, dynamodb-streams, get-product, get-products, put-product",
            err.to_string()
        );
    }

    #[test]
    fn test_select_binary() {
        let manifest_path = fixture_metadata("multi-binary-package");
        let metadata = load_metadata(manifest_path).unwrap();

        let package_filter: Option<fn(&&CargoPackage) -> bool> = None;

        let bin = "delete-product".to_string();
        let binary_filter = selected_bin_filter(vec![bin.clone()]);

        let binaries =
            filter_binary_targets_from_metadata(&metadata, binary_filter, package_filter);

        assert_eq!(1, binaries.len());
        assert!(binaries.contains(&bin));
    }

    #[test]
    fn test_example_packages() {
        let bins = binary_targets(fixture_metadata("examples-package"), true).unwrap();
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
    fn test_release_config_with_workspace() {
        let metadata = load_metadata(fixture_metadata("workspace-package")).unwrap();
        let config = cargo_release_profile_config(&metadata).unwrap();
        assert!(config.contains(STRIP_CONFIG));
        assert!(!config.contains(LTO_CONFIG));
    }
}
