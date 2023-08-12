use std::path::PathBuf;

use cargo_lambda_metadata::error::MetadataError;
use miette::Diagnostic;
use object::Architecture;
use thiserror::Error;

#[derive(Debug, Diagnostic, Error)]
pub(crate) enum BuildError {
    #[error(
        "invalid options: --arm64, --x86-64, and --target cannot be specified at the same time"
    )]
    #[diagnostic()]
    InvalidTargetOptions,
    #[error("invalid options: --compiler=cargo is only allowed on Linux")]
    #[diagnostic()]
    InvalidCompilerOption,
    #[error("install Zig and run cargo-lambda again")]
    #[diagnostic()]
    ZigMissing,
    #[error("binary target is missing from this project: {0}")]
    #[diagnostic()]
    FunctionBinaryMissing(String),
    #[error("binary file for {0} not found, use `cargo lambda {1}` to create it")]
    #[diagnostic()]
    BinaryMissing(String, String),
    #[error("invalid binary architecture: {0:?}")]
    #[diagnostic()]
    InvalidBinaryArchitecture(Architecture),
    #[error("invalid or unsupported target for AWS Lambda: {0}")]
    #[diagnostic()]
    UnsupportedTarget(String),
    #[error("invalid unix file name: {0}")]
    #[diagnostic()]
    InvalidUnixFileName(PathBuf),
    #[error(transparent)]
    #[diagnostic()]
    FailedBuildCommand(#[from] std::io::Error),
    #[error(transparent)]
    #[diagnostic()]
    MetadataError(#[from] MetadataError),
}
