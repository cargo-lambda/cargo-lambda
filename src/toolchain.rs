use std::env;
use std::ffi::OsString;
use std::process::{Command, Stdio};

use miette::{IntoDiagnostic, Result, WrapErr};
use rustc_version::Channel;

/// Check if the target component is installed in the host toolchain, and add
/// it with `rustup` as needed.
///
/// # Note
/// This function calls `rustc -vV` to retrieve the host triple and the release
/// channel name.
#[allow(unused)]
pub fn check_target_component(component: &str) -> Result<()> {
    let rustc_meta = rustc_version::version_meta().into_diagnostic()?;
    let host_target = &rustc_meta.host;
    let release_channel = &rustc_meta.channel;

    check_target_component_with_rustc_meta(component, host_target, release_channel)
}

/// Check if the target component is installed in the host toolchain, and add
/// it with `rustup` as needed.
pub fn check_target_component_with_rustc_meta(
    component: &str,
    host: &str,
    channel: &Channel,
) -> Result<()> {
    // resolve $RUSTUP_HOME, which points to the `rustup` base directory
    // https://rust-lang.github.io/rustup/environment-variables.html#environment-variables
    let rustup_home = home::rustup_home().into_diagnostic()?;

    // convert `Channel` enum to a lower-cased string representation
    let channel_name = match channel {
        Channel::Stable => "stable",
        Channel::Nightly => "nightly",
        Channel::Dev => "dev",
        Channel::Beta => "beta",
    };

    // check if the target component is installed in the host toolchain
    let target_component_is_added = rustup_home
        .join("toolchains")
        .join(format!("{channel_name}-{host}"))
        .join("lib")
        .join("rustlib")
        .join(component)
        .exists();

    if !target_component_is_added {
        // install target component using `rustup`
        let pb = crate::progress::Progress::start(format_args!(
            "Installing target component `{}`...",
            component
        ));

        let result = install_target_component(component);
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
fn install_target_component(component: &str) -> Result<()> {
    let cmd = env::var_os("RUSTUP").unwrap_or_else(|| OsString::from("rustup"));

    let mut child = Command::new(&cmd)
        .args(&["target", "add", component])
        .stderr(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .into_diagnostic()
        .wrap_err_with(|| format!("Failed to run `{:?} target add {}`", cmd, component))?;

    let status = child
        .wait()
        .into_diagnostic()
        .wrap_err("Failed to wait on rustup process")?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
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
    #[test]
    #[ignore]
    fn test_check_target_component() -> crate::Result<()> {
        let component = "aarch64-unknown-linux-gnu";
        check_target_component(component)
    }
}
