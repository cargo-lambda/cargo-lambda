use cargo_lambda_interactive::{command::silent_command, is_user_cancellation_error};
use cargo_lambda_metadata::fs::{copy_without_replace, rename};
use clap::Args;
use liquid::{model::Value, Object, ParserBuilder};
use miette::{IntoDiagnostic, Result, WrapErr};
use regex::Regex;
use std::{
    collections::HashMap,
    env,
    fmt::Debug,
    fs::{copy as copy_file, create_dir_all, File},
    path::{Path, PathBuf},
};
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
    #[arg(long, alias = "default")]
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
#[command(name = "init")]
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
#[command(name = "new")]
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

    if config.extension {
        config.extension_options.validate_options()?;
    } else {
        match config
            .function_options
            .validate_options(config.no_interactive)
        {
            Err(CreateError::UnexpectedInput(err)) if is_user_cancellation_error(&err) => {
                return Ok(())
            }
            Err(err) => return Err(err.into()),
            Ok(()) => {}
        }
    }

    create_project(name, &path, config, replace).await?;
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

async fn create_project<T: AsRef<Path> + Debug>(
    name: &str,
    path: T,
    config: &Config,
    replace: bool,
) -> Result<()> {
    let template_option = match config.template.as_deref() {
        Some(t) => t,
        None if config.extension => extensions::DEFAULT_TEMPLATE_URL,
        None => functions::DEFAULT_TEMPLATE_URL,
    };

    let template_source = TemplateSource::try_from(template_option)?;
    let template_path = template_source.expand().await?;

    let parser = ParserBuilder::with_stdlib().build().into_diagnostic()?;

    let template_vars = if config.extension {
        config.extension_options.variables()?
    } else {
        config.function_options.variables(name, &config.bin_name)?
    };

    let mut globals = liquid::object!({
        "project_name": name,
        "binary_name": config.bin_name,
    });
    globals.extend(template_vars);
    globals.extend(render_variables(config));
    tracing::debug!(variables = ?globals, "rendering templates");

    let render_dir = tempfile::tempdir().into_diagnostic()?;
    let render_path = render_dir.path();

    let walk_dir = WalkDir::new(&template_path).follow_links(false);
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
                    .wrap_err_with(|| format!("unable to create directory: {:?}", entry_path))?;
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

            if entry_name == "LICENSE" || is_ignore_file(config, relative) {
                continue;
            }

            if entry_name == "Cargo.toml"
                || entry_name == "README.md"
                || (entry_name == "main.rs" && parent_name == Some("src"))
                || (entry_name == "lib.rs" && parent_name == Some("src"))
                || parent_name == Some("bin")
                || is_render_file(config, relative)
            {
                let template = parser.parse_file(entry_path).into_diagnostic()?;

                let mut file = File::create(&new_path)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("unable to create file: {:?}", new_path))?;

                template
                    .render_to(&mut file, &globals)
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
        rename(render_path, &path)
    } else {
        copy_without_replace(render_path, &path)
    };

    res.into_diagnostic().wrap_err_with(|| {
        format!(
            "failed to create package: template {:?} to {:?}",
            render_path, path
        )
    })
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
        Err(CreateError::InvalidEditor(path.into()).into())
    } else {
        silent_command(editor.trim(), &[path]).await
    }
}

fn is_render_file(config: &Config, path: &Path) -> bool {
    config
        .render_file
        .as_ref()
        .map(|v| v.contains(&path.to_path_buf()))
        .unwrap_or(false)
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

fn is_ignore_file(config: &Config, path: &Path) -> bool {
    config
        .ignore_file
        .as_ref()
        .map(|v| v.contains(&path.to_path_buf()))
        .unwrap_or(false)
}
