use cargo_lambda_interactive::{
    command::new_command, is_user_cancellation_error, progress::Progress,
};
use cargo_lambda_metadata::fs::{copy_and_replace, copy_without_replace};
use clap::Args;
use liquid::{model::Value, Object, Parser, ParserBuilder};
use miette::{IntoDiagnostic, Result, WrapErr};
use regex::Regex;
use std::{
    collections::HashMap,
    env,
    fmt::Debug,
    fs::{copy as copy_file, create_dir_all, File},
    path::{Path, PathBuf},
};
use template::{config::TemplateConfig, TemplateRoot};
use walkdir::WalkDir;

use crate::template::TemplateSource;

mod error;
use error::CreateError;

mod events;
mod extensions;
mod functions;
mod template;

#[derive(Args, Clone, Debug)]
#[group(skip)]
struct Config {
    /// Where to find the project template. It can be a local directory, a local zip file, or a URL to a remote zip file
    #[arg(long)]
    template: Option<String>,

    /// Start a project for a Lambda Extension
    #[arg(long)]
    extension: bool,

    /// Options for function templates
    #[command(flatten)]
    function_options: functions::Options,

    /// Options for extension templates
    #[command(flatten)]
    extension_options: extensions::Options,

    /// Open the project in a code editor defined by the environment variable EDITOR
    #[arg(short, long)]
    open: bool,

    /// Name of the binary, independent of the package's name
    #[arg(long, alias = "function-name")]
    bin_name: Option<String>,

    /// Apply the default template values without any prompt
    #[arg(short = 'y', long, alias = "default")]
    no_interactive: bool,

    /// List of additional files to render with the template engine
    #[arg(long)]
    render_file: Option<Vec<PathBuf>>,

    /// Map of additional variables to pass to the template engine, in KEY=VALUE format
    #[arg(long)]
    render_var: Option<Vec<String>>,

    /// List of files to ignore from the template
    #[arg(long)]
    ignore_file: Option<Vec<PathBuf>>,
}

#[derive(Args, Clone, Debug)]
#[command(
    name = "init",
    after_help = "Full command documentation: https://www.cargo-lambda.info/commands/init.html"
)]
pub struct Init {
    #[command(flatten)]
    config: Config,

    /// Name of the Rust package, defaults to the directory name
    #[arg(long)]
    name: Option<String>,

    #[arg(default_value = ".")]
    path: PathBuf,
}

impl Init {
    #[tracing::instrument(skip(self), target = "cargo_lambda")]
    pub async fn run(&mut self) -> Result<()> {
        if !self.path.is_dir() {
            Err(CreateError::NotADirectoryPath(self.path.to_path_buf()))?;
        }

        if self.path.join("Cargo.toml").is_file() {
            Err(CreateError::InvalidPackageRoot)?;
        }

        let path = dunce::canonicalize(&self.path).map_err(CreateError::InvalidPath)?;

        let name = self
            .name
            .as_deref()
            .or_else(|| path.file_name().and_then(|s| s.to_str()))
            .ok_or_else(|| miette::miette!("invalid package name"))?;

        new_project(name, &path, &mut self.config, false).await
    }
}

#[derive(Args, Clone, Debug)]
#[command(
    name = "new",
    after_help = "Full command documentation: https://www.cargo-lambda.info/commands/new.html"
)]
pub struct New {
    #[command(flatten)]
    config: Config,

    /// Name of the Rust package to create
    #[arg()]
    name: String,
}

impl New {
    #[tracing::instrument(skip(self), target = "cargo_lambda")]
    pub async fn run(&mut self) -> Result<()> {
        new_project(&self.name, &self.name, &mut self.config, true).await
    }
}

#[tracing::instrument(target = "cargo_lambda")]
async fn new_project<T: AsRef<Path> + Debug>(
    name: &str,
    path: T,
    config: &mut Config,
    replace: bool,
) -> Result<()> {
    tracing::trace!(name, ?path, ?config, "creating new project");

    validate_name(name)?;
    if let Some(name) = &config.bin_name {
        validate_name(name)?;
    }

    let template = get_template(config).await?;
    template.cleanup();

    let template_config = template::config::parse_template_config(template.config_path())?;
    let ignore_default_prompts = template_config.disable_default_prompts || config.no_interactive;

    if config.extension {
        config.extension_options.validate_options()?;
    } else {
        match config
            .function_options
            .validate_options(ignore_default_prompts)
        {
            Err(CreateError::UnexpectedInput(err)) if is_user_cancellation_error(&err) => {
                return Ok(())
            }
            Err(err) => return Err(err.into()),
            Ok(()) => {}
        }
    }

    let globals = build_template_variables(config, &template_config, name)?;
    let render_files = build_render_files(config, &template_config);
    let ignore_files = build_ignore_files(config, &template_config);

    create_project(
        &path,
        &template.final_path(),
        &template_config,
        &globals,
        &render_files,
        &ignore_files,
        replace,
    )
    .await?;
    if config.open {
        let path_ref = path.as_ref();
        let path_str = path_ref
            .to_str()
            .ok_or_else(|| CreateError::NotADirectoryPath(path_ref.to_path_buf()))?;
        open_code_editor(path_str).await
    } else {
        Ok(())
    }
}

async fn get_template(config: &Config) -> Result<TemplateRoot> {
    let progress = Progress::start("downloading template");

    let template_option = match config.template.as_deref() {
        Some(t) => t,
        None if config.extension => extensions::DEFAULT_TEMPLATE_URL,
        None => functions::DEFAULT_TEMPLATE_URL,
    };

    let template_source = TemplateSource::try_from(template_option);
    match template_source {
        Ok(ts) => {
            let result = ts.expand().await;
            progress.finish_and_clear();
            result
        }
        Err(e) => {
            progress.finish_and_clear();
            Err(e)
        }
    }
}

#[tracing::instrument(target = "cargo_lambda")]
async fn create_project<T: AsRef<Path> + Debug>(
    path: T,
    template_path: &Path,
    template_config: &TemplateConfig,
    globals: &Object,
    render_files: &[PathBuf],
    ignore_files: &[PathBuf],
    replace: bool,
) -> Result<()> {
    tracing::trace!("rendering new project's template");

    let parser = ParserBuilder::with_stdlib().build().into_diagnostic()?;

    let render_dir = tempfile::tempdir().into_diagnostic()?;
    let render_path = render_dir.path();

    let walk_dir = WalkDir::new(template_path).follow_links(false);
    for entry in walk_dir {
        let entry = entry.into_diagnostic()?;
        let entry_path = entry.path();

        let entry_name = entry_path
            .file_name()
            .ok_or_else(|| CreateError::InvalidTemplateEntry(entry_path.to_path_buf()))?;

        if entry_path.is_dir() {
            if entry_name != ".git" {
                create_dir_all(entry_path)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("unable to create directory: {entry_path:?}"))?;
            }
        } else if entry_name == "cargo-lambda-template.zip" {
            continue;
        } else {
            let relative = entry_path.strip_prefix(template_path).into_diagnostic()?;

            if should_ignore_file(relative, ignore_files, template_config, globals) {
                continue;
            }

            let mut new_path = render_path.join(relative);
            if let Some(path) = render_path_with_variables(&new_path, &parser, globals) {
                new_path = path;
            }

            let parent_name = if let Some(parent) = new_path.parent() {
                create_dir_all(parent).into_diagnostic()?;
                parent.file_name().and_then(|p| p.to_str())
            } else {
                None
            };

            if entry_name == "Cargo.toml"
                || entry_name == "README.md"
                || (entry_name == "main.rs" && parent_name == Some("src"))
                || (entry_name == "lib.rs" && parent_name == Some("src"))
                || parent_name == Some("bin")
                || should_render_file(relative, render_files, template_config, globals)
            {
                let template = parser.parse_file(entry_path).into_diagnostic()?;

                let mut file = File::create(&new_path)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("unable to create file: {new_path:?}"))?;

                template
                    .render_to(&mut file, globals)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("failed to render template file: {:?}", &new_path))?;
            } else {
                copy_file(entry_path, &new_path)
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

    let res = if replace {
        copy_and_replace(render_path, &path)
    } else {
        copy_without_replace(render_path, &path)
    };

    res.into_diagnostic()
        .wrap_err_with(|| format!("failed to create package: template {render_path:?} to {path:?}"))
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    // TODO(david): use a more extensive verification.
    // See what Cargo does in https://github.com/rust-lang/cargo/blob/42696ae234dfb7b23c9638ad118373826c784c60/src/cargo/util/restricted_names.rs
    let valid_ident = Regex::new(r"^([a-zA-Z][a-zA-Z0-9_-]+)$").into_diagnostic()?;

    match valid_ident.is_match(name) {
        true => Ok(()),
        false => Err(CreateError::InvalidPackageName(name.to_string()).into()),
    }
}

async fn open_code_editor(path: &str) -> Result<()> {
    let editor = env::var("EDITOR").unwrap_or_default();
    let editor = editor.trim();
    if editor.is_empty() {
        return Err(CreateError::InvalidEditor(path.into()).into());
    }

    let mut child = new_command(editor)
        .args([path])
        .spawn()
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to run `{editor} {path}`"))?;

    child
        .wait()
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to wait on {editor} process"))
        .map(|_| ())
}

fn render_variables(config: &Config) -> Object {
    let vars = config.render_var.clone().unwrap_or_default();
    let mut map = HashMap::new();

    for var in vars {
        let mut split = var.splitn(2, '=');
        if let (Some(k), Some(v)) = (split.next(), split.next()) {
            map.insert(k.to_string(), v.to_string());
        }
    }

    let mut object = Object::new();
    for (k, v) in map {
        object.insert(k.into(), Value::scalar(v));
    }

    object
}

fn build_template_variables(
    config: &Config,
    template_config: &TemplateConfig,
    name: &str,
) -> Result<Object> {
    let mut variables = liquid::object!({
        "project_name": name,
        "binary_name": config.bin_name,
    });

    if config.extension {
        variables.extend(config.extension_options.variables()?);
    } else {
        variables.extend(config.function_options.variables(name, &config.bin_name)?);
    };

    if !template_config.prompts.is_empty() {
        let template_variables = template_config.ask_template_options(config.no_interactive)?;
        variables.extend(template_variables);
    }

    variables.extend(render_variables(config));
    tracing::debug!(?variables, "collected template variables");

    Ok(variables)
}

fn build_render_files(config: &Config, template_config: &TemplateConfig) -> Vec<PathBuf> {
    let mut render_files = template_config.render_files.clone();
    render_files.extend(config.render_file.clone().unwrap_or_default());
    render_files
}

fn build_ignore_files(config: &Config, template_config: &TemplateConfig) -> Vec<PathBuf> {
    let mut ignore_files = template_config.ignore_files.clone();
    ignore_files.extend(config.ignore_file.clone().unwrap_or_default());
    ignore_files
}

fn should_render_file(
    relative: &Path,
    render_files: &[PathBuf],
    template_config: &TemplateConfig,
    variables: &Object,
) -> bool {
    if template_config.render_all_files {
        return true;
    }

    if render_files.contains(&relative.to_path_buf()) {
        return true;
    }

    let Some(unix_path) = convert_to_unix_path(relative) else {
        return false;
    };

    if render_files.contains(&PathBuf::from(&unix_path)) {
        return true;
    }

    let condition = template_config
        .render_conditional_files
        .get(&unix_path)
        .or_else(|| {
            relative
                .to_str()
                .and_then(|s| template_config.render_conditional_files.get(s))
        });

    if let Some(condition) = condition {
        let Some(variable) = variables.get::<str>(&condition.var) else {
            return false;
        };

        if let Some(condition_value) = &condition.r#match {
            if condition_value.to_value() == *variable {
                return true;
            }
        }

        if let Some(condition_value) = &condition.not_match {
            if condition_value.to_value() != *variable {
                return true;
            }
        }
    }

    false
}

fn should_ignore_file(
    relative: &Path,
    ignore_files: &[PathBuf],
    template_config: &TemplateConfig,
    variables: &Object,
) -> bool {
    if ignore_files.contains(&relative.to_path_buf()) {
        return true;
    }

    let Some(unix_path) = convert_to_unix_path(relative) else {
        return false;
    };

    if ignore_files.contains(&PathBuf::from(&unix_path)) {
        return true;
    }

    let condition = template_config
        .ignore_conditional_files
        .get(&unix_path)
        .or_else(|| {
            relative
                .to_str()
                .and_then(|s| template_config.ignore_conditional_files.get(s))
        });

    if let Some(condition) = condition {
        let Some(variable) = variables.get::<str>(&condition.var) else {
            return false;
        };

        if let Some(condition_value) = &condition.r#match {
            if condition_value.to_value() == *variable {
                return true;
            }
        }

        if let Some(condition_value) = &condition.not_match {
            if condition_value.to_value() != *variable {
                return true;
            }
        }
    }

    false
}

fn render_path_with_variables(path: &Path, parser: &Parser, variables: &Object) -> Option<PathBuf> {
    let re = regex::Regex::new(r"\{\{[^/]*\}\}").ok()?;

    let path_str = path.to_string_lossy();
    if !re.is_match(&path_str) {
        return None;
    }

    let template = parser.parse(&path_str).ok()?;
    let path_str = template.render(&variables).ok()?;

    Some(PathBuf::from(path_str))
}

#[cfg(target_os = "windows")]
fn convert_to_unix_path(path: &Path) -> Option<String> {
    let mut path_str = String::new();
    for component in path.components() {
        if let std::path::Component::Normal(os_str) = component {
            if !path_str.is_empty() {
                path_str.push('/');
            }
            path_str.push_str(os_str.to_str()?);
        }
    }
    Some(path_str)
}

#[cfg(not(target_os = "windows"))]
fn convert_to_unix_path(path: &Path) -> Option<String> {
    path.to_str().map(String::from)
}

#[cfg(test)]
mod tests {
    use liquid::{model::Value, Object};
    use template::config::{PromptValue, RenderCondition};

    use super::*;

    #[test]
    fn test_render_relative_path_with_render_conditional_files() {
        #[cfg(not(target_os = "windows"))]
        let path = Path::new("src/main.rs");
        #[cfg(target_os = "windows")]
        let path = Path::new("src\\main.rs");

        let render_files = vec![];
        let mut template_config = TemplateConfig::default();
        template_config.render_conditional_files.insert(
            "src/main.rs".into(),
            RenderCondition {
                var: "render_main_rs".into(),
                r#match: Some(PromptValue::Boolean(true)),
                not_match: None,
            },
        );
        let mut variables = Object::new();
        variables.insert("render_main_rs".into(), Value::scalar(true));

        assert!(should_render_file(
            &path,
            &render_files,
            &template_config,
            &variables
        ));
    }

    #[test]
    fn test_render_relative_path_with_render_files() {
        #[cfg(not(target_os = "windows"))]
        let path = Path::new("src/main.rs");
        #[cfg(target_os = "windows")]
        let path = Path::new("src\\main.rs");

        let render_files = vec![PathBuf::from("src/main.rs")];
        let template_config = TemplateConfig::default();
        let variables = Object::new();
        assert!(should_render_file(
            &path,
            &render_files,
            &template_config,
            &variables
        ));
    }

    #[test]
    fn test_render_relative_path_with_render_conditional_files_false() {
        #[cfg(not(target_os = "windows"))]
        let path = Path::new("src/main.rs");
        #[cfg(target_os = "windows")]
        let path = Path::new("src\\main.rs");

        let render_files = vec![];
        let template_config = TemplateConfig::default();
        let variables = Object::new();
        assert!(!should_render_file(
            &path,
            &render_files,
            &template_config,
            &variables
        ));
    }

    #[test]
    fn test_render_relative_path_with_render_all_files() {
        #[cfg(not(target_os = "windows"))]
        let path = Path::new("src/main.rs");
        #[cfg(target_os = "windows")]
        let path = Path::new("src\\main.rs");

        let render_files = vec![];
        let mut template_config = TemplateConfig::default();
        template_config.render_all_files = true;
        let variables = Object::new();
        assert!(should_render_file(
            &path,
            &render_files,
            &template_config,
            &variables
        ));
    }

    #[test]
    fn test_render_path_with_variables() {
        #[cfg(not(target_os = "windows"))]
        let path = Path::new("{{ci_provider}}/actions/build.yml");
        #[cfg(target_os = "windows")]
        let path = Path::new("{{ci_provider}}\\actions\\build.yml");

        #[cfg(not(target_os = "windows"))]
        let expected = PathBuf::from(".github/actions/build.yml");
        #[cfg(target_os = "windows")]
        let expected = PathBuf::from(".github\\actions\\build.yml");

        let parser = ParserBuilder::with_stdlib().build().unwrap();
        let mut variables = Object::new();
        variables.insert("ci_provider".into(), Value::scalar(".github"));

        assert_eq!(
            render_path_with_variables(&path, &parser, &variables),
            Some(expected)
        );
    }

    #[test]
    fn test_should_ignore_file() {
        #[cfg(not(target_os = "windows"))]
        let path = Path::new("src/http.rs");
        #[cfg(target_os = "windows")]
        let path = Path::new("src\\http.rs");

        let ignore_files = vec![];
        let mut template_config = TemplateConfig::default();
        template_config.ignore_conditional_files.insert(
            "src/http.rs".into(),
            RenderCondition {
                var: "http_function".into(),
                r#match: None,
                not_match: Some(PromptValue::Boolean(true)),
            },
        );

        let mut variables = Object::new();
        variables.insert("http_function".into(), Value::scalar(false));

        assert!(should_ignore_file(
            &path,
            &ignore_files,
            &template_config,
            &variables
        ));
    }

    #[test]
    fn test_should_not_ignore_file() {
        #[cfg(not(target_os = "windows"))]
        let path = Path::new("src/http.rs");
        #[cfg(target_os = "windows")]
        let path = Path::new("src\\http.rs");

        let ignore_files = vec![];
        let mut template_config = TemplateConfig::default();
        template_config.ignore_conditional_files.insert(
            "src/http.rs".into(),
            RenderCondition {
                var: "http_function".into(),
                r#match: None,
                not_match: Some(PromptValue::Boolean(true)),
            },
        );

        let mut variables = Object::new();
        variables.insert("http_function".into(), Value::scalar(true));

        assert!(!should_ignore_file(
            &path,
            &ignore_files,
            &template_config,
            &variables
        ));
    }
}
