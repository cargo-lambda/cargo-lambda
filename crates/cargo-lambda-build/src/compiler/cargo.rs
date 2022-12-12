use super::{build_profile, Compiler};
use crate::TargetArch;
use cargo_lambda_metadata::cargo::CargoCompilerOptions;
use cargo_options::Build;
use miette::Result;
use rustc_version::VersionMeta;
use std::{collections::VecDeque, env, ffi::OsStr, process::Command};

pub(crate) struct Cargo {
    compiler: CargoCompilerOptions,
}

impl Cargo {
    pub fn new(compiler: CargoCompilerOptions) -> Self {
        let mut cargo = Cargo { compiler };

        if let Ok(subcommand) = env::var("CARGO_LAMBDA_COMPILER_SUBCOMMAND") {
            cargo.compiler.subcommand = Some(subcommand.split(' ').map(String::from).collect());
        }

        if let Ok(extra_args) = env::var("CARGO_LAMBDA_COMPILER_EXTRA_ARGS") {
            cargo.compiler.extra_args = Some(extra_args.split(' ').map(String::from).collect());
        }

        cargo
    }
}

#[async_trait::async_trait]
impl Compiler for Cargo {
    async fn command(
        &self,
        cargo: &Build,
        _rustc_meta: &VersionMeta,
        _target_arch: &TargetArch,
    ) -> Result<Command> {
        tracing::debug!("compiling with Cargo");

        let mut cmd = if let Some(subcommand) = &self.compiler.subcommand {
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

        if let Some(extra) = &self.compiler.extra_args {
            cmd.args(extra);
        }
        Ok(cmd)
    }

    fn build_profile<'a>(&self, cargo: &'a Build) -> &'a str {
        build_profile(cargo.profile.as_deref(), cargo.release)
    }
}
