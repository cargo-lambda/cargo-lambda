use crate::TargetArch;
use cargo_lambda_metadata::cargo::{CargoMetadata, CompilerOptions};
use cargo_options::Build;
use miette::Result;
use std::process::Command;

mod cargo;
use cargo::Cargo;
mod cargo_zigbuild;
use self::cargo_zigbuild::CargoZigbuild;
mod cross;
use cross::Cross;

#[async_trait::async_trait]
pub(crate) trait Compiler {
    async fn command(
        &self,
        cargo: &Build,
        target_arch: &TargetArch,
        cargo_metadata: &CargoMetadata,
        skip_target_check: bool,
    ) -> Result<Command>;

    fn build_profile<'a>(&self, cargo: &'a Build) -> &'a str {
        build_profile(cargo.profile.as_deref(), cargo.release)
    }
}

#[tracing::instrument(target = "cargo_lambda")]
pub(crate) fn new_compiler(compiler: CompilerOptions) -> Box<dyn Compiler> {
    tracing::trace!("initializing Lambda compiler");
    match compiler {
        CompilerOptions::CargoZigbuild => Box::new(CargoZigbuild),
        CompilerOptions::Cargo(opts) => Box::new(Cargo::new(opts)),
        CompilerOptions::Cross => Box::new(Cross),
    }
}

pub fn build_profile(profile: Option<&str>, release: bool) -> &str {
    match profile {
        Some("dev" | "test") => "debug",
        Some("release" | "bench") => "release",
        Some(profile) => profile,
        None if release => "release",
        None => "debug",
    }
}
