use cargo_lambda_interactive::{error::InquireError, is_user_cancellation_error};
use cargo_lambda_metadata::{
    cargo::{
        binary_targets_from_metadata, cargo_release_profile_config, function_build_metadata,
        load_metadata, target_dir_from_metadata, CompilerOptions,
    },
    fs::copy_and_replace,
};
use cargo_options::Build as CargoBuild;
use clap::{Args, ValueHint};
use miette::{IntoDiagnostic, Report, Result, WrapErr};
use std::{
    collections::HashSet,
    fs::create_dir_all,
    path::{Path, PathBuf},
    str::FromStr,
};
use strum_macros::EnumString;
use target_arch::TargetArch;
use tracing::{debug, warn};

pub use cargo_zigbuild::Zig;

mod archive;
pub use archive::{create_binary_archive, zip_binary, BinaryArchive, BinaryData};

mod compiler;
use compiler::{build_command, build_profile};

mod error;
use error::BuildError;

mod target_arch;
use target_arch::validate_linux_target;

mod toolchain;
use toolchain::rustup_cmd;

mod zig;
pub use zig::{
    check_installation, install_options, install_zig, print_install_options, InstallOption,
};

#[derive(Args, Clone, Debug)]
#[command(
    name = "build",
    after_help = "Full command documentation: https://www.cargo-lambda.info/commands/build.html"
)]
pub struct Build {
    /// The format to produce the compile Lambda into, acceptable values are [Binary, Zip]
    #[arg(short, long, default_value_t = OutputFormat::Binary)]
    output_format: OutputFormat,

    /// Directory where the final lambda binaries will be located
    #[arg(short, long, value_hint = ValueHint::DirPath)]
    lambda_dir: Option<PathBuf>,

    /// Shortcut for --target aarch64-unknown-linux-gnu
    #[arg(long)]
    arm64: bool,

    /// Shortcut for --target x86_64-unknown-linux-gnu
    #[arg(long)]
    x86_64: bool,

    /// Whether the code that you're building is a Lambda Extension
    #[arg(long)]
    extension: bool,

    /// Whether an extension is internal or external
    #[arg(long, requires = "extension")]
    internal: bool,

    /// Put a bootstrap file in the root of the lambda directory.
    /// Use the name of the compiled binary to choose which file to move.
    #[arg(long)]
    flatten: Option<String>,

    /// Whether to skip the target check
    #[arg(long)]
    skip_target_check: bool,

    /// Backend to build the project with
    #[arg(short, long, env = "CARGO_LAMBDA_COMPILER")]
    compiler: Option<CompilerFlag>,

    /// Disable all default release optimizations
    #[arg(long)]
    disable_optimizations: bool,

    /// Option to add one or more files and directories to include in the output ZIP file (only works with --output-format=zip).
    #[arg(short, long)]
    include: Option<Vec<String>>,

    #[command(flatten)]
    build: CargoBuild,
}

#[derive(Clone, Debug, strum_macros::Display, EnumString)]
#[strum(ascii_case_insensitive)]
enum OutputFormat {
    Binary,
    Zip,
}

#[derive(Clone, Debug, strum_macros::Display, EnumString, Eq, PartialEq)]
#[strum(ascii_case_insensitive, serialize_all = "snake_case")]
enum CompilerFlag {
    CargoZigbuild,
    Cargo,
    Cross,
}

impl Build {
    #[tracing::instrument(skip(self), target = "cargo_lambda")]
    pub async fn run(&mut self) -> Result<()> {
        tracing::trace!(options = ?self, "building project");

        let manifest_path = self.build.manifest_path.clone();
        let manifest_path = manifest_path
            .as_deref()
            .unwrap_or_else(|| Path::new("Cargo.toml"));

        let metadata = load_metadata(manifest_path).map_err(BuildError::MetadataError)?;
        let build_config = function_build_metadata(&metadata).map_err(BuildError::MetadataError)?;
        let compiler_option = match (&build_config.compiler, &self.compiler) {
            (None, None) => CompilerOptions::default(),
            (_, Some(c)) => CompilerOptions::from(c.to_string()),
            (Some(c), _) => c.clone(),
        };

        if (self.arm64 || self.x86_64) && !self.build.target.is_empty() {
            Err(BuildError::InvalidTargetOptions)?;
        }

        let target_arch = if self.arm64 {
            TargetArch::arm64()
        } else if self.x86_64 {
            TargetArch::x86_64()
        } else {
            let build_target = self.build.target.first().or(build_config.target.as_ref());
            match build_target {
                Some(target) => {
                    validate_linux_target(target)?;
                    TargetArch::from_str(target)?
                }
                None => TargetArch::from_host()?,
            }
        };

        self.build.target = vec![target_arch.to_string()];

        let build_examples = self.build.examples || !self.build.example.is_empty();
        let binaries = binary_targets_from_metadata(&metadata, build_examples);
        debug!(binaries = ?binaries, "found new target binaries to build");

        let binaries = if !self.build.bin.is_empty() {
            let mut final_binaries = HashSet::with_capacity(binaries.len());

            for name in &self.build.bin {
                if !binaries.contains(name) {
                    return Err(BuildError::FunctionBinaryMissing(name.into()).into());
                }
                final_binaries.insert(name.into());
            }

            final_binaries
        } else {
            binaries
        };

        if compiler_option.is_local_cargo() {
            // This check only makes sense when the build host is local.
            // If the build host was ever going to be remote, like in a container,
            // this is not checked
            if !target_arch.compatible_host_linker() && !target_arch.is_static_linking() {
                return Err(BuildError::InvalidCompilerOption.into());
            }
        }

        if self.build.release && !self.disable_optimizations {
            let release_optimizations =
                cargo_release_profile_config(manifest_path).map_err(BuildError::MetadataError)?;
            self.build.config.extend(
                release_optimizations
                    .into_iter()
                    .map(String::from)
                    .collect::<Vec<_>>(),
            );

            let build_flags = format!(
                "build.rustflags=[\"-C\", \"target-cpu={}\"]",
                target_arch.target_cpu()
            );
            self.build.config.push(build_flags);

            debug!(config = ?self.build.config, "release optimizations");
        }

        let profile = build_profile(&self.build, &compiler_option);
        let cmd = build_command(
            &compiler_option,
            &self.build,
            &target_arch,
            &metadata,
            self.skip_target_check(),
        )
        .await;

        let mut cmd = match cmd {
            Ok(cmd) => cmd,
            Err(err) if downcasted_user_cancellation(&err) => return Ok(()),
            Err(err) => return Err(err),
        };

        let mut child = cmd.spawn().map_err(BuildError::FailedBuildCommand)?;
        let status = child.wait().map_err(BuildError::FailedBuildCommand)?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }

        // extract resolved target dir from cargo metadata
        let target_dir =
            target_dir_from_metadata(&metadata).unwrap_or_else(|_| PathBuf::from("target"));
        let target_dir = Path::new(&target_dir);
        let lambda_dir = if let Some(dir) = &self.lambda_dir {
            dir.clone()
        } else {
            target_dir.join("lambda")
        };

        let mut base = target_dir
            .join(target_arch.rustc_target_without_glibc_version())
            .join(profile);
        if build_examples {
            base = base.join("examples");
        }

        let mut found_binaries = false;
        for name in &binaries {
            let binary = base.join(name);
            debug!(binary = ?binary, exists = binary.exists(), "checking function binary");

            if binary.exists() {
                found_binaries = true;

                let bootstrap_dir = if self.extension {
                    lambda_dir.join("extensions")
                } else {
                    match self.flatten {
                        Some(ref n) if n == name => lambda_dir.clone(),
                        _ => lambda_dir.join(name),
                    }
                };
                create_dir_all(&bootstrap_dir)
                    .into_diagnostic()
                    .wrap_err_with(|| {
                        format!("error creating lambda directory {bootstrap_dir:?}")
                    })?;

                let data = BinaryData::new(name.as_str(), self.extension, self.internal);

                match self.output_format {
                    OutputFormat::Binary => {
                        let output_location = bootstrap_dir.join(data.binary_name());
                        copy_and_replace(&binary, &output_location)
                            .into_diagnostic()
                            .wrap_err_with(|| {
                                format!("error moving the binary `{binary:?}` into the output location `{output_location:?}`")
                            })?;
                    }
                    OutputFormat::Zip => {
                        let extra_files = self
                            .include
                            .clone()
                            .or_else(|| build_config.include.clone());
                        zip_binary(binary, bootstrap_dir, &data, extra_files)?;
                    }
                }
            }
        }
        if !found_binaries {
            warn!(?base, "no binaries found in target directory after build, try using the --bin, --example, or --package options to build specific binaries");
        }

        Ok(())
    }

    fn skip_target_check(&self) -> bool {
        self.skip_target_check || which::which(rustup_cmd()).is_err()
    }
}

fn downcasted_user_cancellation(err: &Report) -> bool {
    match err.root_cause().downcast_ref::<InquireError>() {
        Some(err) => is_user_cancellation_error(err),
        None => false,
    }
}
