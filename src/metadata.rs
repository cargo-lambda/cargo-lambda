use cargo_metadata::Package;
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Deserialize;
use std::{collections::HashMap, path::PathBuf};

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

pub(crate) fn binary_packages(manifest_path: PathBuf) -> Result<HashMap<String, Package>> {
    let mut metadata_cmd = cargo_metadata::MetadataCommand::new();
    metadata_cmd.no_deps();
    metadata_cmd.manifest_path(manifest_path);
    let metadata = metadata_cmd.exec().into_diagnostic()?;

    let mut binaries = HashMap::new();
    for pkg in metadata.packages {
        let mut bin_name = None;
        for target in &pkg.targets {
            if target.kind.iter().any(|s| s == "bin") {
                bin_name = Some(target.name.clone());
                break;
            }
        }
        if let Some(name) = bin_name {
            binaries.insert(name, pkg);
        }
    }

    Ok(binaries)
}

pub(crate) fn function_metadata(
    manifest_path: PathBuf,
    name: &str,
) -> Result<Option<PackageMetadata>> {
    let binaries = binary_packages(manifest_path)?;
    let package = match binaries.get(name) {
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
