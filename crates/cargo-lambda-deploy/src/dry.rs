use cargo_lambda_build::{BinaryArchive, BinaryModifiedAt};
use cargo_lambda_metadata::cargo::deploy::{Deploy, FunctionDeployConfig};
use miette::Result;
use serde::Serialize;
use std::{collections::HashMap, fmt::Display, path::PathBuf};

use crate::binary_name_or_default;

#[derive(PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DeployKind {
    Function,
    Extension,
}

impl Display for DeployKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeployKind::Function => write!(f, "function"),
            DeployKind::Extension => write!(f, "extension"),
        }
    }
}

#[derive(Serialize)]
pub(crate) struct DeployOutput {
    kind: DeployKind,
    name: String,
    path: PathBuf,
    arch: String,
    files: Vec<String>,
    runtimes: Vec<String>,
    tags: Option<String>,
    bucket: Option<String>,
    config: FunctionDeployConfig,
    binary_modified_at: BinaryModifiedAt,
}

impl Display for DeployOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "🔍 deployment for {} `{}`:", self.kind, self.name)?;
        writeln!(f, "🏠 zip file located at {}", self.path.display())?;
        writeln!(
            f,
            "🛠️  binary last compiled {}",
            self.binary_modified_at.humanize()
        )?;
        writeln!(f, "🔗 architecture {}", self.arch)?;

        if let Some(tags) = &self.tags {
            writeln!(f, "🏷️  tagged with {}", tags.replace(',', ", "))?;
        }

        if let Some(bucket) = &self.bucket {
            writeln!(f, "🪣 stored on S3 bucket `{}`", bucket)?;
        }

        if !self.runtimes.is_empty() {
            write!(f, "👟 compatible with {}", self.runtimes.join(", "))?;
        }

        if !self.files.is_empty() {
            writeln!(f, "🗃️  files included in the zip file:")?;
            for file in &self.files {
                writeln!(f, "  - {}", file)?;
            }
        }

        if self.kind == DeployKind::Function {
            writeln!(f, "🍿 function configuration:")?;
            writeln!(f, "  - timeout: {:?}", self.config.timeout)?;
            writeln!(f, "  - memory: {:?}", self.config.memory)?;
            writeln!(
                f,
                "  - enable_function_url: {}",
                self.config.enable_function_url
            )?;
            writeln!(
                f,
                "  - disable_function_url: {}",
                self.config.disable_function_url
            )?;
            writeln!(f, "  - tracing: {:?}", self.config.tracing)?;
            writeln!(f, "  - role: {:?}", self.config.role)?;
            writeln!(f, "  - layer: {:?}", self.config.layer)?;
            writeln!(f, "  - vpc: {:?}", self.config.vpc)?;
            writeln!(f, "  - runtime: {:?}", self.config.runtime())?;
            if let Some(env_options) = &self.config.env_options {
                let env = env_options
                    .lambda_environment(&HashMap::new())
                    .unwrap_or_default();
                writeln!(f, "  - env_options: {:?}", env)?;
            }
        }

        Ok(())
    }
}

impl DeployOutput {
    pub(crate) fn new(config: &Deploy, name: &str, archive: &BinaryArchive) -> Result<Self> {
        let (kind, name, runtimes) = if config.extension {
            (
                DeployKind::Extension,
                name.to_owned(),
                config.compatible_runtimes(),
            )
        } else {
            let binary_name = binary_name_or_default(config, name);
            (DeployKind::Function, binary_name, vec![])
        };

        Ok(DeployOutput {
            kind,
            name,
            runtimes,
            path: archive.path.clone(),
            arch: archive.architecture.clone(),
            bucket: config.s3_bucket.clone(),
            tags: config.s3_tags(),
            config: config.function_config.clone(),
            files: archive.list()?,
            binary_modified_at: archive.binary_modified_at.clone(),
        })
    }
}
