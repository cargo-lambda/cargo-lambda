use cargo_metadata::Metadata as CargoMetadata;
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

#[derive(Default, Deserialize)]
#[non_exhaustive]
pub(crate) struct Metadata {
    #[serde(default)]
    pub lambda: LambdaMetadata,
}

#[derive(Clone, Default, Deserialize)]
#[non_exhaustive]
pub(crate) struct LambdaMetadata {
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub bin: HashMap<String, PackageMetadata>,
}

#[derive(Clone, Default, Deserialize)]
#[non_exhaustive]
pub(crate) struct PackageMetadata {
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// Extract all the binary names from a Cargo.toml file
pub(crate) fn binary_packages(manifest_path: PathBuf) -> Result<HashSet<String>> {
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
pub(crate) fn function_metadata(
    manifest_path: PathBuf,
    name: &str,
) -> Result<Option<PackageMetadata>> {
    let metadata = load_metadata(manifest_path)?;

    let package = metadata.packages.iter().find(|p| {
        p.targets
            .iter()
            .any(|target| target.kind.iter().any(|k| k == "bin") && target.name == name)
    });

    let package = match package {
        None => return Ok(None),
        Some(p) => p,
    };

    let metadata: Metadata = serde_json::from_value(package.metadata.clone())
        .into_diagnostic()
        .wrap_err("invalid lambda metadata in Cargo.toml file")?;

    let mut env = HashMap::new();
    env.extend(metadata.lambda.env);
    if let Some(bin) = metadata.lambda.bin.get(name) {
        env.extend(bin.env.clone());
    }

    Ok(Some(PackageMetadata { env }))
}

fn load_metadata(manifest_path: PathBuf) -> Result<CargoMetadata> {
    let mut metadata_cmd = cargo_metadata::MetadataCommand::new();
    metadata_cmd.no_deps();
    metadata_cmd.manifest_path(manifest_path);
    metadata_cmd.exec().into_diagnostic()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_binary_packages() {
        let bins =
            binary_packages("test/fixtures/single-binary-package/Cargo.toml".into()).unwrap();
        assert_eq!(1, bins.len());
        assert!(bins.contains("basic-lambda"));
    }

    #[test]
    fn test_binary_packages_with_mutiple_bin_entries() {
        let bins = binary_packages("test/fixtures/multi-binary-package/Cargo.toml".into()).unwrap();
        assert_eq!(5, bins.len());
        assert!(bins.contains("delete-product"));
        assert!(bins.contains("get-product"));
        assert!(bins.contains("get-products"));
        assert!(bins.contains("put-product"));
        assert!(bins.contains("dynamodb-streams"));
    }

    #[test]
    fn test_binary_packages_with_workspace() {
        let bins = binary_packages("test/fixtures/workspace-package/Cargo.toml".into()).unwrap();
        assert_eq!(2, bins.len());
        assert!(bins.contains("basic-lambda-1"));
        assert!(bins.contains("basic-lambda-2"));
    }

    #[test]
    fn test_binary_packages_with_missing_binary_info() {
        let err =
            binary_packages("test/fixtures/missing-binary-package/Cargo.toml".into()).unwrap_err();
        assert!(err
            .to_string()
            .contains("a [lib] section, or [[bin]] section must be present"));
    }

    #[test]
    fn test_metadata_packages() {
        let meta = function_metadata(
            "test/fixtures/single-binary-package/Cargo.toml".into(),
            "basic-lambda",
        )
        .unwrap()
        .unwrap();

        assert_eq!("BAR", meta.env["FOO"]);
    }

    #[test]
    fn test_metadata_multi_packages() {
        let meta = function_metadata(
            "test/fixtures/multi-binary-package/Cargo.toml".into(),
            "get-product",
        )
        .unwrap()
        .unwrap();

        assert_eq!("BAR", meta.env["FOO"]);

        let meta = function_metadata(
            "test/fixtures/multi-binary-package/Cargo.toml".into(),
            "delete-product",
        )
        .unwrap()
        .unwrap();

        assert_eq!("QUX", meta.env["BAZ"]);
    }

    #[test]
    fn test_metadata_workspace_packages() {
        let meta = function_metadata(
            "test/fixtures/workspace-package/Cargo.toml".into(),
            "basic-lambda-1",
        )
        .unwrap()
        .unwrap();

        assert_eq!("BAR", meta.env["FOO"]);

        let meta = function_metadata(
            "test/fixtures/workspace-package/Cargo.toml".into(),
            "basic-lambda-2",
        )
        .unwrap()
        .unwrap();

        assert_eq!("BAR", meta.env["FOO"]);
    }
}
