use std::{collections::HashMap, path::PathBuf};

use cargo_lambda_metadata::{
    cargo::load_metadata,
    config::{
        Config, ConfigOptions, general_config_figment, get_config_from_all_packages,
        load_config_without_cli_flags,
    },
};
use clap::Args;
use miette::{IntoDiagnostic, Result};

use cargo_lambda_build::zig::{ZigInfo, check_installation, get_zig_info};
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

    pub fn pkg_name(&self) -> Option<String> {
        self.package.clone()
    }
}

#[derive(Debug, Serialize)]
struct Info {
    zig: ZigInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    config: Option<ConfigInfo>,
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
pub async fn run(config: &System, options: &ConfigOptions) -> Result<()> {
    trace!("running config command");

    if config.setup {
        let zig_info = check_installation().await?;
        return print_config(config.output_format.as_ref(), zig_info);
    }

    let mut info = Info {
        zig: get_zig_info()?,
        config: None,
    };
    let manifest_path = config.manifest_path();

    if manifest_path.exists() {
        let metadata = load_metadata(manifest_path, None)?;

        let config_info = if !options.names.is_empty() || metadata.packages.len() == 1 {
            let config = load_config_without_cli_flags(&metadata, options)?;
            ConfigInfo::Package(config)
        } else {
            let (_, _, workspace) = general_config_figment(&metadata, options)?;

            let packages = get_config_from_all_packages(&metadata)?;

            ConfigInfo::Global {
                workspace: workspace.extract().into_diagnostic()?,
                packages,
            }
        };

        info.config = Some(config_info);
    }

    print_config(config.output_format.as_ref(), info)
}

fn print_config(format: Option<&OutputFormat>, info: impl Serialize) -> Result<()> {
    match format {
        Some(OutputFormat::Json) => {
            serde_json::to_writer_pretty(std::io::stdout(), &info).into_diagnostic()
        }
        _ => serde_yml::to_writer(std::io::stdout(), &info).into_diagnostic(),
    }
}
