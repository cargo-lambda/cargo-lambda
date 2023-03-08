use cargo_lambda_interactive::{command::silent_command, progress::Progress};
use miette::{IntoDiagnostic, Result};
use rustc_version::Channel;
use std::env;

use crate::target_arch::TargetArch;

/// Check if the target component is installed in the host toolchain, and add
/// it with `rustup` as needed.
pub async fn check_target_component_with_rustc_meta(target_arch: &TargetArch) -> Result<()> {
    let component = &target_arch.rustc_target_without_glibc_version;

    // resolve $RUSTUP_HOME, which points to the `rustup` base directory
    // https://rust-lang.github.io/rustup/environment-variables.html#environment-variables
    let rustup_home = home::rustup_home().into_diagnostic()?;

    // convert `Channel` enum to a lower-cased string representation
    let toolchain = match target_arch.channel()? {
        Channel::Stable => "stable",
        Channel::Nightly => "nightly",
        Channel::Dev => "dev",
        Channel::Beta => "beta",
    };
    let toolchain_target = format!("{}-{}", toolchain, &target_arch.host);

    // check if the target component is installed in the host toolchain
    let target_component_exists = rustup_home
        .join("toolchains")
        .join(&toolchain_target)
        .join("lib")
        .join("rustlib")
        .join(component)
        .exists();

    tracing::trace!(
        toolchain_target,
        target_component_exists,
        rustup_home = ?rustup_home,
        target_arch = ?target_arch,
        "checking target toolchain installation"
    );

    if !target_component_exists {
        // install target component using `rustup`
        let pb = Progress::start(format_args!("Installing target component `{component}`..."));

        let result = install_target_component(component, toolchain).await;
        let finish = if result.is_ok() {
            "Target component installed"
        } else {
            "Failed to install target component"
        };

        pb.finish(finish);
    }

    Ok(())
}

/// Install target component in the host toolchain, using `rustup target add`
async fn install_target_component(component: &str, toolchain: &str) -> Result<()> {
    let cmd = env::var("RUSTUP").unwrap_or_else(|_| "rustup".to_string());
    let args = [&format!("+{toolchain}"), "target", "add", component];
    tracing::trace!(
        cmd = ?cmd,
        args = ?args,
        "installing target component"
    );

    silent_command(&cmd, &args).await
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    /// Check target component is installed in the host toolchain, and install
    /// it via `rustup target add` otherwise.
    ///
    /// # Note
    /// This test is marked as **ignored** so it doesn't add the target
    /// component in a CI build.
    #[tokio::test]
    #[ignore]
    async fn test_check_target_component() -> Result<()> {
        let component = "aarch64-unknown-linux-gnu";
        let arch = TargetArch::from_str(component)?;
        check_target_component_with_rustc_meta(&arch).await
    }
}
