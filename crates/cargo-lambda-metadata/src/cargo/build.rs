use cargo_metadata::Metadata as CargoMetadata;
use serde::Deserialize;

use crate::{
    cargo::{LambdaMetadata, Metadata},
    error::MetadataError,
};

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

pub fn merge_build_config(base: &mut BuildConfig, package_build: &BuildConfig) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cargo::{tests::fixture, *};

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
}
