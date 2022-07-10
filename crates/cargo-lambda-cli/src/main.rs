use std::boxed::Box;

use cargo_lambda_build::{Build, Zig};
use cargo_lambda_deploy::Deploy;
use cargo_lambda_invoke::Invoke;
use cargo_lambda_new::New;
use cargo_lambda_watch::Watch;
use clap::{Parser, Subcommand};
use miette::{miette, Result};

#[derive(Parser)]
#[clap(name = "cargo")]
#[clap(bin_name = "cargo")]
#[clap(global_setting(clap::AppSettings::DeriveDisplayOrder))]
enum App {
    #[clap(subcommand)]
    Lambda(Box<Lambda>),
    #[clap(subcommand, hide = true)]
    Zig(Zig),
}

/// Cargo Lambda is a CLI to work with AWS Lambda functions locally
#[derive(Clone, Debug, Subcommand)]
#[clap(version)]
pub enum Lambda {
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

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::parse();

    match app {
        App::Lambda(lambda) => match *lambda {
            Lambda::Build(mut b) => b.run().await,
            Lambda::Deploy(d) => d.run().await,
            Lambda::Invoke(i) => i.run().await,
            Lambda::New(mut n) => n.run().await,
            Lambda::Watch(w) => w.run().await,
        },
        App::Zig(zig) => zig.execute().map_err(|e| miette!(e)),
    }
}
