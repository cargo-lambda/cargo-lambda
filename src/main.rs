use atty::is;
use cargo_zigbuild::{Build, Zig};
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use miette::{miette, IntoDiagnostic, Result, WrapErr};
use std::boxed::Box;
use std::path::Path;
use std::process::{Command, Stdio};

#[derive(Parser)]
#[clap(name = "cargo")]
#[clap(bin_name = "cargo")]
#[clap(version, global_setting(clap::AppSettings::DeriveDisplayOrder))]
enum App {
    #[clap(subcommand)]
    Lambda(Box<Lambda>),
    #[clap(subcommand, hide = true)]
    Zig(Zig),
}

/// Build AWS Lambda functions compiled with zig as the linker
#[derive(Clone, Debug, Subcommand)]
pub enum Lambda {
    Build(Build),
}

fn main() -> Result<()> {
    let app = App::parse();
    match app {
        App::Lambda(lambda) => match *lambda {
            Lambda::Build(mut b) => run_build(&mut b),
        },
        App::Zig(zig) => zig.execute().map_err(|e| miette!(e)),
    }
}

fn run_build(build: &mut Build) -> Result<()> {
    let rustc_meta = rustc_version::version_meta().into_diagnostic()?;
    let host_target = &rustc_meta.host;

    match build.target.as_ref() {
        // Same explicit target as host target
        Some(target) if host_target == target => build.disable_zig_linker = true,
        // No explicit target, but build host same as target host
        None if host_target == "aarch64-unknown-linux-gnu"
            || host_target == "x86_64-unknown-linux-gnu" =>
        {
            build.disable_zig_linker = true;
            // Set the target explicitly, so it's easier to find the binaries later
            build.target = Some(host_target.into());
        }
        // No explicit target, and build host not compatible with Lambda hosts
        None => {
            build.target = Some("x86_64-unknown-linux-gnu".into());
        }
        _ => {}
    }

    if !build.disable_zig_linker {
        check_zig_installation()?;
    }

    let mut cmd = build
        .build_command("build")
        .map_err(|e| miette::miette!("{}", e))?;
    if build.release {
        cmd.env("RUSTFLAGS", "-C strip=symbols");
    }

    let mut child = cmd
        .spawn()
        .into_diagnostic()
        .wrap_err("Failed to run cargo build")?;
    let status = child
        .wait()
        .into_diagnostic()
        .wrap_err("Failed to wait on cargo build process")?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    let manifest_path = build
        .manifest_path
        .as_deref()
        .unwrap_or_else(|| Path::new("Cargo.toml"));
    let mut metadata_cmd = cargo_metadata::MetadataCommand::new();
    metadata_cmd.no_deps();
    metadata_cmd.manifest_path(&manifest_path);
    let metadata = metadata_cmd.exec().into_diagnostic()?;

    let mut binaries: Vec<String> = Vec::new();
    for pkg in metadata.packages {
        for target in pkg.targets {
            if target.kind.iter().any(|s| s == "bin") {
                binaries.push(target.name);
            }
        }
    }

    let final_target = build
        .target
        .as_deref()
        .unwrap_or("x86_64-unknown-linux-gnu");
    let profile = match build.profile.as_deref() {
        Some("dev" | "test") => "debug",
        Some("release" | "bench") => "release",
        Some(profile) => profile,
        None if build.release => "release",
        None => "debug",
    };

    let target = Path::new("target");
    let base = target.join(final_target).join(profile);
    let build = target.join("lambda");

    for name in &binaries {
        let binary = base.join(name);
        if binary.exists() {
            let bootstrap_dir = build.join(name);
            std::fs::create_dir_all(&bootstrap_dir).into_diagnostic()?;
            std::fs::rename(binary, bootstrap_dir.join("bootstrap")).into_diagnostic()?;
        }
    }

    Ok(())
}

fn check_zig_installation() -> Result<()> {
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
        let pb = Progress::start("Installing Zig...");
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

struct Progress {
    bar: Option<ProgressBar>,
}

impl Progress {
    fn start(msg: &str) -> Progress {
        let bar = if is(atty::Stream::Stdout) {
            Some(show_progress(msg))
        } else {
            println!("▹▹▹▹▹ {}", msg);
            None
        };
        Progress { bar }
    }

    fn finish(&self, msg: &str) {
        if let Some(bar) = &self.bar {
            bar.finish_with_message(msg.to_string());
        } else {
            println!("▪▪▪▪▪ {}", msg);
        }
    }
}

fn show_progress(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(120);
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.blue} {msg}")
            .tick_strings(&[
                "▹▹▹▹▹",
                "▸▹▹▹▹",
                "▹▸▹▹▹",
                "▹▹▸▹▹",
                "▹▹▹▸▹",
                "▹▹▹▹▸",
                "▪▪▪▪▪",
            ]),
    );
    pb.set_message(msg.to_string());
    pb
}
