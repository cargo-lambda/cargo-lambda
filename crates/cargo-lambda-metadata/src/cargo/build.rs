use std::path::PathBuf;

use cargo_options::Build as CargoBuild;
use clap::{Args, ValueHint};
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};

use crate::cargo::{count_common_options, serialize_common_options};

#[derive(Args, Clone, Debug, Default, Deserialize)]
#[command(
    name = "build",
    after_help = "Full command documentation: https://www.cargo-lambda.info/commands/build.html"
)]
pub struct Build {
    /// The format to produce the compile Lambda into, acceptable values are [Binary, Zip]
    #[arg(short, long)]
    #[serde(default)]
    pub output_format: Option<OutputFormat>,

    /// Directory where the final lambda binaries will be located
    #[arg(short, long, value_hint = ValueHint::DirPath)]
    #[serde(default)]
    pub lambda_dir: Option<PathBuf>,

    /// Shortcut for --target aarch64-unknown-linux-gnu
    #[arg(long)]
    #[serde(default)]
    pub arm64: bool,

    /// Shortcut for --target x86_64-unknown-linux-gnu
    #[arg(long)]
    #[serde(default)]
    pub x86_64: bool,

    /// Whether the code that you're building is a Lambda Extension
    #[arg(long)]
    #[serde(default)]
    pub extension: bool,

    /// Whether an extension is internal or external
    #[arg(long, requires = "extension")]
    #[serde(default)]
    pub internal: bool,

    /// Put a bootstrap file in the root of the lambda directory.
    /// Use the name of the compiled binary to choose which file to move.
    #[arg(long)]
    #[serde(default)]
    pub flatten: Option<String>,

    /// Whether to skip the target check
    #[arg(long)]
    #[serde(default)]
    pub skip_target_check: bool,

    /// Backend to build the project with
    #[arg(short, long, env = "CARGO_LAMBDA_COMPILER")]
    #[serde(default)]
    pub compiler: Option<CompilerOptions>,

    /// Disable all default release optimizations
    #[arg(long)]
    #[serde(default)]
    pub disable_optimizations: bool,

    /// Option to add one or more files and directories to include in the output ZIP file (only works with --output-format=zip).
    #[arg(short, long)]
    #[serde(default)]
    pub include: Option<Vec<String>>,

    #[command(flatten)]
    #[serde(default, flatten)]
    pub cargo_opts: CargoBuild,
}

#[derive(Clone, Debug, Default, Deserialize, Display, EnumString, PartialEq, Serialize)]
#[strum(ascii_case_insensitive)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    #[default]
    Binary,
    Zip,
}

#[derive(Clone, Debug, Default, Deserialize, Display, Eq, PartialEq, Serialize)]
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct CargoCompilerOptions {
    #[serde(default)]
    pub subcommand: Option<Vec<String>>,
    #[serde(default)]
    pub extra_args: Option<Vec<String>>,
}

impl Build {
    pub fn manifest_path(&self) -> PathBuf {
        self.cargo_opts
            .manifest_path
            .clone()
            .unwrap_or_else(|| "Cargo.toml".into())
    }

    pub fn output_format(&self) -> &OutputFormat {
        self.output_format.as_ref().unwrap_or(&OutputFormat::Binary)
    }

    /// Returns the package name if there is only one package in the list of `packages`,
    /// otherwise None.
    pub fn pkg_name(&self) -> Option<String> {
        if self.cargo_opts.packages.len() > 1 {
            return None;
        }
        self.cargo_opts.packages.first().map(|s| s.to_string())
    }

    pub fn bin_name(&self) -> Option<String> {
        if self.cargo_opts.bin.len() > 1 {
            return None;
        }
        self.cargo_opts.bin.first().map(|s| s.to_string())
    }
}

impl Serialize for Build {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        // Count how many fields we'll actually serialize
        let field_count = self.output_format.is_some() as usize
            + self.lambda_dir.is_some() as usize
            + self.flatten.is_some() as usize
            + self.compiler.is_some() as usize
            + self.include.is_some() as usize
            + self.arm64 as usize
            + self.x86_64 as usize
            + self.extension as usize
            + self.internal as usize
            + self.skip_target_check as usize
            + self.disable_optimizations as usize
            + self.cargo_opts.manifest_path.is_some() as usize
            + self.cargo_opts.bins as usize
            + !self.cargo_opts.bin.is_empty() as usize
            + self.cargo_opts.examples as usize
            + !self.cargo_opts.example.is_empty() as usize
            + self.cargo_opts.all_targets as usize
            + !self.cargo_opts.packages.is_empty() as usize
            + self.cargo_opts.workspace as usize
            + !self.cargo_opts.exclude.is_empty() as usize
            + self.cargo_opts.tests as usize
            + !self.cargo_opts.test.is_empty() as usize
            + self.cargo_opts.benches as usize
            + !self.cargo_opts.bench.is_empty() as usize
            + count_common_options(&self.cargo_opts.common);

        let mut state = serializer.serialize_struct("Build", field_count)?;

        // Optional fields
        if let Some(ref output_format) = self.output_format {
            state.serialize_field("output_format", output_format)?;
        }
        if let Some(ref lambda_dir) = self.lambda_dir {
            state.serialize_field("lambda_dir", lambda_dir)?;
        }
        if let Some(ref flatten) = self.flatten {
            state.serialize_field("flatten", flatten)?;
        }
        if let Some(ref compiler) = self.compiler {
            state.serialize_field("compiler", compiler)?;
        }
        if let Some(ref include) = self.include {
            state.serialize_field("include", include)?;
        }

        // Boolean fields
        if self.arm64 {
            state.serialize_field("arm64", &true)?;
        }
        if self.x86_64 {
            state.serialize_field("x86_64", &true)?;
        }
        if self.extension {
            state.serialize_field("extension", &true)?;
        }
        if self.internal {
            state.serialize_field("internal", &true)?;
        }
        if self.skip_target_check {
            state.serialize_field("skip_target_check", &true)?;
        }
        if self.disable_optimizations {
            state.serialize_field("disable_optimizations", &true)?;
        }

        // Cargo opts fields
        if let Some(ref manifest_path) = self.cargo_opts.manifest_path {
            state.serialize_field("manifest_path", manifest_path)?;
        }
        if self.cargo_opts.release {
            state.serialize_field("release", &true)?;
        }
        if self.cargo_opts.bins {
            state.serialize_field("bins", &true)?;
        }
        if !self.cargo_opts.bin.is_empty() {
            state.serialize_field("bin", &self.cargo_opts.bin)?;
        }
        if self.cargo_opts.examples {
            state.serialize_field("examples", &true)?;
        }
        if !self.cargo_opts.example.is_empty() {
            state.serialize_field("example", &self.cargo_opts.example)?;
        }
        if self.cargo_opts.all_targets {
            state.serialize_field("all_targets", &true)?;
        }
        if !self.cargo_opts.packages.is_empty() {
            state.serialize_field("packages", &self.cargo_opts.packages)?;
        }
        if self.cargo_opts.workspace {
            state.serialize_field("workspace", &true)?;
        }
        if !self.cargo_opts.exclude.is_empty() {
            state.serialize_field("exclude", &self.cargo_opts.exclude)?;
        }
        if self.cargo_opts.tests {
            state.serialize_field("tests", &true)?;
        }
        if !self.cargo_opts.test.is_empty() {
            state.serialize_field("test", &self.cargo_opts.test)?;
        }
        if self.cargo_opts.benches {
            state.serialize_field("benches", &true)?;
        }
        if !self.cargo_opts.bench.is_empty() {
            state.serialize_field("bench", &self.cargo_opts.bench)?;
        }
        serialize_common_options::<S>(&mut state, &self.cargo_opts.common)?;

        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cargo_options::CommonOptions;
    use serde_json::json;

    #[test]
    fn test_serialize_minimal_build() {
        let build = Build::default();
        let serialized = serde_json::to_value(&build).unwrap();

        assert_eq!(serialized, json!({}));
    }

    #[test]
    fn test_serialize_with_optional_fields() {
        let build = Build {
            lambda_dir: Some(PathBuf::from("/tmp/lambda")),
            compiler: Some(CompilerOptions::Cross),
            include: Some(vec!["file1.txt".to_string(), "file2.txt".to_string()]),
            ..Default::default()
        };

        let serialized = serde_json::to_value(&build).unwrap();

        assert_eq!(
            serialized,
            json!({
                "lambda_dir": "/tmp/lambda",
                "compiler": { "type": "cross" },
                "include": ["file1.txt", "file2.txt"]
            })
        );
    }

    #[test]
    fn test_serialize_with_boolean_fields() {
        let build = Build {
            arm64: true,
            extension: true,
            skip_target_check: true,
            ..Default::default()
        };

        let serialized = serde_json::to_value(&build).unwrap();

        assert_eq!(
            serialized,
            json!({
                "arm64": true,
                "extension": true,
                "skip_target_check": true
            })
        );
    }

    #[test]
    fn test_serialize_with_cargo_opts() {
        let build = Build {
            cargo_opts: CargoBuild {
                common: CommonOptions {
                    target: vec!["x86_64-unknown-linux-gnu".to_string()],
                    features: vec!["feature1".to_string(), "feature2".to_string()],
                    all_features: true,
                    profile: Some("release".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let serialized = serde_json::to_value(&build).unwrap();

        assert_eq!(
            serialized,
            json!({
                "target": ["x86_64-unknown-linux-gnu"],
                "features": ["feature1", "feature2"],
                "all_features": true,
                "profile": "release"
            })
        );
    }

    #[test]
    fn test_serialize_complete_build() {
        let build = Build {
            // Main struct fields
            output_format: Some(OutputFormat::Zip),
            lambda_dir: Some(PathBuf::from("/tmp/lambda")),
            arm64: true,
            extension: true,
            compiler: Some(CompilerOptions::CargoZigbuild),
            include: Some(vec!["include1".to_string()]),

            // Cargo opts
            cargo_opts: CargoBuild {
                common: CommonOptions {
                    target: vec!["x86_64-unknown-linux-gnu".to_string()],
                    features: vec!["feature1".to_string()],
                    all_features: true,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let serialized = serde_json::to_value(&build).unwrap();

        assert_eq!(
            serialized,
            json!({
                "output_format": "zip",
                "lambda_dir": "/tmp/lambda",
                "arm64": true,
                "extension": true,
                "compiler": { "type": "cargo_zigbuild" },
                "include": ["include1"],
                "target": ["x86_64-unknown-linux-gnu"],
                "features": ["feature1"],
                "all_features": true
            })
        );
    }
}
