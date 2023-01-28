use cargo_lambda_interactive::{command::silent_command, progress::Progress};
use miette::{IntoDiagnostic, Result};
use rustc_version::Channel;
use std::env;

/// Check if the target component is installed in the host toolchain, and add
/// it with `rustup` as needed.
///
/// # Note
/// This function calls `rustc -vV` to retrieve the host triple and the release
/// channel name.
#[allow(unused)]
pub async fn check_target_component(component: &str) -> Result<()> {
    let rustc_meta = rustc_version::version_meta().into_diagnostic()?;
    let host_target = &rustc_meta.host;
    let release_channel = &rustc_meta.channel;

    check_target_component_with_rustc_meta(component, host_target, release_channel).await
}

/// Check if the target component is installed in the host toolchain, and add
/// it with `rustup` as needed.
pub async fn check_target_component_with_rustc_meta(
    component: &str,
    host: &str,
    channel: &Channel,
) -> Result<()> {
    // resolve $RUSTUP_HOME, which points to the `rustup` base directory
    // https://rust-lang.github.io/rustup/environment-variables.html#environment-variables
    let rustup_home = home::rustup_home().into_diagnostic()?;

    // convert `Channel` enum to a lower-cased string representation
    let toolchain = match channel {
        Channel::Stable => "stable",
        Channel::Nightly => "nightly",
        Channel::Dev => "dev",
        Channel::Beta => "beta",
    };

    // check if the target component is installed in the host toolchain
    let target_component_is_added = rustup_home
        .join("toolchains")
        .join(format!("{toolchain}-{host}"))
        .join("lib")
        .join("rustlib")
        .join(component)
        .exists();

    if !target_component_is_added {
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

    silent_command(
        &cmd,
        &[&format!("+{toolchain}"), "target", "add", component],
    )
    .await
}

#[cfg(test)]
mod tests {
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
        check_target_component(component).await
    }
}
