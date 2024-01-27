use std::{num::ParseIntError, path::PathBuf};

use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Diagnostic, Error)]
pub enum MetadataError {
    #[error("invalid memory value `{0}`")]
    #[diagnostic()]
    InvalidMemory(i32),
    #[error("invalid lambda metadata in Cargo.toml file: {0}")]
    #[diagnostic()]
    InvalidCargoMetadata(#[from] serde_json::Error),
    #[error("invalid timeout value")]
    #[diagnostic()]
    InvalidTimeout(#[from] ParseIntError),
    #[error("invalid tracing option `{0}`")]
    #[diagnostic()]
    InvalidTracing(String),
    #[error("there are more than one binary in the project, please specify a binary name with --binary-name or --binary-path. This is the list of binaries I found: {0}")]
    #[diagnostic()]
    MultipleBinariesInProject(String),
    #[error("there are no binaries in this project")]
    #[diagnostic()]
    MissingBinaryInProject,
    #[error("invalid environment variable `{0}`")]
    #[diagnostic()]
    InvalidEnvVar(String),
    #[error("invalid environment file `{0}`: {1}")]
    #[diagnostic()]
    InvalidEnvFile(PathBuf, std::io::Error),
    #[error(transparent)]
    #[diagnostic()]
    FailedCmdExecution(#[from] cargo_metadata::Error),
    #[error("invalid manifest file `{0}`: {1}")]
    #[diagnostic()]
    InvalidManifestFile(PathBuf, std::io::Error),
    #[error(transparent)]
    #[diagnostic()]
    InvalidTomlManifest(toml::de::Error),
}
