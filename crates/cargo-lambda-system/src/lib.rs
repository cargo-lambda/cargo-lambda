use clap::Args;
use miette::Result;

use cargo_lambda_build::{install_options, install_zig, print_install_options, Zig};
use cargo_lambda_interactive::is_stdin_tty;
use tracing::trace;

#[derive(Args, Clone, Debug)]
#[command(
    name = "system",
    after_help = "Full command documentation: https://www.cargo-lambda.info/commands/system.html"
)]
pub struct System {
    /// Setup and install Zig if it is not already installed.
    #[arg(long, visible_alias = "install")]
    setup: bool,
}

impl System {
    #[tracing::instrument(skip(self), target = "cargo_lambda")]
    pub async fn run(&self) -> Result<()> {
        trace!(options = ?self, "running system command");

        if let Ok((path, _)) = Zig::find_zig() {
            println!("Zig installation found at:");
            println!("{}", path.display());
        } else {
            let options = install_options();
            if self.setup && is_stdin_tty() {
                install_zig(options).await?;
            } else {
                print_install_options(&options);
            }
        }

        Ok(())
    }
}
