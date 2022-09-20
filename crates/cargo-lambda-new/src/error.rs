use std::path::PathBuf;

use cargo_lambda_interactive::error::InquireError;
use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Diagnostic, Error)]
pub(crate) enum CreateError {
    #[error("missing options: --event-type, --http")]
    MissingFunctionOptions,
    #[error("invalid options: --event-type and --http cannot be specified at the same time")]
    InvalidFunctionOptions,
    #[error("unexpected input")]
    UnexpectedInput(#[from] InquireError),
    #[error("invalid file path in template {0:?}")]
    InvalidTemplateEntry(PathBuf),
    #[error("project created in {0}, but the EDITOR variable is missing")]
    InvalidEditor(String),
    #[error("invalid package name: {0}")]
    InvalidPackageName(String),
}
