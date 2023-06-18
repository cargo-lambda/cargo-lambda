use super::{build_profile, Compiler};
use crate::TargetArch;
use cargo_options::Build;
use cargo_zigbuild::Build as ZigBuild;
use miette::Result;
use std::process::Command;

pub(crate) struct CargoZigbuild;

#[async_trait::async_trait]
impl Compiler for CargoZigbuild {
    async fn command(
        &self,
        cargo: &Build,
        target_arch: &TargetArch,
        skip_target_check: bool,
    ) -> Result<Command> {
        tracing::debug!("compiling with CargoZigbuild");
        crate::zig::check_installation().await?;

        // confirm that target component is included in host toolchain, or add
        // it with `rustup` otherwise.
        if !skip_target_check {
            crate::toolchain::check_target_component_with_rustc_meta(target_arch).await?;
        }

        #[allow(unused_mut)]
        let mut zig_build: ZigBuild = cargo.to_owned().into();

        #[cfg(windows)]
        // To understand why we need this,
        // see https://github.com/cargo-lambda/cargo-lambda/issues/77
        if !zig_build.release {
            tracing::info!("Changing profile to release mode. Cargo-lambda doesn't support building on debug mode on Windows");
            zig_build.release = true;
            zig_build.profile = Some("release".to_string());
        }

        zig_build.build_command().map_err(|e| miette::miette!(e))
    }

    #[cfg(windows)]
    fn build_profile<'a>(&self, cargo: &'a Build) -> &'a str {
        // To understand why we need this,
        // see https://github.com/cargo-lambda/cargo-lambda/issues/77
        if !cargo.release {
            "release"
        } else {
            build_profile(cargo.profile.as_deref(), cargo.release)
        }
    }

    #[cfg(not(windows))]
    fn build_profile<'a>(&self, cargo: &'a Build) -> &'a str {
        build_profile(cargo.profile.as_deref(), cargo.release)
    }
}
