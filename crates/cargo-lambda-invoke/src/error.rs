use serde::Deserialize;

use miette::Diagnostic;
use thiserror::Error;

#[derive(Debug, Diagnostic, Error)]
pub enum InvokeError {
    #[error("failed to download example data from {0}:\n {1:?}")]
    ExampleDownloadFailed(String, reqwest::Response),
    #[error(
        "invalid function name, it must match the name you used to create the function remotely"
    )]
    InvalidFunctionName,
    #[error(
        "no data payload provided, use one of the data flags: `--data-file`, `--data-ascii`, `--data-example`"
    )]
    MissingPayload,
    #[error("invalid error payload {0}")]
    InvalidErrorPayload(#[from] serde_json::Error),
}

#[derive(Debug, Deserialize)]
pub struct RemoteInvokeError {
    #[serde(rename = "errorType", alias = "title")]
    code: String,
    #[serde(rename = "errorMessage", alias = "detail")]
    message: String,
}

impl std::fmt::Display for RemoteInvokeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for RemoteInvokeError {}

impl Diagnostic for RemoteInvokeError {
    fn code(&self) -> Option<Box<dyn std::fmt::Display>> {
        let c = self.code.trim_start_matches('&');
        Some(Box::new(c.to_string()))
    }
}

impl TryFrom<&str> for RemoteInvokeError {
    type Error = InvokeError;
    fn try_from(vec: &str) -> Result<Self, Self::Error> {
        let e = serde_json::from_str(vec)?;
        Ok(e)
    }
}
