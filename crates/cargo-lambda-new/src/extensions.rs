use clap::Args;
use liquid::{Object, model::Value};
use miette::Result;

pub(crate) const DEFAULT_TEMPLATE_URL: &str =
    "https://github.com/cargo-lambda/new-extensions-template/archive/refs/heads/main.zip";

#[derive(Args, Clone, Debug, Default)]
#[group(requires = "extension", id = "extension-opts")]
pub(crate) struct Options {
    /// Whether the extension includes a Logs processor
    #[arg(long, conflicts_with = "telemetry")]
    logs: bool,
    /// Whether the extension includes a Telemetry processor
    #[arg(long, conflicts_with = "logs")]
    telemetry: bool,
    /// Whether the extension includes an Events processor
    #[arg(long)]
    events: bool,
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
            "telemetry": self.telemetry,
            "events": self.add_events_extension(),
            "lambda_extension_version": lv,
        }))
    }

    fn add_events_extension(&self) -> bool {
        self.events || (!self.logs && !self.telemetry)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_add_events_extension() {
        let cases = [
            (Options::default(), true),
            (
                Options {
                    logs: true,
                    events: true,
                    ..Default::default()
                },
                true,
            ),
            (
                Options {
                    telemetry: true,
                    events: true,
                    ..Default::default()
                },
                true,
            ),
            (
                Options {
                    logs: true,
                    ..Default::default()
                },
                false,
            ),
            (
                Options {
                    telemetry: true,
                    ..Default::default()
                },
                false,
            ),
        ];

        for (opt, exp) in cases {
            assert_eq!(exp, opt.add_events_extension(), "options: {opt:?}");
        }
    }
}
