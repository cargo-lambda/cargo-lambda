use cargo_lambda_interactive::{
    choose_option, command::silent_command, is_stdin_tty, progress::Progress,
};
use cargo_zigbuild::Zig;
use miette::{IntoDiagnostic, Result};

pub async fn check_installation() -> Result<()> {
    if Zig::find_zig().is_ok() {
        return Ok(());
    }

    let options = install_options();

    if !is_stdin_tty() || options.is_empty() {
        println!("Zig is not installed in your system.");
        if !options.is_empty() {
            println!("You can use any of the following options to install it:");
            for option in &options {
                println!("\t* {}: `{}`", option, option.usage());
            }
        }
        println!("\t* Download Zig 0.9.1 or newer from https://ziglang.org/download/ and add it to your PATH");
        return Err(miette::miette!("Install Zig and run cargo-lambda again"));
    }

    let choice = choose_option(
        "Zig is not installed in your system.\nHow do you want to install Zig?",
        options,
    )
    .into_diagnostic()?;

    choice.install().await
}

enum InstallOption {
    #[cfg(not(windows))]
    Brew,
    #[cfg(windows)]
    Choco,
    #[cfg(not(windows))]
    Npm,
    Pip3,
    #[cfg(windows)]
    Scoop,
}

impl std::fmt::Display for InstallOption {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            #[cfg(not(windows))]
            InstallOption::Brew => write!(f, "Install with Homebrew"),
            #[cfg(windows)]
            InstallOption::Choco => write!(f, "Install with Chocolatey"),
            #[cfg(not(windows))]
            InstallOption::Npm => write!(f, "Install with NPM"),
            InstallOption::Pip3 => write!(f, "Install with Pip3 (Python 3)"),
            #[cfg(windows)]
            InstallOption::Scoop => write!(f, "Install with Scoop"),
        }
    }
}

impl InstallOption {
    fn usage(&self) -> &'static str {
        match self {
            #[cfg(not(windows))]
            InstallOption::Brew => "brew install zig",
            #[cfg(windows)]
            InstallOption::Choco => "choco install zig",
            #[cfg(not(windows))]
            InstallOption::Npm => "npm install -g @ziglang/cli",
            InstallOption::Pip3 => "pip3 install ziglang",
            #[cfg(windows)]
            InstallOption::Scoop => "scoop install zig",
        }
    }

    async fn install(self) -> Result<()> {
        let pb = Progress::start("Installing Zig...");
        let result = match self {
            #[cfg(not(windows))]
            InstallOption::Brew => silent_command("brew", &["install", "zig"]).await,
            #[cfg(windows)]
            InstallOption::Choco => silent_command("choco", &["install", "zig"]).await,
            #[cfg(not(windows))]
            InstallOption::Npm => silent_command("npm", &["install", "-g", "@ziglang/cli"]).await,
            InstallOption::Pip3 => silent_command("pip3", &["install", "ziglang"]).await,
            #[cfg(windows)]
            InstallOption::Scoop => silent_command("scoop", &["install", "zig"]).await,
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

fn install_options() -> Vec<InstallOption> {
    let mut options = Vec::new();

    #[cfg(not(windows))]
    if which::which("brew").is_ok() {
        options.push(InstallOption::Brew);
    }

    #[cfg(windows)]
    if which::which("choco").is_ok() {
        options.push(InstallOption::Choco);
    }

    #[cfg(windows)]
    if which::which("scoop").is_ok() {
        options.push(InstallOption::Scoop);
    }

    if which::which("pip3").is_ok() {
        options.push(InstallOption::Pip3);
    }

    #[cfg(not(windows))]
    if which::which("npm").is_ok() {
        options.push(InstallOption::Npm);
    }
    options
}
