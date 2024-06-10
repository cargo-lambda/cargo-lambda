use crate::TargetArch;
use cargo_lambda_metadata::cargo::{CargoMetadata, CompilerOptions};
use cargo_options::Build;
use miette::Result;
use std::process::Command;

mod cargo;
use cargo::Cargo;
mod cargo_zigbuild;
use cargo_zigbuild::CargoZigbuild;
mod cross;
use cross::Cross;

pub(crate) async fn build_command(
    compiler: &CompilerOptions,
    cargo: &Build,
    target_arch: &TargetArch,
    cargo_metadata: &CargoMetadata,
    skip_target_check: bool,
) -> Result<Command> {
    match compiler {
        CompilerOptions::CargoZigbuild => {
            CargoZigbuild::command(cargo, target_arch, cargo_metadata, skip_target_check).await
        }
        CompilerOptions::Cargo(opts) => Cargo::command(cargo, opts).await,
        CompilerOptions::Cross => Cross::command(cargo, target_arch, cargo_metadata).await,
    }
}

#[allow(unused_variables)]
pub(crate) fn build_profile<'a>(cargo: &'a Build, compiler: &'a CompilerOptions) -> &'a str {
    match cargo.profile.as_deref() {
        Some("dev" | "test") => "debug",
        Some("release" | "bench") => "release",
        Some(profile) => profile,
        None if cargo.release => "release",
        None => "debug",
    }
}
