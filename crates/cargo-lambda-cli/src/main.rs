#![warn(rust_2018_idioms, unused_lifetimes, clippy::multiple_crate_versions)]
use std::boxed::Box;

use cargo_lambda_build::{Build, Zig};
use cargo_lambda_deploy::Deploy;
use cargo_lambda_invoke::Invoke;
use cargo_lambda_new::New;
use cargo_lambda_watch::Watch;
use clap::{CommandFactory, Parser, Subcommand};
use miette::{miette, IntoDiagnostic, Result};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[clap(name = "cargo", bin_name = "cargo", disable_version_flag = true)]
enum App {
    Lambda(Lambda),
    #[clap(subcommand, hide = true)]
    Zig(Zig),
}

/// Cargo Lambda is a CLI to work with AWS Lambda functions locally
#[derive(Clone, Debug, clap::Args)]
struct Lambda {
    #[clap(subcommand)]
    subcommand: Option<Box<LambdaSubcommand>>,
    /// Enable trace logs in any subcommand
    #[clap(short, long, global = true)]
    verbose: bool,
    /// Print version information
    #[clap(short = 'V', long)]
    version: bool,
}

#[derive(Clone, Debug, Subcommand)]
enum LambdaSubcommand {
    /// Build Lambda functions compiled with zig as the linker
    Build(Box<Build>),
    /// Deploy Lambda functions to AWS
    Deploy(Deploy),
    /// Send requests to Lambda functions running on the emulator
    Invoke(Invoke),
    /// Create a new package with a Lambda function from our Lambda Template
    New(New),
    /// Start a Lambda Runtime emulator to test and debug functions locally
    Watch(Watch),
}

impl LambdaSubcommand {
    async fn run(self) -> Result<()> {
        match self {
            Self::Build(mut b) => b.run().await,
            Self::Deploy(d) => d.run().await,
            Self::Invoke(i) => i.run().await,
            Self::New(mut n) => n.run().await,
            Self::Watch(w) => w.run().await,
        }
    }
}

fn print_version() -> Result<()> {
    println!(
        "cargo-lambda {} {}",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_LAMBDA_BUILD_INFO")
    );
    Ok(())
}

fn print_help() -> Result<()> {
    let mut app = App::command();
    let lambda = app
        .find_subcommand_mut("lambda")
        .cloned()
        .map(|a| a.name("cargo lambda").bin_name("cargo lambda"));

    match lambda {
        Some(mut lambda) => lambda.print_help().into_diagnostic(),
        None => {
            println!("Run `cargo lambda --help` to see usage");
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::parse();

    let lambda = match app {
        App::Zig(zig) => return zig.execute().map_err(|e| miette!(e)),
        App::Lambda(lambda) => lambda,
    };

    if lambda.version {
        return print_version();
    }

    let subcommand = match lambda.subcommand {
        None => return print_help(),
        Some(subcommand) => subcommand,
    };

    let log_directive = if lambda.verbose {
        "cargo_lambda=trace".into()
    } else {
        std::env::var("RUST_LOG").unwrap_or_else(|_| "cargo_lambda=info".into())
    };

    let fmt = tracing_subscriber::fmt::layer()
        .with_target(false)
        .without_time();

    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(log_directive))
        .with(fmt);

    if let LambdaSubcommand::Watch(w) = &*subcommand {
        subscriber.with(w.xray_layer()).init();
    } else {
        subscriber.init();
    }

    subcommand.run().await
}
