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
}
