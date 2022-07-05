use miette::Result;
use std::{fmt::Display, str::FromStr};

const TARGET_ARM: &str = "aarch64-unknown-linux-gnu";
const TARGET_X86_64: &str = "x86_64-unknown-linux-gnu";
const AL2_GLIBC: &str = "2.26";

#[derive(Debug, PartialEq)]
enum Arch {
    ARM64,
    X86_64,
}

pub struct TargetArch {
    rustc_target_without_glibc_version: String,
    glibc_version: Option<String>,
    arch: Arch,
}

impl TargetArch {
    pub fn arm64() -> Self {
        Self {
            glibc_version: None,
            rustc_target_without_glibc_version: TARGET_ARM.into(),
            arch: Arch::ARM64,
        }
    }

    pub fn x86_64() -> Self {
        Self {
            glibc_version: None,
            rustc_target_without_glibc_version: TARGET_X86_64.into(),
            arch: Arch::X86_64,
        }
    }

    pub fn target_cpu(&self) -> String {
        match self.arch {
            Arch::ARM64 => "neoverse-n1".to_string(),
            Arch::X86_64 => "haswell".to_string(),
        }
    }

    pub fn rustc_target_without_glibc_version(&self) -> String {
        self.rustc_target_without_glibc_version.to_string()
    }

    pub fn set_al2_glibc_version(&mut self) {
        self.glibc_version = Some(AL2_GLIBC.to_string());
    }

    pub fn compatible_host_linker(host_target: &str) -> bool {
        host_target == TARGET_ARM || host_target == TARGET_X86_64
    }
}

impl Display for TargetArch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.rustc_target_without_glibc_version)?;
        if let Some(glibc_version) = self.glibc_version.as_ref() {
            write!(f, ".{}", glibc_version)?;
        }
        Ok(())
    }
}

impl FromStr for TargetArch {
    type Err = miette::Report;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Validate that the build target is supported in AWS Lambda
        let arch = check_build_target(s)?;
        match s.split_once('.') {
            Some((rustc_target_without_glibc_version, glibc_version)) => Ok(Self {
                rustc_target_without_glibc_version: rustc_target_without_glibc_version.into(),
                glibc_version: Some(glibc_version.into()),
                arch,
            }),
            None => Ok(Self {
                rustc_target_without_glibc_version: s.into(),
                glibc_version: None,
                arch,
            }),
        }
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
        // Unsupported target for an AWS Lambda environment
        Err(miette::miette!(
            "Invalid or unsupported target for AWS Lambda: {}",
            target
        ))
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
            t.rustc_target_without_glibc_version().as_str()
        );
        assert_eq!(Some("2.27".to_string()), t.glibc_version);
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
        assert!(TargetArch::compatible_host_linker(
            "x86_64-unknown-linux-gnu"
        ));
        assert!(TargetArch::compatible_host_linker(
            "aarch64-unknown-linux-gnu"
        ));
        assert!(!TargetArch::compatible_host_linker("x86_64-pc-windows-gnu"));
    }
}
