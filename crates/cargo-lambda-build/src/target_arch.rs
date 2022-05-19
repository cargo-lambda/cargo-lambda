use miette::Result;
use std::str::FromStr;

use crate::{TARGET_ARM, TARGET_X86_64};

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
    pub fn full_zig_string(&self) -> String {
        format!(
            "{}.{}",
            self.rustc_target_without_glibc_version,
            self.glibc_version.as_ref().unwrap_or(&"".to_string())
        )
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
