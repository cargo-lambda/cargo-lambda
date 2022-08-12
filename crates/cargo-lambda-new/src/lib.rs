use cargo_lambda_interactive::{
    choose_option, command::silent_command, is_stdin_tty, Confirm, Text,
};
use cargo_lambda_metadata::fs::rename;
use clap::Args;
use liquid::{model::Value, ParserBuilder};
use miette::{IntoDiagnostic, Result, WrapErr};
use regex::Regex;
use std::{
    env,
    fs::{copy as copy_file, create_dir_all, File},
};
use walkdir::WalkDir;

use crate::template::TemplateSource;

mod events;
mod template;

#[derive(Args, Clone, Debug)]
#[clap(name = "new")]
pub struct New {
    #[clap(flatten)]
    template_options: TemplateOptions,

    /// Open the project in a code editor defined by the environment variable EDITOR
    #[clap(short, long)]
    open: bool,

    /// Name of the Rust package to create
    #[clap()]
    package_name: String,
}

#[derive(Args, Clone, Debug, Default)]
pub struct TemplateOptions {
    /// Where to find the project template. It can be a local directory, a local zip file, or a URL to a remote zip file
    #[clap(long)]
    template: Option<String>,

    /// Whether the function is going to be an HTTP endpoint or not
    #[clap(long)]
    http: bool,

    /// The specific HTTP feature to enable
    #[clap(long)]
    http_feature: Option<HttpFeature>,

    /// Name of function's binary, independent of the package's name
    #[clap(long)]
    function_name: Option<String>,

    /// Type of AWS event that this function is going to receive, from the aws_lambda_events crate, for example s3::S3Event
    #[clap(long)]
    event_type: Option<String>,
}

#[derive(Clone, Debug, strum_macros::Display, strum_macros::EnumString)]
#[strum(ascii_case_insensitive, serialize_all = "snake_case")]
enum HttpFeature {
    Alb,
    ApigwRest,
    ApigwHttp,
    ApigwWebsockets,
}

enum HttpEndpoints {
    Alb,
    ApigwRest,
    ApigwHttp,
    ApigwWebsockets,
    LambdaUrls,
    Unknown,
}

impl std::fmt::Display for HttpEndpoints {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Alb => write!(f, "Amazon Elastic Application Load Balancer (ALB)"),
            Self::ApigwRest => write!(f, "Amazon Api Gateway REST Api"),
            Self::ApigwHttp => write!(f, "Amazon Api Gateway HTTP Api"),
            Self::ApigwWebsockets => write!(f, "Amazon Api Gateway Websockets"),
            Self::LambdaUrls => write!(f, "AWS Lambda function URLs"),
            Self::Unknown => write!(f, "I don't know yet"),
        }
    }
}

impl HttpEndpoints {
    fn to_feature(&self) -> Option<HttpFeature> {
        match self {
            Self::Alb => Some(HttpFeature::Alb),
            Self::ApigwRest => Some(HttpFeature::ApigwRest),
            Self::ApigwHttp | Self::LambdaUrls => Some(HttpFeature::ApigwHttp),
            Self::ApigwWebsockets => Some(HttpFeature::ApigwWebsockets),
            Self::Unknown => None,
        }
    }

    fn all() -> Vec<HttpEndpoints> {
        vec![
            HttpEndpoints::Alb,
            HttpEndpoints::ApigwRest,
            HttpEndpoints::ApigwHttp,
            HttpEndpoints::ApigwWebsockets,
            HttpEndpoints::LambdaUrls,
            HttpEndpoints::Unknown,
        ]
    }
}

impl New {
    #[tracing::instrument(skip(self), target = "cargo_lambda")]
    pub async fn run(&mut self) -> Result<()> {
        tracing::trace!(options = ?self, "creating new project");

        validate_name(&self.package_name)?;
        if self.template_options.http_feature.is_some() && !self.is_http_function() {
            self.template_options.http = true;
        }

        if self.missing_options() {
            if !is_stdin_tty() {
                return Err(miette::miette!("missing options: --event-type, --http"));
            }

            self.ask_template_options()?;
            if self.missing_options() {
                return Err(miette::miette!("missing options: --event-type, --http"));
            }
        }

        if self.is_http_function() && self.has_event_type() {
            return Err(miette::miette!(
                "invalid options: --event-type and --http cannot be specified at the same time"
            ));
        }

        self.create_package().await?;
        self.open_code_editor().await
    }

    fn ask_template_options(&mut self) -> Result<()> {
        if let Some(fn_name) = &self.template_options.function_name {
            validate_name(fn_name)?;
        }

        if !self.template_options.http {
            let is_http = Confirm::new("Is this function an HTTP function?")
                .with_help_message("type `yes` if the Lambda function is triggered by an API Gateway, Amazon Load Balancer(ALB), or a Lambda URL")
                .with_default(false)
                .prompt()
                .into_diagnostic()?;
            self.template_options.http = is_http;
        }

        if self.template_options.http && self.template_options.http_feature.is_none() {
            let http_endpoint = choose_option(
                "Which service is this function receiving events from?",
                HttpEndpoints::all(),
            )
            .into_diagnostic()?;
            self.template_options.http_feature = http_endpoint.to_feature();
        }

        if !self.template_options.http {
            let event_type = Text::new("AWS Event type that this function receives")
            .with_suggester(&suggest_event_type)
            .with_validator(&validate_event_type)
            .with_help_message("↑↓ to move, tab to auto-complete, enter to submit. Leave it blank if you don't want to use any event from the aws_lambda_events crate")
            .prompt()
            .into_diagnostic()?;
            self.template_options.event_type = Some(event_type);
        }

        Ok(())
    }

    fn missing_options(&self) -> bool {
        self.template_options.missing_options()
    }

    fn is_http_function(&self) -> bool {
        self.template_options.http
    }

    fn has_event_type(&self) -> bool {
        matches!(&self.template_options.event_type, Some(s) if !s.is_empty())
    }

    fn event_type_triple(&self) -> Result<(Value, Value, Value)> {
        match &self.template_options.event_type {
            Some(s) if !s.is_empty() => {
                let import = Value::scalar(format!("aws_lambda_events::event::{}", s));
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

    async fn create_package(&self) -> Result<()> {
        let template_source = TemplateSource::try_from(self.template_options.template.as_deref())?;
        let template_path = template_source.expand().await?;

        let parser = ParserBuilder::with_stdlib().build().into_diagnostic()?;

        let use_basic_example = !self.is_http_function() && !self.has_event_type();

        let (ev_import, ev_feat, ev_type) = self.event_type_triple()?;

        let fn_name = match self.template_options.function_name.as_deref() {
            Some(fn_name) if fn_name != self.package_name => Value::scalar(fn_name.to_string()),
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
            .template_options
            .http_feature
            .as_ref()
            .map(|v| Value::scalar(v.to_string()))
            .unwrap_or(Value::Nil);

        let globals = liquid::object!({
            "project_name": self.package_name,
            "function_name": fn_name,
            "basic_example": use_basic_example,
            "http_function": self.is_http_function(),
            "http_feature": http_feature,
            "event_type": ev_type,
            "event_type_feature": ev_feat,
            "event_type_import": ev_import,
            "lambda_http_version": lhv,
            "lambda_runtime_version": lrv,
            "aws_lambda_events_version": lev,
        });
        tracing::debug!(variables = ?globals, "rendering templates");

        let render_dir = tempfile::tempdir().into_diagnostic()?;
        let render_path = render_dir.path();

        let walk_dir = WalkDir::new(&template_path).follow_links(false);
        for entry in walk_dir {
            let entry = entry.into_diagnostic()?;
            let entry_path = entry.path();

            let entry_name = entry_path
                .file_name()
                .ok_or_else(|| miette::miette!("invalid entry: {:?}", &entry_path))?;

            if entry_path.is_dir() {
                if entry_name != ".git" {
                    create_dir_all(&entry_path).into_diagnostic()?;
                }
            } else if entry_name == "cargo-lambda-template.zip" {
                continue;
            } else {
                let relative = entry_path.strip_prefix(&template_path).into_diagnostic()?;
                let new_path = render_path.join(relative);
                let parent_name = if let Some(parent) = new_path.parent() {
                    create_dir_all(parent).into_diagnostic()?;
                    parent.file_name().and_then(|p| p.to_str())
                } else {
                    None
                };

                if entry_name == "Cargo.toml"
                    || entry_name == "README.md"
                    || (entry_name == "main.rs" && parent_name == Some("src"))
                {
                    let template = parser.parse_file(&entry_path).into_diagnostic()?;

                    let mut file = File::create(&new_path).into_diagnostic()?;
                    template
                        .render_to(&mut file, &globals)
                        .into_diagnostic()
                        .wrap_err_with(|| {
                            format!("failed to render template file: {:?}", &new_path)
                        })?;
                } else {
                    copy_file(&entry_path, &new_path)
                        .into_diagnostic()
                        .wrap_err_with(|| {
                            format!(
                                "failed to copy file: from {:?} to {:?}",
                                &entry_path, &new_path
                            )
                        })?;
                }
            }
        }

        rename(&render_path, &self.package_name)
            .into_diagnostic()
            .wrap_err_with(|| {
                format!(
                    "failed to move package: from {:?} to {:?}",
                    &render_path, &self.package_name
                )
            })?;

        Ok(())
    }

    async fn open_code_editor(&self) -> Result<()> {
        if !self.open {
            return Ok(());
        }
        let editor = env::var("EDITOR").unwrap_or_default();
        let editor = editor.trim();
        if editor.is_empty() {
            Err(miette::miette!(
                "project created in {}, but the EDITOR variable is missing",
                &self.package_name
            ))
        } else {
            silent_command(editor.trim(), &[&self.package_name]).await
        }
    }
}

impl TemplateOptions {
    fn missing_options(&self) -> bool {
        !self.http && self.event_type.is_none()
    }
}

fn validate_name(name: &str) -> Result<()> {
    // TODO(david): use a more extensive verification.
    // See what Cargo does in https://github.com/rust-lang/cargo/blob/42696ae234dfb7b23c9638ad118373826c784c60/src/cargo/util/restricted_names.rs
    let valid_ident = Regex::new(r"^([a-zA-Z][a-zA-Z0-9_-]+)$").into_diagnostic()?;

    match valid_ident.is_match(name) {
        true => Ok(()),
        false => Err(miette::miette!("invalid package name: {}", name)),
    }
}

fn validate_event_type(name: &str) -> Result<(), String> {
    match name.is_empty() || events::WELL_KNOWN_EVENTS.contains(&name) {
        true => Ok(()),
        false => Err(format!("invalid event type: {}", name)),
    }
}

fn suggest_event_type(text: &str) -> Vec<String> {
    events::WELL_KNOWN_EVENTS
        .iter()
        .filter_map(|s| {
            if s.starts_with(text) {
                Some(s.to_string())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_http_features_to_string() {
        assert_eq!("apigw_http", HttpFeature::ApigwHttp.to_string().as_str());
    }
}
