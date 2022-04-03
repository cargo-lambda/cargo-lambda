use cargo_metadata::Metadata as CargoMetadata;
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
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
pub fn binary_targets(manifest_path: PathBuf) -> Result<HashSet<String>> {
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

/// Return the lambda metadata section for a function
/// See the documentation to learn about how we use this metadata:
/// https://github.com/calavera/cargo-lambda#start---environment-variables
pub fn function_metadata(manifest_path: PathBuf, name: &str) -> Result<Option<PackageMetadata>> {
    let metadata = match package_metadata(manifest_path, name)? {
        None => return Ok(None),
        Some(m) => m,
    };

    let mut env = HashMap::new();
    env.extend(metadata.lambda.env);
    if let Some(bin) = metadata.lambda.bin.get(name) {
        env.extend(bin.env.clone());
    }

    Ok(Some(PackageMetadata { env }))
}

/// Create metadata about the root package in the Cargo manifest, without any dependencies.
fn load_metadata(manifest_path: PathBuf) -> Result<CargoMetadata> {
    let mut metadata_cmd = cargo_metadata::MetadataCommand::new();
    metadata_cmd.no_deps();
    metadata_cmd.manifest_path(manifest_path);
    metadata_cmd.exec().into_diagnostic()
}

// Find the package in the Cargo manifest that contains a binary `name`.
fn package_metadata(manifest_path: PathBuf, name: &str) -> Result<Option<Metadata>> {
    let metadata = load_metadata(manifest_path)?;
    for pkg in metadata.packages {
        for target in &pkg.targets {
            if target.name == name && target.kind.iter().any(|kind| kind == "bin") {
                let metadata: Metadata = serde_json::from_value(pkg.metadata.clone())
                    .into_diagnostic()
                    .wrap_err("invalid lambda metadata in Cargo.toml file")?;
                return Ok(Some(metadata));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_binary_packages_with_missing_binary_info() {
        let err = binary_targets(fixture("missing-binary-package")).unwrap_err();
        assert!(err
            .to_string()
            .contains("a [lib] section, or [[bin]] section must be present"));
    }

    #[test]
    fn test_metadata_packages() {
        let meta = function_metadata(fixture("single-binary-package"), "basic-lambda")
            .unwrap()
            .unwrap();

        assert_eq!("BAR", meta.env["FOO"]);
    }

    #[test]
    fn test_metadata_multi_packages() {
        let meta = function_metadata(fixture("multi-binary-package"), "get-product")
            .unwrap()
            .unwrap();

        assert_eq!("BAR", meta.env["FOO"]);

        let meta = function_metadata(fixture("multi-binary-package"), "delete-product")
            .unwrap()
            .unwrap();

        assert_eq!("QUX", meta.env["BAZ"]);
    }

    #[test]
    fn test_metadata_workspace_packages() {
        let meta = function_metadata(fixture("workspace-package"), "basic-lambda-1")
            .unwrap()
            .unwrap();

        assert_eq!("BAR", meta.env["FOO"]);

        let meta = function_metadata(fixture("workspace-package"), "basic-lambda-2")
            .unwrap()
            .unwrap();

        assert_eq!("BAR", meta.env["FOO"]);
    }
}
