use std::boxed::Box;

use cargo_lambda_build::{Build, Zig};
use cargo_lambda_deploy::Deploy;
use cargo_lambda_invoke::Invoke;
use cargo_lambda_new::New;
use cargo_lambda_watch::Watch;
use clap::{Parser, Subcommand};
use miette::{miette, Result};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser)]
#[clap(name = "cargo")]
#[clap(bin_name = "cargo")]
#[clap(global_setting(clap::AppSettings::DeriveDisplayOrder))]
enum App {
    Lambda(Lambda),
    #[clap(subcommand, hide = true)]
    Zig(Zig),
}

/// Cargo Lambda is a CLI to work with AWS Lambda functions locally
#[derive(Clone, Debug, clap::Args)]
#[clap(version)]
struct Lambda {
    #[clap(subcommand)]
    subcommand: Box<LambdaSubcommand>,
    /// Enable trace logs in any subcommand
    #[clap(short, long, global = true)]
    verbose: bool,
}

#[derive(Clone, Debug, Subcommand)]
#[clap(version)]
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

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::parse();

    let lambda = match app {
        App::Zig(zig) => return zig.execute().map_err(|e| miette!(e)),
        App::Lambda(lambda) => lambda,
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

    if let LambdaSubcommand::Watch(w) = &*lambda.subcommand {
        subscriber.with(w.xray_layer()).init();
    } else {
        subscriber.init();
    }

    lambda.subcommand.run().await
}
