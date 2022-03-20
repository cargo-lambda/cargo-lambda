use cargo_zigbuild::Zig;
use miette::{IntoDiagnostic, Result, WrapErr};
use std::process::{Command, Stdio};

pub fn check_installation() -> Result<()> {
    if Zig::find_zig().is_ok() {
        return Ok(());
    }

    if atty::isnt(atty::Stream::Stdin) {
        println!("Zig is not installed in your system.\nYou can use any of the following options to install it:");
        println!("\t* pip3 install ziglang (Python 3 required)");
        println!("\t* npm install -g @ziglang/cli (NPM required)");
        println!("\t* Download a recent version from https://ziglang.org/download/ and add it to your PATH");
        return Err(miette::miette!("Install Zig and run cargo-lambda again"));
    }

    let options = vec![InstallOption::Pip3, InstallOption::Npm];
    let choice = inquire::Select::new(
        "Zig is not installed in your system.\nHow do you want to install Zig?",
        options,
    )
    .with_vim_mode(true)
    .with_help_message("Press Ctrl+C to abort and exit cargo-lambda")
    .prompt()
    .into_diagnostic()?;

    choice.install()
}

enum InstallOption {
    Pip3,
    Npm,
}

impl std::fmt::Display for InstallOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InstallOption::Pip3 => write!(f, "Install with Pip3 (Python 3)"),
            InstallOption::Npm => write!(f, "Install with NPM"),
        }
    }
}

impl InstallOption {
    fn install(self) -> Result<()> {
        let pb = crate::progress::Progress::start("Installing Zig...");
        let result = match self {
            InstallOption::Pip3 => install_with_pip3(),
            InstallOption::Npm => install_with_npm(),
        };
        let finish = if result.is_ok() {
            "Zig installed"
        } else {
            "Failed to install Zig"
        };
        pb.finish(finish);

        result
    }
}

fn install_with_pip3() -> Result<()> {
    let mut child = Command::new("pip3")
        .args(&["install", "ziglang"])
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .into_diagnostic()
        .wrap_err("Failed to run `pip3 install ziglang`")?;

    let status = child
        .wait()
        .into_diagnostic()
        .wrap_err("Failed to wait on pip3 process")?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}

fn install_with_npm() -> Result<()> {
    let mut child = Command::new("npm")
        .args(&["install", "-g", "@ziglang/cli"])
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .into_diagnostic()
        .wrap_err("Failed to run `npm install @ziglang/cli`")?;

    let status = child
        .wait()
        .into_diagnostic()
        .wrap_err("Failed to wait on npm process")?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
