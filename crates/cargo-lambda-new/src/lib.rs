use cargo_lambda_interactive::command::silent_command;
use cargo_lambda_metadata::fs::rename;
use clap::Args;
use liquid::{model::Value, Object, ParserBuilder};
use miette::{IntoDiagnostic, Result, WrapErr};
use regex::Regex;
use std::{
    collections::HashMap,
    env,
    fs::{copy as copy_file, create_dir_all, File},
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

use crate::template::TemplateSource;

mod events;
mod extensions;
mod functions;
mod template;

#[derive(Args, Clone, Debug)]
#[clap(name = "new")]
pub struct New {
    /// Where to find the project template. It can be a local directory, a local zip file, or a URL to a remote zip file
    #[clap(long)]
    template: Option<String>,

    /// Start a project for a Lambda Extension
    #[clap(long)]
    extension: bool,

    /// Options for function templates
    #[clap(flatten)]
    function_options: functions::Options,

    /// Options for extension templates
    #[clap(flatten)]
    extension_options: extensions::Options,

    /// Open the project in a code editor defined by the environment variable EDITOR
    #[clap(short, long)]
    open: bool,

    /// Name of the binary, independent of the package's name
    #[clap(long, alias = "function-name")]
    bin_name: Option<String>,

    /// Don't show any prompt
    #[clap(long)]
    no_interactive: bool,

    /// List of additional files to render with the template engine
    #[clap(long)]
    render_file: Option<Vec<PathBuf>>,

    /// Map of additional variables to pass to the template engine, in KEY=VALUE format
    #[clap(long)]
    render_var: Option<Vec<String>>,

    /// List of files to ignore from the template
    #[clap(long)]
    ignore_file: Option<Vec<PathBuf>>,

    /// Name of the Rust package to create
    #[clap()]
    package_name: String,
}

impl New {
    #[tracing::instrument(skip(self), target = "cargo_lambda")]
    pub async fn run(&mut self) -> Result<()> {
        tracing::trace!(options = ?self, "creating new project");

        validate_name(&self.package_name)?;
        if let Some(name) = &self.bin_name {
            validate_name(name)?;
        }

        if self.extension {
            self.extension_options.validate_options()?;
        } else {
            self.function_options
                .validate_options(self.no_interactive)?;
        }

        self.create_package().await?;
        self.open_code_editor().await
    }

    async fn create_package(&self) -> Result<()> {
        let template_source = TemplateSource::try_from(self.template_option())?;
        let template_path = template_source.expand().await?;

        let parser = ParserBuilder::with_stdlib().build().into_diagnostic()?;

        let template_vars = if self.extension {
            self.extension_options.variables()?
        } else {
            self.function_options
                .variables(&self.package_name, &self.bin_name)?
        };

        let mut globals = liquid::object!({
            "project_name": self.package_name,
            "binary_name": self.bin_name,
        });
        globals.extend(template_vars);
        globals.extend(self.render_variables());
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

                if entry_name == "LICENSE" || self.is_ignore_file(relative) {
                    continue;
                }

                if entry_name == "Cargo.toml"
                    || entry_name == "README.md"
                    || (entry_name == "main.rs" && parent_name == Some("src"))
                    || (entry_name == "lib.rs" && parent_name == Some("src"))
                    || parent_name == Some("bin")
                    || self.is_render_file(relative)
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

    fn template_option(&self) -> &str {
        match self.template.as_deref() {
            Some(t) => t,
            None if self.extension => extensions::DEFAULT_TEMPLATE_URL,
            None => functions::DEFAULT_TEMPLATE_URL,
        }
    }

    fn is_render_file(&self, path: &Path) -> bool {
        self.render_file
            .as_ref()
            .map(|v| v.contains(&path.to_path_buf()))
            .unwrap_or(false)
    }

    fn render_variables(&self) -> Object {
        let vars = self.render_var.clone().unwrap_or_default();
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

    fn is_ignore_file(&self, path: &Path) -> bool {
        self.ignore_file
            .as_ref()
            .map(|v| v.contains(&path.to_path_buf()))
            .unwrap_or(false)
    }
}

pub(crate) fn validate_name(name: &str) -> Result<()> {
    // TODO(david): use a more extensive verification.
    // See what Cargo does in https://github.com/rust-lang/cargo/blob/42696ae234dfb7b23c9638ad118373826c784c60/src/cargo/util/restricted_names.rs
    let valid_ident = Regex::new(r"^([a-zA-Z][a-zA-Z0-9_-]+)$").into_diagnostic()?;

    match valid_ident.is_match(name) {
        true => Ok(()),
        false => Err(miette::miette!("invalid package name: {}", name)),
    }
}
