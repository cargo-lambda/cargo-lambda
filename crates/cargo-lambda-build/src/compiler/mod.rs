use crate::TargetArch;
use cargo_lambda_metadata::cargo::CompilerOptions;
use cargo_options::Build;
use miette::Result;
use rustc_version::VersionMeta;
use std::process::Command;

mod cargo;
use cargo::Cargo;
mod cargo_zigbuild;
use self::cargo_zigbuild::CargoZigbuild;

#[async_trait::async_trait]
pub(crate) trait Compiler {
    async fn command(
        &self,
        cargo: &Build,
        rustc_meta: &VersionMeta,
        target_arch: &TargetArch,
    ) -> Result<Command>;
}

pub(crate) fn new_compiler(compiler: CompilerOptions) -> Box<dyn Compiler> {
    match compiler {
        CompilerOptions::CargoZigbuild => Box::new(CargoZigbuild),
        CompilerOptions::Cargo(opts) => Box::new(Cargo::new(opts)),
    }
}
