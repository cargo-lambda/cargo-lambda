use cargo_lambda_interactive::{
    command::{new_command, silent_command},
    progress::Progress,
};
use miette::{IntoDiagnostic, Result, WrapErr};
use rustc_version::Channel;
use std::{env, str};

use crate::target_arch::TargetArch;

/// Check if the target component is installed in the host toolchain, and add
/// it with `rustup` as needed.
pub async fn check_target_component_with_rustc_meta(target_arch: &TargetArch) -> Result<()> {
    let component = &target_arch.rustc_target_without_glibc_version;

    // convert `Channel` enum to a lower-cased string representation
    let toolchain = match target_arch.channel()? {
        Channel::Stable => "stable",
        Channel::Nightly => "nightly",
        Channel::Dev => "dev",
        Channel::Beta => "beta",
    };

    let cmd = rustup_cmd();
    let args = [&format!("+{toolchain}"), "target", "list", "--installed"];

    tracing::trace!(
        cmd = ?cmd,
        args = ?args,
        target_arch = ?target_arch,
        "checking target toolchain installation"
    );

    let output = new_command(&cmd)
        .args(args)
        .output()
        .await
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to run `{} {}`", cmd, args.join(" ")))?;

    let out = str::from_utf8(&output.stdout)
        .into_diagnostic()
        .wrap_err("failed to read rustup output")?;
    let target_component_exists = out.lines().any(|line| line == component);

    tracing::trace!(target_component_exists, "completed target search");

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
    let cmd = rustup_cmd();
    let args = [&format!("+{toolchain}"), "target", "add", component];
    tracing::trace!(
        cmd = ?cmd,
        args = ?args,
        "installing target component"
    );

    silent_command(&cmd, &args).await
}

pub(crate) fn rustup_cmd() -> String {
    env::var("RUSTUP").unwrap_or_else(|_| "rustup".to_string())
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
