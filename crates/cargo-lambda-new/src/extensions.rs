use clap::Args;
use liquid::{model::Value, Object};
use miette::Result;

pub(crate) const DEFAULT_TEMPLATE_URL: &str =
    "https://github.com/cargo-lambda/default-extension-template/archive/refs/heads/main.zip";

#[derive(Args, Clone, Debug, Default)]
pub(crate) struct Options {
    /// Whether the extension is going to be a Logs extension or not
    #[clap(long)]
    logs: bool,
}

impl Options {
    pub(crate) fn validate_options(&mut self) -> Result<()> {
        Ok(())
    }

    pub(crate) fn variables(&self) -> Result<Object> {
        let lv = option_env!("CARGO_LAMBDA_EXTENSION_VERSION")
            .map(|v| Value::scalar(v.to_string()))
            .unwrap_or(Value::Nil);

        Ok(liquid::object!({
            "logs": self.logs,
            "lambda_extension_version": lv,
        }))
    }
}
