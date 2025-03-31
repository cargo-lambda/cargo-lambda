use crate::error::BuildError;
use cargo_lambda_interactive::{
    choose_option, command::silent_command, is_stdin_tty, progress::Progress,
};
use cargo_zigbuild::Zig;
use miette::{IntoDiagnostic, Result};
use serde::Serialize;
use std::{path::PathBuf, process::Command};

#[derive(Debug, Default, Serialize)]
pub struct ZigInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    install_options: Option<Vec<InstallOption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
}

/// Print information about the Zig installation.
pub fn print_install_options(options: &[InstallOption]) {
    println!("You can use any of the following options to install it:");
    for option in options {
        println!("\t* {}: `{}`", option, option.usage());
    }
    println!(
        "Or download Zig 0.13.0 or newer from https://ziglang.org/download/ and add it to your PATH"
    );
}

/// Install Zig using a choice prompt.
pub async fn install_zig_interactive(options: Vec<InstallOption>) -> Result<()> {
    let choice = choose_option("Pick an option to install it:", options);

    match choice {
        Ok(choice) => choice.install().await.map(|_| ()),
        Err(err) => Err(err).into_diagnostic(),
    }
}

pub async fn check_installation() -> Result<ZigInfo> {
    if let Ok((path, run_modifiers)) = Zig::find_zig() {
        return get_zig_version(path, run_modifiers);
    }

    println!("Zig is not installed in your system.");
    let options = install_options();
    if options.is_empty() {
        println!(
            "Download Zig 0.13.0 or newer from https://ziglang.org/download/ and add it to your PATH"
        );
        return Err(BuildError::ZigMissing.into());
    }

    if options.len() == 1 {
        let Some(choice) = options.first().cloned() else {
            return Err(BuildError::ZigMissing.into());
        };

        choice.install().await?;
        get_zig_info()
    } else if is_stdin_tty() {
        install_zig_interactive(options).await?;
        get_zig_info()
    } else {
        print_install_options(&options);
        Err(BuildError::ZigMissing.into())
    }
}

pub fn get_zig_info() -> Result<ZigInfo> {
    let Ok((path, run_modifiers)) = Zig::find_zig() else {
        let options = install_options();
        return Ok(ZigInfo {
            install_options: Some(options),
            ..Default::default()
        });
    };

    get_zig_version(path, run_modifiers)
}

fn get_zig_version(
    path: PathBuf,
    run_modifiers: Vec<String>,
) -> std::result::Result<ZigInfo, miette::Error> {
    let mut cmd = Command::new(&path);
    cmd.args(&run_modifiers);
    cmd.arg("version");
    let output = cmd.output().into_diagnostic()?;
    let version = String::from_utf8(output.stdout)
        .into_diagnostic()?
        .trim()
        .to_string();

    Ok(ZigInfo {
        path: Some(path),
        version: Some(version),
        ..Default::default()
    })
}

#[derive(Clone, Debug)]
pub enum InstallOption {
    #[cfg(not(windows))]
    Brew,
    #[cfg(windows)]
    Choco,
    #[cfg(not(windows))]
    Nix,
    #[cfg(not(windows))]
    Npm,
    Pip3,
    #[cfg(windows)]
    Scoop,
    #[cfg(windows)]
    Winget,
}

impl serde::Serialize for InstallOption {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.usage())
    }
}

impl std::fmt::Display for InstallOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(not(windows))]
            InstallOption::Brew => write!(f, "Install with Homebrew"),
            #[cfg(windows)]
            InstallOption::Choco => write!(f, "Install with Chocolatey"),
            #[cfg(not(windows))]
            InstallOption::Nix => write!(f, "Install with Nix"),
            #[cfg(not(windows))]
            InstallOption::Npm => write!(f, "Install with NPM"),
            InstallOption::Pip3 => write!(f, "Install with Pip3 (Python 3)"),
            #[cfg(windows)]
            InstallOption::Scoop => write!(f, "Install with Scoop"),
            #[cfg(windows)]
            InstallOption::Winget => write!(f, "Install with Winget"),
        }
    }
}

impl InstallOption {
    pub fn usage(&self) -> &'static str {
        match self {
            #[cfg(not(windows))]
            InstallOption::Brew => "brew install zig",
            #[cfg(windows)]
            InstallOption::Choco => "choco install zig",
            #[cfg(not(windows))]
            InstallOption::Nix => "nix-env -iA nixpkgs.zig",
            #[cfg(not(windows))]
            InstallOption::Npm => "npm install -g @ziglang/cli",
            InstallOption::Pip3 => "pip3 install ziglang",
            #[cfg(windows)]
            InstallOption::Scoop => "scoop install zig",
            #[cfg(windows)]
            InstallOption::Winget => "winget install zig.zig",
        }
    }

    pub async fn install(self) -> Result<()> {
        let pb = Progress::start("Installing Zig...");
        let usage = self.usage().split(' ').collect::<Vec<_>>();
        let usage = usage.as_slice();
        let result = silent_command(usage[0], &usage[1..usage.len()]).await;

        let finish = if result.is_ok() {
            "Zig installed"
        } else {
            "Failed to install Zig"
        };
        pb.finish(finish);

        result
    }
}

pub fn install_options() -> Vec<InstallOption> {
    let mut options = Vec::new();

    #[cfg(not(windows))]
    if which::which("brew").is_ok() {
        options.push(InstallOption::Brew);
    }

    #[cfg(windows)]
    if which::which("choco").is_ok() {
        options.push(InstallOption::Choco);
    }

    #[cfg(not(windows))]
    if which::which("nix-env").is_ok() {
        options.push(InstallOption::Nix);
    }

    #[cfg(not(windows))]
    if which::which("npm").is_ok() {
        options.push(InstallOption::Npm);
    }

    if which::which("pip3").is_ok() {
        options.push(InstallOption::Pip3);
    }

    #[cfg(windows)]
    if which::which("scoop").is_ok() {
        options.push(InstallOption::Scoop);
    }

    #[cfg(windows)]
    if which::which("winget").is_ok() {
        options.push(InstallOption::Winget);
    }

    options
}
