use cargo_lambda_interactive::{
    error::{CustomUserError, InquireError},
    is_stdin_tty,
    validator::{ErrorMessage, Validation},
    Confirm, Text,
};
use clap::Args;
use liquid::{model::Value, Object};
use miette::Result;

use crate::error::CreateError;

pub(crate) const DEFAULT_TEMPLATE_URL: &str =
    "https://github.com/cargo-lambda/default-template/archive/refs/heads/main.zip";

#[derive(Args, Clone, Debug, Default)]
#[group(multiple = false, conflicts_with_all = ["extension", "extension-opts"], id = "function-opts")]
pub(crate) struct Options {
    /// Whether the function is going to be an HTTP endpoint or not
    #[arg(long)]
    http: bool,

    /// The specific HTTP feature to enable
    #[arg(long, conflicts_with = "http")]
    http_feature: Option<HttpFeature>,

    /// Type of AWS event that this function is going to receive, from the aws_lambda_events crate, for example s3::S3Event
    #[arg(long, conflicts_with_all = ["http", "http_feature"])]
    event_type: Option<String>,
}

#[derive(Clone, Debug, strum_macros::Display, strum_macros::EnumString)]
#[strum(ascii_case_insensitive, serialize_all = "snake_case")]
pub(crate) enum HttpFeature {
    Alb,
    ApigwRest,
    ApigwHttp,
    ApigwWebsockets,
}

impl Options {
    pub(crate) fn validate_options(&mut self, no_interactive: bool) -> Result<(), CreateError> {
        if no_interactive {
            return Ok(());
        }

        if self.http_feature.is_some() && !self.http {
            self.http = true;
        }

        if self.missing_options() {
            if !is_stdin_tty() {
                return Err(CreateError::MissingFunctionOptions);
            }

            self.ask_template_options()?;

            if self.missing_options() {
                return Err(CreateError::MissingFunctionOptions);
            }
        }

        if self.http && self.has_event_type() {
            return Err(CreateError::InvalidFunctionOptions);
        }

        Ok(())
    }

    pub(crate) fn ask_template_options(&mut self) -> Result<(), InquireError> {
        if !self.http {
            self.http = Confirm::new("Is this function an HTTP function?")
                .with_help_message("type `yes` if the Lambda function is triggered by an API Gateway, Amazon Load Balancer(ALB), or a Lambda URL")
                .with_default(false)
                .prompt()?;
        }

        if !self.http {
            let event_type = Text::new("AWS Event type that this function receives")
            .with_autocomplete(suggest_event_type)
            .with_validator(validate_event_type)
            .with_help_message("↑↓ to move, tab to auto-complete, enter to submit. Leave it blank if you don't want to use any event from the aws_lambda_events crate")
            .prompt()?;
            self.event_type = Some(event_type);
        }

        Ok(())
    }

    pub(crate) fn variables(
        &self,
        package_name: &str,
        binary_name: &Option<String>,
    ) -> Result<Object> {
        let use_basic_example = !self.http && !self.has_event_type();

        let (ev_import, ev_feat, ev_type) = self.event_type_triple()?;

        let fn_name = match binary_name {
            Some(name) if name != package_name => Value::scalar(name.clone()),
            _ => Value::Nil,
        };

        let lhv = option_env!("CARGO_LAMBDA_LAMBDA_HTTP_VERSION")
            .map(|v| Value::scalar(v.to_string()))
            .unwrap_or(Value::Nil);

        let lrv = option_env!("CARGO_LAMBDA_LAMBDA_RUNTIME_VERSION")
            .map(|v| Value::scalar(v.to_string()))
            .unwrap_or(Value::Nil);

        let lev = option_env!("CARGO_LAMBDA_LAMBDA_EVENTS_VERSION")
            .map(|v| Value::scalar(v.to_string()))
            .unwrap_or(Value::Nil);

        let http_feature = self
            .http_feature
            .as_ref()
            .map(|v| Value::scalar(v.to_string()))
            .unwrap_or(Value::Nil);

        Ok(liquid::object!({
            "function_name": fn_name,
            "basic_example": use_basic_example,
            "http_function": self.http,
            "http_feature": http_feature,
            "event_type": ev_type,
            "event_type_feature": ev_feat,
            "event_type_import": ev_import,
            "lambda_http_version": lhv,
            "lambda_runtime_version": lrv,
            "aws_lambda_events_version": lev,
        }))
    }

    fn missing_options(&self) -> bool {
        !self.http && self.event_type.is_none()
    }

    fn has_event_type(&self) -> bool {
        matches!(&self.event_type, Some(s) if !s.is_empty())
    }

    fn event_type_triple(&self) -> Result<(Value, Value, Value)> {
        match &self.event_type {
            Some(s) if s == "serde_json::Value" => Ok((
                Value::scalar(s.clone()),
                Value::scalar("serde_json"),
                Value::scalar("Value"),
            )),
            Some(s) if !s.is_empty() => {
                let import = Value::scalar(format!("aws_lambda_events::event::{s}"));
                match s.splitn(2, "::").collect::<Vec<_>>()[..] {
                    [ev_mod, ev_type] => Ok((
                        import,
                        Value::scalar(ev_mod.to_string()),
                        Value::scalar(ev_type.to_string()),
                    )),
                    _ => Err(miette::miette!("unexpected event type")),
                }
            }
            _ => Ok((Value::Nil, Value::Nil, Value::Nil)),
        }
    }
}

fn validate_event_type(name: &str) -> Result<Validation, CustomUserError> {
    match name.is_empty() || crate::events::WELL_KNOWN_EVENTS.contains(&name) {
        true => Ok(Validation::Valid),
        false => Ok(Validation::Invalid(ErrorMessage::Custom(format!(
            "invalid event type: {name}"
        )))),
    }
}

fn suggest_event_type(text: &str) -> Result<Vec<String>, CustomUserError> {
    Ok(crate::events::WELL_KNOWN_EVENTS
        .iter()
        .filter_map(|s| {
            if s.starts_with(text) {
                Some(s.to_string())
            } else {
                None
            }
        })
        .collect())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_http_features_to_string() {
        assert_eq!("apigw_http", HttpFeature::ApigwHttp.to_string().as_str());
    }

    #[test]
    fn test_json_value_event_type() {
        let opt = Options {
            http: false,
            http_feature: None,
            event_type: Some("serde_json::Value".to_string()),
        };

        let (imp, module, kind) = opt.event_type_triple().unwrap();
        assert_eq!(Value::scalar("serde_json::Value"), imp);
        assert_eq!(Value::scalar("serde_json"), module);
        assert_eq!(Value::scalar("Value"), kind);
    }

    #[test]
    fn test_sns_event_type() {
        let opt = Options {
            http: false,
            http_feature: None,
            event_type: Some("sns::SnsEvent".to_string()),
        };

        let (imp, module, kind) = opt.event_type_triple().unwrap();
        assert_eq!(
            Value::scalar("aws_lambda_events::event::sns::SnsEvent"),
            imp
        );
        assert_eq!(Value::scalar("sns"), module);
        assert_eq!(Value::scalar("SnsEvent"), kind);
    }
}
