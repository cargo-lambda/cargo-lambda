use std::boxed::Box;

use cargo_zigbuild::Zig;
use clap::{Parser, Subcommand};
use miette::{miette, Result};

mod build;
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

/// Build AWS Lambda functions compiled with zig as the linker
#[derive(Clone, Debug, Subcommand)]
#[clap(version)]
pub enum Lambda {
    Build(build::Build),
}

fn main() -> Result<()> {
    let app = App::parse();
    match app {
        App::Lambda(lambda) => match *lambda {
            Lambda::Build(mut b) => b.run(),
        },
        App::Zig(zig) => zig.execute().map_err(|e| miette!(e)),
    }
}
