use std::{collections::HashMap, path::PathBuf};

use cargo_lambda_metadata::{
    cargo::CargoMetadata,
    config::{
        Config, ConfigOptions, general_config_figment, get_config_from_all_packages,
        load_config_without_cli_flags,
    },
};
use clap::Args;
use miette::{IntoDiagnostic, Result};

use cargo_lambda_build::{InstallOption, Zig, install_options, install_zig, print_install_options};
use cargo_lambda_interactive::is_stdin_tty;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumString};
use tracing::trace;

#[derive(Clone, Debug, Default, Deserialize, Display, EnumString, Serialize)]
#[strum(ascii_case_insensitive)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Args, Clone, Debug)]
#[command(
    visible_alias = "config",
    after_help = "Full command documentation: https://www.cargo-lambda.info/commands/system.html"
)]
pub struct System {
    /// Setup and install Zig if it is not already installed.
    #[arg(long, visible_alias = "install-zig", alias = "install")]
    setup: bool,

    /// Manifest path to show information for
    #[arg(short, long)]
    manifest_path: Option<PathBuf>,

    /// Format to render the output (text, or json)
    #[arg(short, long)]
    output_format: Option<OutputFormat>,

    /// Package name to show information for
    #[arg(short, long)]
    package: Option<String>,
}

impl System {
    pub fn manifest_path(&self) -> PathBuf {
        self.manifest_path
            .clone()
            .unwrap_or_else(|| "Cargo.toml".into())
    }

    pub fn package(&self) -> Option<String> {
        self.package.clone()
    }
}

#[derive(Debug, Default, Serialize)]
struct ZigInfo {
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    install_options: Option<Vec<InstallOption>>,
}

#[derive(Debug, Serialize)]
struct Info {
    zig: ZigInfo,
    config: ConfigInfo,
}

#[derive(Debug, Serialize)]
enum ConfigInfo {
    #[serde(rename = "package")]
    Package(Config),
    #[serde(rename = "global")]
    Global {
        workspace: Config,
        packages: HashMap<String, Config>,
    },
}

#[tracing::instrument(target = "cargo_lambda")]
pub async fn run(config: &System, metadata: &CargoMetadata, options: &ConfigOptions) -> Result<()> {
    trace!("running config command");

    if config.setup {
        let options = install_options();
        if is_stdin_tty() {
            install_zig(options).await?;
        } else {
            print_install_options(&options);
        }

        return Ok(());
    }

    let config_info = if options.name.is_some() || metadata.packages.len() == 1 {
        let config = load_config_without_cli_flags(metadata, options)?;
        ConfigInfo::Package(config)
    } else {
        let (_, _, workspace) = general_config_figment(metadata, options)?;

        let packages = get_config_from_all_packages(metadata)?;

        ConfigInfo::Global {
            workspace: workspace.extract().into_diagnostic()?,
            packages,
        }
    };

    let zig_info = if let Ok((path, _)) = Zig::find_zig() {
        ZigInfo {
            path: Some(path),
            install_options: None,
        }
    } else {
        let options = install_options();
        ZigInfo {
            path: None,
            install_options: Some(options),
        }
    };

    let info = Info {
        zig: zig_info,
        config: config_info,
    };

    match config.output_format {
        Some(OutputFormat::Json) => {
            println!("{}", serde_json::to_string_pretty(&info).into_diagnostic()?);
        }
        _ => {
            let data = serde_yml::to_string(&info).unwrap();
            bat::PrettyPrinter::new()
                .language("yaml")
                .input_from_bytes(data.as_bytes())
                .colored_output(false)
                .print()
                .into_diagnostic()?;
        }
    }

    Ok(())
}
