use super::Compiler;
use crate::TargetArch;
use cargo_lambda_metadata::cargo::CargoMetadata;
use cargo_options::Build;
use miette::Result;
use std::{collections::VecDeque, env, ffi::OsStr, fs, process::Command};

pub(crate) struct Cross;

#[async_trait::async_trait]
impl Compiler for Cross {
    #[tracing::instrument(skip(self), target = "cargo_lambda")]
    async fn command(
        &self,
        cargo: &Build,
        target_arch: &TargetArch,
        cargo_metadata: &CargoMetadata,
        _skip_target_check: bool,
    ) -> Result<Command> {
        tracing::debug!(?target_arch, "compiling with Cross");

        let cmd = cargo.command();
        let args = cmd.get_args().collect::<VecDeque<&OsStr>>();

        let mut cmd = Command::new("cross");
        cmd.args(args);

        if let Some((name, image)) = default_cross_image(
            target_arch.rustc_target_without_glibc_version(),
            cargo_metadata,
        ) {
            cmd.env(name, image);
        }

        Ok(cmd)
    }
}

fn default_cross_image(target: &str, metadata: &CargoMetadata) -> Option<(String, String)> {
    let env_name = format!(
        "CROSS_TARGET_{}_IMAGE",
        target.to_uppercase().replace('-', "_")
    );

    if is_build_image_configured(target, &env_name, metadata) {
        return None;
    }

    let env_value = format!("ghcr.io/cross-rs/{}:0.2.5", target);
    Some((env_name, env_value))
}

fn is_build_image_configured(target_arch: &str, env_name: &str, metadata: &CargoMetadata) -> bool {
    // Check for cross configuration in the package's Cargo.toml
    'outer: for pkg in &metadata.packages {
        for target in &pkg.targets {
            if target.kind.iter().any(|kind| kind == "bin") && pkg.metadata.is_object() {
                let Some(cross) = pkg.metadata.get("cross") else {
                    break 'outer;
                };
                let Some(t) = cross.get("target") else {
                    break 'outer;
                };
                let Some(arch) = t.get(target_arch) else {
                    break 'outer;
                };
                if arch.get("image").is_some() {
                    return true;
                }
            }
        }
    }

    // Check for cross configuration in the workspace's Cargo.toml
    if let Some(cross) = metadata.workspace_metadata.get("cross") {
        if let Some(target) = cross.get("target") {
            if let Some(arch) = target.get(target_arch) {
                if arch.get("image").is_some() {
                    return true;
                }
            }
        }
    }

    // Check for cross configuration in Cross.toml
    if let Ok(conf) = fs::read_to_string("Cross.toml") {
        if let Ok(cross) = toml::from_str::<toml::Value>(&conf) {
            if cross
                .get("target")
                .is_some_and(|t| t.get(target_arch).is_some_and(|t| t.get("image").is_some()))
            {
                return true;
            }
        }
    }

    // Check that the variable is not in the environment already
    env::var(env_name).is_ok()
}
