use cargo_lambda_metadata::cargo::CargoCompilerOptions;
use cargo_options::Build;
use miette::Result;
use std::{collections::VecDeque, env, ffi::OsStr, process::Command};

pub(crate) struct Cargo;

impl Cargo {
    #[tracing::instrument(target = "cargo_lambda")]
    pub(crate) async fn command(cargo: &Build, options: &CargoCompilerOptions) -> Result<Command> {
        tracing::debug!("compiling with Cargo");

        let (subcommand, extra_args) = cargo_subcommand(options);

        let mut cmd = if let Some(subcommand) = subcommand {
            let cmd = cargo.command();
            let mut args = cmd.get_args().collect::<VecDeque<&OsStr>>();
            // remove the `build` subcommand from the front.
            let _ = args.pop_front();

            let mut cmd = Command::new("cargo");
            cmd.args(subcommand);
            cmd.args(args);

            cmd
        } else {
            cargo.command()
        };

        if let Some(extra) = extra_args {
            cmd.args(extra);
        }
        Ok(cmd)
    }
}

fn cargo_subcommand(options: &CargoCompilerOptions) -> (Option<Vec<String>>, Option<Vec<String>>) {
    let subcommand = env::var("CARGO_LAMBDA_COMPILER_SUBCOMMAND")
        .map(|s: String| s.split(' ').map(String::from).collect())
        .ok()
        .or_else(|| options.subcommand.clone());

    let extra_args = env::var("CARGO_LAMBDA_COMPILER_EXTRA_ARGS")
        .map(|s: String| s.split(' ').map(String::from).collect())
        .ok()
        .or_else(|| options.extra_args.clone());

    (subcommand, extra_args)
}
