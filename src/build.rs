use crate::{metadata, toolchain, zig};
use cargo_zigbuild::Build as ZigBuild;
use clap::{Args, ValueHint};
use miette::{IntoDiagnostic, Result, WrapErr};
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use strum_macros::EnumString;

#[derive(Args, Clone, Debug)]
#[clap(name = "build")]
pub struct Build {
    /// The format to produce the compile Lambda into, acceptable values are [Binary, Zip].
    #[clap(long, default_value_t = OutputFormat::Binary)]
    output_format: OutputFormat,
    /// Directory where the final lambda binaries will be located
    #[clap(short, long, value_hint = ValueHint::DirPath)]
    lambda_dir: Option<PathBuf>,
    #[clap(flatten)]
    build: ZigBuild,
}

#[derive(Clone, Debug, strum_macros::Display, EnumString)]
#[strum(ascii_case_insensitive)]
enum OutputFormat {
    Binary,
    Zip,
}

impl Build {
    pub fn run(&mut self) -> Result<()> {
        let rustc_meta = rustc_version::version_meta().into_diagnostic()?;
        let host_target = &rustc_meta.host;
        let release_channel = &rustc_meta.channel;

        let build_target = self.build.target.get(0);
        let resolved_build_target: &str = match build_target {
            Some(target) => {
                // Validate that the build target is supported in AWS Lambda
                check_build_target(target)?;
                // Same explicit target as host target
                if host_target == target {
                    self.build.disable_zig_linker = true
                }
                target
            }
            // No explicit target, but build host same as target host
            None if host_target == "aarch64-unknown-linux-gnu"
                || host_target == "x86_64-unknown-linux-gnu" =>
            {
                self.build.disable_zig_linker = true;
                // Set the target explicitly, so it's easier to find the binaries later
                self.build.target = vec![host_target.into()];
                host_target
            }
            // No explicit target, and build host not compatible with Lambda hosts
            None => {
                let target = "x86_64-unknown-linux-gnu";
                self.build.target = vec![target.into()];
                target
            }
        };

        // confirm that target component is included in host toolchain, or add
        // it with `rustup` otherwise.
        toolchain::check_target_component_with_rustc_meta(
            resolved_build_target,
            host_target,
            release_channel,
        )?;

        let manifest_path = self
            .build
            .manifest_path
            .as_deref()
            .unwrap_or_else(|| Path::new("Cargo.toml"));
        let binaries = metadata::binary_packages(manifest_path.to_path_buf())?;

        if !self.build.bin.is_empty() {
            for name in &self.build.bin {
                if !binaries.contains(name) {
                    return Err(miette::miette!(
                        "binary target is missing from this project: {}",
                        name
                    ));
                }
            }
        }

        if !self.build.disable_zig_linker {
            zig::check_installation()?;
        }

        let mut cmd = self
            .build
            .build_command("build")
            .map_err(|e| miette::miette!("{}", e))?;
        if self.build.release {
            cmd.env("RUSTFLAGS", "-C strip=symbols");
        }

        let mut child = cmd
            .spawn()
            .into_diagnostic()
            .wrap_err("Failed to run cargo build")?;
        let status = child
            .wait()
            .into_diagnostic()
            .wrap_err("Failed to wait on cargo build process")?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }

        let final_target = self
            .build
            .target
            .get(0)
            .map(|x| x.as_str())
            .unwrap_or("x86_64-unknown-linux-gnu");
        let profile = match self.build.profile.as_deref() {
            Some("dev" | "test") => "debug",
            Some("release" | "bench") => "release",
            Some(profile) => profile,
            None if self.build.release => "release",
            None => "debug",
        };

        let target_dir = Path::new("target");
        let lambda_dir = if let Some(dir) = &self.lambda_dir {
            dir.clone()
        } else {
            target_dir.join("lambda")
        };

        let base = target_dir.join(final_target).join(profile);

        for name in &binaries {
            let binary = base.join(name);
            if binary.exists() {
                let bootstrap_dir = lambda_dir.join(name);
                std::fs::create_dir_all(&bootstrap_dir).into_diagnostic()?;
                match self.output_format {
                    OutputFormat::Binary => {
                        std::fs::rename(binary, bootstrap_dir.join("bootstrap"))
                            .into_diagnostic()?;
                    }
                    OutputFormat::Zip => {
                        let zipped_binary =
                            std::fs::File::create(bootstrap_dir.join("bootstrap.zip"))
                                .into_diagnostic()?;
                        let mut zip = zip::ZipWriter::new(zipped_binary);
                        zip.start_file("bootstrap", Default::default())
                            .into_diagnostic()?;
                        zip.write_all(&std::fs::read(binary).into_diagnostic()?)
                            .into_diagnostic()?;
                        zip.finish().into_diagnostic()?;
                    }
                }
            }
        }

        Ok(())
    }
}

/// Validate that the build target is supported in AWS Lambda
///
/// Here we use *starts with* instead of an exact match because:
///   - the target could also also be a *musl* variant: `x86_64-unknown-linux-musl`
///   - the target could also [specify a glibc version], which `cargo-zigbuild` supports
///
/// [specify a glibc version]: https://github.com/messense/cargo-zigbuild#specify-glibc-version
fn check_build_target(target: &str) -> Result<()> {
    if !target.starts_with("aarch64-unknown-linux") && !target.starts_with("x86_64-unknown-linux") {
        // Unsupported target for an AWS Lambda environment
        return Err(miette::miette!(
            "Invalid or unsupported target for AWS Lambda: {}",
            target
        ));
    }

    Ok(())
}
