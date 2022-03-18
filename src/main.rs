use std::boxed::Box;

use cargo_zigbuild::Zig;
use clap::{Parser, Subcommand};
use miette::{miette, Result};

mod build;
mod invoke;
mod progress;
mod start;
mod zig;

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
    Build(Box<build::Build>),
    /// Send requests to Lambda functions running on the emulator
    Invoke(invoke::Invoke),
    /// Start a Lambda Runtime emulator to test and debug functions locally
    Start(start::Start),
}

#[tokio::main]
async fn main() -> Result<()> {
    let app = App::parse();
    match app {
        App::Lambda(lambda) => match *lambda {
            Lambda::Build(mut b) => b.run(),
            Lambda::Invoke(i) => i.run().await,
            Lambda::Start(s) => s.run().await,
        },
        App::Zig(zig) => zig.execute().map_err(|e| miette!(e)),
    }
}
