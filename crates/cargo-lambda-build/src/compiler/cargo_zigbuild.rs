use crate::TargetArch;
use cargo_lambda_metadata::cargo::CargoMetadata;
use cargo_options::Build;
use cargo_zigbuild::Build as ZigBuild;
use miette::Result;
use std::process::Command;

pub(crate) struct CargoZigbuild;

impl CargoZigbuild {
    #[tracing::instrument(target = "cargo_lambda")]
    pub(crate) async fn command(
        cargo: &Build,
        target_arch: &TargetArch,
        _cargo_metadata: &CargoMetadata,
        skip_target_check: bool,
    ) -> Result<Command> {
        tracing::debug!("compiling with CargoZigbuild");
        crate::zig::check_installation().await?;

        // confirm that target component is included in host toolchain, or add
        // it with `rustup` otherwise.
        if !skip_target_check {
            crate::toolchain::check_target_component_with_rustc_meta(target_arch).await?;
        }

        let zig_build: ZigBuild = cargo.to_owned().into();
        zig_build.build_command().map_err(|e| miette::miette!(e))
    }
}
