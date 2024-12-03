use std::{fmt::Display, str::FromStr};

use miette::{Context, IntoDiagnostic, Result};
use rustc_version::Channel;

use crate::error::BuildError;

const TARGET_ARM: &str = "aarch64-unknown-linux-gnu";
const TARGET_X86_64: &str = "x86_64-unknown-linux-gnu";

#[derive(Debug, Default, PartialEq)]
pub enum Arch {
    #[default]
    X86_64,
    ARM64,
}

impl Arch {
    fn target_cpu(&self) -> &'static str {
        match self {
            Arch::ARM64 => "neoverse-n1",
            Arch::X86_64 => "haswell",
        }
    }
}

#[derive(Debug)]
pub struct TargetArch {
    rustc_target: String,
    channel: Option<Channel>,
}

impl TargetArch {
    pub fn arm64() -> Self {
        Self {
            rustc_target: TARGET_ARM.into(),
            channel: None,
        }
    }

    pub fn x86_64() -> Self {
        Self {
            rustc_target: TARGET_X86_64.into(),
            channel: None,
        }
    }

    pub fn from_host() -> Result<Self> {
        let rustc_meta = rustc_version::version_meta()
            .into_diagnostic()
            .wrap_err("error reading Rust Metadata information")?;
        let mut target = TargetArch::from_str(&rustc_meta.host)?;
        if !target.compatible_host_linker() {
            target = TargetArch::x86_64();
        }
        target.channel = Some(rustc_meta.channel);
        Ok(target)
    }

    pub fn arch(&self) -> Arch {
        if self.rustc_target.starts_with("aarch64-unknown-linux") {
            Arch::ARM64
        } else {
            Arch::X86_64
        }
    }

    pub fn target_cpu(&self) -> &'static str {
        self.arch().target_cpu()
    }

    pub fn compatible_host_linker(&self) -> bool {
        let target = self.rustc_target_without_glibc_version();
        target == TARGET_ARM || target == TARGET_X86_64
    }

    #[cfg(target_os = "linux")]
    pub fn is_static_linking(&self) -> bool {
        self.rustc_target == "x86_64-unknown-linux-musl"
            || self.rustc_target == "aarch64-unknown-linux-musl"
    }

    #[cfg(not(target_os = "linux"))]
    pub fn is_static_linking(&self) -> bool {
        false
    }

    pub fn channel(&self) -> Result<Channel> {
        match self.channel {
            Some(c) => Ok(c),
            None => rustc_version::version_meta()
                .map(|m| m.channel)
                .into_diagnostic()
                .wrap_err("error reading Rust version information"),
        }
    }

    pub fn rustc_target_without_glibc_version(&self) -> &str {
        let Some((rustc_target_without_glibc_version, _)) = self.rustc_target.split_once('.')
        else {
            return self.rustc_target.as_str();
        };
        rustc_target_without_glibc_version
    }
}

impl Display for TargetArch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.rustc_target)?;
        Ok(())
    }
}

impl FromStr for TargetArch {
    type Err = miette::Report;

    fn from_str(host: &str) -> Result<Self, Self::Err> {
        Ok(Self {
            rustc_target: host.into(),
            channel: None,
        })
    }
}

/// Validate that the build target is supported in AWS Lambda.
///
/// Here we use *starts with* instead of an exact match because:
///   - the target could also also be a *musl* variant: `x86_64-unknown-linux-musl`
///   - the target could also [specify a glibc version], which `cargo-zigbuild` supports
///
/// [specify a glibc version]: https://github.com/messense/cargo-zigbuild#specify-glibc-version
pub(crate) fn validate_linux_target(target: &str) -> Result<()> {
    if target.starts_with("aarch64-unknown-linux") || target.starts_with("x86_64-unknown-linux") {
        Ok(())
    } else {
        Err(BuildError::UnsupportedTarget(target.into()).into())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_target_arch_from_str() {
        let t = TargetArch::from_str("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!("x86_64-unknown-linux-gnu", t.to_string().as_str());
    }

    #[test]
    fn test_target_arch_arch() {
        let t = TargetArch::from_str("x86_64-unknown-linux-gnu.2.27").unwrap();
        assert_eq!(Arch::X86_64, t.arch());

        let t = TargetArch::from_str("aarch64-unknown-linux-gnu.2.27").unwrap();
        assert_eq!(Arch::ARM64, t.arch());
    }

    #[test]
    fn test_validate_linux_target() {
        let res = validate_linux_target("x86_64-unknown-linux-gnu.2.27");
        assert!(res.is_ok());

        let res = validate_linux_target("aarch64-unknown-linux-gnu.2.27");
        assert!(res.is_ok());

        let err = validate_linux_target("aarch64-unknown-darwin").unwrap_err();
        assert_eq!(
            "invalid or unsupported target for AWS Lambda: aarch64-unknown-darwin",
            err.to_string()
        );
    }

    #[test]
    fn test_compatible_host_linker() {
        assert!(TargetArch::from_str("x86_64-unknown-linux-gnu")
            .unwrap()
            .compatible_host_linker());
        assert!(TargetArch::from_str("aarch64-unknown-linux-gnu")
            .unwrap()
            .compatible_host_linker());
        assert!(!TargetArch::from_str("x86_64-pc-windows-gnu")
            .unwrap()
            .compatible_host_linker());
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn test_is_static_linking() {
        assert!(TargetArch::from_str("x86_64-unknown-linux-musl")
            .unwrap()
            .is_static_linking());
        assert!(TargetArch::from_str("aarch64-unknown-linux-musl")
            .unwrap()
            .is_static_linking());
    }

    #[test]
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    fn test_is_static_linking() {
        assert!(TargetArch::from_str("aarch64-unknown-linux-musl")
            .unwrap()
            .is_static_linking());
        assert!(TargetArch::from_str("x86_64-unknown-linux-musl")
            .unwrap()
            .is_static_linking());
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn test_is_static_linking() {
        assert!(!TargetArch::from_str("aarch64-unknown-linux-musl")
            .unwrap()
            .is_static_linking());
        assert!(!TargetArch::from_str("x86_64-unknown-linux-musl")
            .unwrap()
            .is_static_linking());
    }
}
