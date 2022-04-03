use std::boxed::Box;

use cargo_lambda_build::{Build, Zig};
use cargo_lambda_invoke::Invoke;
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
    /// Build AWS Lambda functions compiled with zig as the linker
    Build(Box<Build>),
    /// Send requests to Lambda functions running on the emulator
    Invoke(Invoke),
    /// Start a Lambda Runtime emulator to test and debug functions locally
    Watch(Watch),
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::parse();
    match app {
        App::Lambda(lambda) => match *lambda {
            Lambda::Build(mut b) => b.run().await,
            Lambda::Invoke(i) => i.run().await,
            Lambda::Watch(s) => s.run().await,
        },
        App::Zig(zig) => zig.execute().map_err(|e| miette!(e)),
    }
}
