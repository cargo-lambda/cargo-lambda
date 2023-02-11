use crate::error::BuildError;
use miette::{IntoDiagnostic, Result};
use rustc_version::Channel;
use std::{fmt::Display, str::FromStr};

const TARGET_ARM: &str = "aarch64-unknown-linux-gnu";
const TARGET_X86_64: &str = "x86_64-unknown-linux-gnu";
const AL2_GLIBC: &str = "2.26";

#[derive(Debug, Default, PartialEq)]
enum Arch {
    #[default]
    X86_64,
    ARM64,
}

#[derive(Debug, Default)]
pub struct TargetArch {
    pub host: String,
    arch: Arch,
    pub rustc_target_without_glibc_version: String,
    glibc_version: Option<String>,
    channel: Option<Channel>,
}

impl TargetArch {
    pub fn arm64() -> Self {
        Self {
            host: TARGET_ARM.into(),
            rustc_target_without_glibc_version: TARGET_ARM.into(),
            arch: Arch::ARM64,
            ..Default::default()
        }
    }

    pub fn x86_64() -> Self {
        Self {
            host: TARGET_X86_64.into(),
            rustc_target_without_glibc_version: TARGET_X86_64.into(),
            arch: Arch::X86_64,
            ..Default::default()
        }
    }

    pub fn from_host() -> Result<Self> {
        let rustc_meta = rustc_version::version_meta().into_diagnostic()?;
        let mut target = TargetArch::from_str(&rustc_meta.host)?;
        if !target.compatible_host_linker() {
            target = TargetArch::x86_64();
        }
        target.channel = Some(rustc_meta.channel);
        Ok(target)
    }

    pub fn target_cpu(&self) -> String {
        match self.arch {
            Arch::ARM64 => "neoverse-n1".to_string(),
            Arch::X86_64 => "haswell".to_string(),
        }
    }

    pub fn set_al2_glibc_version(&mut self) {
        self.glibc_version = Some(AL2_GLIBC.into());
    }

    pub fn compatible_host_linker(&self) -> bool {
        self.rustc_target_without_glibc_version == TARGET_ARM
            || self.rustc_target_without_glibc_version == TARGET_X86_64
    }

    pub fn channel(&self) -> Result<Channel> {
        match self.channel {
            Some(c) => Ok(c),
            None => rustc_version::version_meta()
                .map(|m| m.channel)
                .into_diagnostic(),
        }
    }
}

impl Display for TargetArch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.rustc_target_without_glibc_version)?;
        if let Some(glibc_version) = self.glibc_version.as_ref() {
            write!(f, ".{glibc_version}")?;
        }
        Ok(())
    }
}

impl FromStr for TargetArch {
    type Err = miette::Report;

    fn from_str(host: &str) -> Result<Self, Self::Err> {
        // Validate that the build target is supported in AWS Lambda
        let arch = check_build_target(host)?;
        let mut target = Self {
            host: host.into(),
            rustc_target_without_glibc_version: host.into(),
            arch,
            ..Default::default()
        };
        if let Some((rustc_target_without_glibc_version, glibc_version)) = host.split_once('.') {
            target.rustc_target_without_glibc_version = rustc_target_without_glibc_version.into();
            target.glibc_version = Some(glibc_version.into());
        }
        Ok(target)
    }
}

/// Validate that the build target is supported in AWS Lambda
///
/// Here we use *starts with* instead of an exact match because:
///   - the target could also also be a *musl* variant: `x86_64-unknown-linux-musl`
///   - the target could also [specify a glibc version], which `cargo-zigbuild` supports
///
/// [specify a glibc version]: https://github.com/messense/cargo-zigbuild#specify-glibc-version
fn check_build_target(target: &str) -> Result<Arch> {
    if target.starts_with("aarch64-unknown-linux") {
        Ok(Arch::ARM64)
    } else if target.starts_with("x86_64-unknown-linux") {
        Ok(Arch::X86_64)
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
    fn test_target_arch_set_al2_glibc_version() {
        let mut t = TargetArch::from_str("x86_64-unknown-linux-gnu").unwrap();
        t.set_al2_glibc_version();
        assert_eq!("x86_64-unknown-linux-gnu.2.26", t.to_string().as_str());
    }

    #[test]
    fn test_target_arch_rustc_without_glibc_version() {
        let t = TargetArch::from_str("x86_64-unknown-linux-gnu.2.27").unwrap();
        assert_eq!(
            "x86_64-unknown-linux-gnu",
            t.rustc_target_without_glibc_version
        );
        assert_eq!(Some("2.27".into()), t.glibc_version);
    }

    #[test]
    fn test_target_arch_arch() {
        let t = TargetArch::from_str("x86_64-unknown-linux-gnu.2.27").unwrap();
        assert_eq!(Arch::X86_64, t.arch);

        let t = TargetArch::from_str("aarch64-unknown-linux-gnu.2.27").unwrap();
        assert_eq!(Arch::ARM64, t.arch);
    }

    #[test]
    fn test_check_build_target() {
        let arch = check_build_target("x86_64-unknown-linux-gnu.2.27").unwrap();
        assert_eq!(Arch::X86_64, arch);

        let arch = check_build_target("aarch64-unknown-linux-gnu.2.27").unwrap();
        assert_eq!(Arch::ARM64, arch);
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
}
