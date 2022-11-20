use std::num::ParseIntError;

use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Diagnostic, Error)]
pub enum MetadataError {
    #[error("invalid memory value `{0}`")]
    InvalidMemory(i32),
    #[error("invalid lambda metadata in Cargo.toml file: {0}")]
    InvalidCargoMetadata(#[from] serde_json::Error),
    #[error("invalid timeout value")]
    InvalidTimeout(#[from] ParseIntError),
    #[error("invalid tracing option `{0}`")]
    InvalidTracing(String),
    #[error("there are more than one package in the project, you must specify a function name")]
    MultiplePackagesInProject,
    #[error("there are no packages in this project")]
    MissingPackageInProject,
}
