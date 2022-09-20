use cargo_metadata::{Metadata as CargoMetadata, Package};
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Default, Deserialize)]
#[non_exhaustive]
pub struct Metadata {
    #[serde(default)]
    pub lambda: LambdaMetadata,
}

#[derive(Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct LambdaMetadata {
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub bin: HashMap<String, PackageMetadata>,
}

#[derive(Clone, Default, Deserialize)]
#[non_exhaustive]
pub struct PackageMetadata {
    #[serde(default)]
    pub env: HashMap<String, String>,
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

/// Extract target directory information
///
/// This fetches the target directory from `cargo metadata`, resolving the
/// user and project configuration and the environment variables in the right
/// way.
pub fn target_dir<P: AsRef<Path>>(manifest_path: P) -> Result<PathBuf> {
    let metadata = load_metadata(manifest_path)?;
    Ok(metadata.target_directory.into_std_path_buf())
}

/// Create metadata about the root package in the Cargo manifest, without any dependencies.
fn load_metadata<P: AsRef<Path>>(manifest_path: P) -> Result<CargoMetadata> {
    let mut metadata_cmd = cargo_metadata::MetadataCommand::new();
    metadata_cmd.no_deps();

    // try to split manifest path and assign current_dir to enable parsing a project-specific
    // cargo config
    match (
        manifest_path.as_ref().parent(),
        manifest_path.as_ref().file_name(),
    ) {
        (Some(mut project_dir), Some(manifest_file)) => {
            if project_dir == Path::new("") {
                project_dir = Path::new("./")
            }
            metadata_cmd.current_dir(project_dir);
            metadata_cmd.manifest_path(manifest_file);
        }
        _ => {
            // fall back to using the manifest_path without changing the dir
            // this means there will not be any proejct-specific config parsing
            metadata_cmd.manifest_path(manifest_path.as_ref());
        }
    }

    metadata_cmd.exec().into_diagnostic()
}

/// Create a HashMap of environment varibales from the package and workspace manifest
/// See the documentation to learn about how we use this metadata:
/// https://github.com/cargo-lambda/cargo-lambda#start---environment-variables
pub fn function_metadata<P: AsRef<Path>>(
    manifest_path: P,
    name: Option<&str>,
) -> Result<HashMap<String, String>> {
    let metadata = load_metadata(manifest_path)?;
    let ws_metadata: LambdaMetadata =
        serde_json::from_value(metadata.workspace_metadata).unwrap_or_default();

    let mut env = HashMap::new();
    env.extend(ws_metadata.env);

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
                    .into_diagnostic()
                    .wrap_err("invalid lambda metadata in Cargo.toml file")?;

                env.extend(package_metadata.lambda.env);
                if let Some(res) = package_metadata.lambda.bin.get(name) {
                    env.extend(res.env.clone());
                }
            }
        }
    }

    tracing::debug!(env = ?env, "using environment variables from metadata");
    Ok(env)
}

/// Load the main package in the project.
/// It returns an error if the project includes from than one package.
/// Use this function when the user didn't provide any funcion name
/// assuming that there is only one package in the project
pub fn root_package<P: AsRef<Path>>(manifest_path: P) -> Result<Package> {
    let metadata = load_metadata(manifest_path)?;
    if metadata.packages.len() > 1 {
        Err(miette::miette!(
            "there are more than one package in the project, you must specify a function name"
        ))
    } else if metadata.packages.is_empty() {
        Err(miette::miette!("there are no packages in this project"))
    } else {
        Ok(metadata
            .packages
            .into_iter()
            .next()
            .expect("failed to extract the root package from the metadata"))
    }
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
            function_metadata(fixture("single-binary-package"), Some("basic-lambda")).unwrap();

        assert_eq!(env.get("FOO").unwrap(), "BAR");
    }

    #[test]
    fn test_metadata_multi_packages() {
        let env = function_metadata(fixture("multi-binary-package"), Some("get-product")).unwrap();

        assert_eq!(env.get("FOO").unwrap(), "BAR");

        let env =
            function_metadata(fixture("multi-binary-package"), Some("delete-product")).unwrap();

        assert_eq!(env.get("BAZ").unwrap(), "QUX");
    }

    #[test]
    fn test_metadata_workspace_packages() {
        let env = function_metadata(fixture("workspace-package"), Some("basic-lambda-1")).unwrap();

        assert_eq!(env.get("FOO").unwrap(), "BAR");

        let env = function_metadata(fixture("workspace-package"), Some("basic-lambda-2")).unwrap();

        assert_eq!(env.get("FOO").unwrap(), "BAR");
    }

    #[test]
    fn test_metadata_packages_without_name() {
        let env = function_metadata(fixture("single-binary-package"), None).unwrap();

        assert_eq!(env.get("FOO").unwrap(), "BAR");
    }

    #[test]
    fn test_target_dir_non_set() {
        use std::env;

        // ensure there is no environment variable set
        env::remove_var("CARGO_TARGET_DIR");
        let target_dir = target_dir(fixture("single-binary-package")).unwrap();
        assert!(target_dir.ends_with("test/fixtures/single-binary-package/target"));
    }

    #[test]
    fn test_target_dir_from_project_config() {
        use std::env;

        // ensure there is no environment variable set
        env::remove_var("CARGO_TARGET_DIR");
        let target_dir = target_dir(fixture("target-dir-set-in-project")).unwrap();
        assert!(target_dir.ends_with("project_specific_target"));
    }

    #[test]
    fn test_target_dir_from_env() {
        use std::env;

        // set environment variable
        env::set_var("CARGO_TARGET_DIR", "/tmp/exotic_path");
        let target_dir = target_dir(fixture("single-binary-package")).unwrap();
        assert!(target_dir.ends_with("/tmp/exotic_path"));
    }
}
