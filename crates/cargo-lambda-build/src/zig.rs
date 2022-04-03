use cargo_lambda_interactive::{
    choose_option, command::silent_command, is_stdin_tty, progress::Progress,
};
use cargo_zigbuild::Zig;
use miette::{IntoDiagnostic, Result};

pub async fn check_installation() -> Result<()> {
    if Zig::find_zig().is_ok() {
        return Ok(());
    }

    if !is_stdin_tty() {
        println!("Zig is not installed in your system.\nYou can use any of the following options to install it:");
        println!("\t* pip3 install ziglang (Python 3 required)");
        println!("\t* npm install -g @ziglang/cli (NPM required)");
        println!("\t* Download a recent version from https://ziglang.org/download/ and add it to your PATH");
        return Err(miette::miette!("Install Zig and run cargo-lambda again"));
    }

    let options = vec![InstallOption::Pip3, InstallOption::Npm];
    let choice = choose_option(
        "Zig is not installed in your system.\nHow do you want to install Zig?",
        options,
    )
    .into_diagnostic()?;

    choice.install().await
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
    async fn install(self) -> Result<()> {
        let pb = Progress::start("Installing Zig...");
        let result = match self {
            InstallOption::Pip3 => silent_command("pip3", &["install", "ziglang"]).await,
            InstallOption::Npm => silent_command("npm", &["install", "-g", "@ziglang/cli"]).await,
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
