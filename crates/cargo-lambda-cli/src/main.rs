#![warn(rust_2018_idioms, unused_lifetimes, clippy::multiple_crate_versions)]
use std::boxed::Box;

use cargo_lambda_build::{Build, Zig as ZigBuild};
use cargo_lambda_deploy::Deploy;
use cargo_lambda_invoke::{is_remote_invoke_err, Invoke};
use cargo_lambda_new::New;
use cargo_lambda_watch::Watch;
use clap::{CommandFactory, Parser, Subcommand};
use error_reporting::capture_error;
use miette::{miette, IntoDiagnostic, Result};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod error_reporting;
use error_reporting::*;
mod telemetry;
use telemetry::*;

#[derive(Parser)]
#[command(name = "cargo", bin_name = "cargo", disable_version_flag = true)]
enum App {
    Lambda(Lambda),
    #[command(hide = true)]
    Zig(Zig),
}

/// Cargo Lambda is a CLI to work with AWS Lambda functions locally
#[derive(Clone, Debug, Parser)]
struct Lambda {
    #[command(subcommand)]
    subcommand: Option<Box<LambdaSubcommand>>,
    /// Enable logs in any subcommand. Use `-v` for debug logs, and `-vv` for trace logs
    #[arg(short = 'v', long, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Print version information
    #[arg(short = 'V', long)]
    version: bool,
    /// Disable telemetry and error reporting
    #[clap(long, env = "DO_NOT_TRACK")]
    do_not_track: bool,
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

#[derive(Clone, Debug, Parser)]
struct Zig {
    #[clap(subcommand)]
    subcommand: ZigBuild,

    /// Disable telemetry and error reporting
    #[clap(long, env = "DO_NOT_TRACK")]
    do_not_track: bool,
}

impl Lambda {
    async fn run(self) -> Result<()> {
        if self.do_not_track && !is_do_not_track_enabled() {
            enable_do_not_track();
        }

        let tm_handle = send_telemetry_data();

        if self.version {
            return print_version();
        }

        let subcommand = match self.subcommand {
            None => return print_help(),
            Some(subcommand) => subcommand,
        };

        let log_directive = if self.verbose == 0 {
            std::env::var("RUST_LOG").unwrap_or_else(|_| "cargo_lambda=info".into())
        } else if self.verbose == 1 {
            "cargo_lambda=debug".into()
        } else {
            "cargo_lambda=trace".into()
        };

        let fmt = tracing_subscriber::fmt::layer()
            .with_target(false)
            .without_time();

        let subscriber = tracing_subscriber::registry()
            .with(sentry::integrations::tracing::layer())
            .with(tracing_subscriber::EnvFilter::new(log_directive))
            .with(fmt);

        if let LambdaSubcommand::Watch(w) = &*subcommand {
            subscriber.with(w.xray_layer()).init();
        } else {
            subscriber.init();
        }

        let res = subcommand.run().await;
        let _ = tm_handle.await;

        res
    }
}

impl Zig {
    fn run(self) -> Result<()> {
        if self.do_not_track && !is_do_not_track_enabled() {
            enable_do_not_track();
        }
        self.subcommand.execute().map_err(|e| miette!(e))
    }
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
    println!("cargo-lambda {}", version());
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

async fn run_command() -> Result<()> {
    let app = App::parse();

    match app {
        App::Zig(zig) => zig.run(),
        App::Lambda(lambda) => lambda.run().await,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let _sentry = sentry::init((
        SENTRY_DSN,
        sentry::ClientOptions {
            release: Some(version().into()),
            traces_sample_rate: 1.0,
            ..Default::default()
        },
    ));

    let res = run_command().await;

    if let Err(err) = res.as_ref() {
        if !is_do_not_track_enabled() && !is_remote_invoke_err(err) {
            capture_error(err);
        }
    }

    res
}
