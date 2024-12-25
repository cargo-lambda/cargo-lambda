use cargo_lambda_interactive::{error::InquireError, is_user_cancellation_error};
use cargo_lambda_metadata::{
    cargo::{
        binary_targets_from_metadata,
        build::{Build, OutputFormat},
        cargo_release_profile_config, target_dir_from_metadata, CargoMetadata,
    },
    fs::copy_and_replace,
};
use miette::{IntoDiagnostic, Report, Result, WrapErr};
use std::{
    collections::HashSet,
    fs::create_dir_all,
    path::{Path, PathBuf},
    str::FromStr,
};
use target_arch::TargetArch;
use tracing::{debug, warn};

pub use cargo_zigbuild::Zig;

mod archive;
pub use archive::{create_binary_archive, zip_binary, BinaryArchive, BinaryData, BinaryModifiedAt};

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

#[tracing::instrument(skip(build, metadata), target = "cargo_lambda")]
pub async fn run(build: &mut Build, metadata: &CargoMetadata) -> Result<()> {
    tracing::trace!(options = ?build, "building project");

    let manifest_path = build.manifest_path();

    if (build.arm64 || build.x86_64) && !build.cargo_opts.target.is_empty() {
        Err(BuildError::InvalidTargetOptions)?;
    }

    let target_arch = if build.arm64 {
        TargetArch::arm64()
    } else if build.x86_64 {
        TargetArch::x86_64()
    } else {
        // let build_target = build.cargo_opts.target.first().or(metadata.target.as_ref());
        match build.cargo_opts.target.first() {
            Some(target) => {
                validate_linux_target(target)?;
                TargetArch::from_str(target)?
            }
            None => TargetArch::from_host()?,
        }
    };

    build.cargo_opts.target = vec![target_arch.to_string()];

    let build_examples = build.cargo_opts.examples || !build.cargo_opts.example.is_empty();
    let binaries = binary_targets_from_metadata(metadata, build_examples);
    debug!(binaries = ?binaries, "found new target binaries to build");

    let binaries = if !build.cargo_opts.bin.is_empty() {
        let mut final_binaries = HashSet::with_capacity(binaries.len());

        for name in &build.cargo_opts.bin {
            if !binaries.contains(name) {
                return Err(BuildError::FunctionBinaryMissing(name.into()).into());
            }
            final_binaries.insert(name.into());
        }

        final_binaries
    } else {
        binaries
    };

    let compiler_option = build.compiler.clone().unwrap_or_default();
    if compiler_option.is_local_cargo() {
        // This check only makes sense when the build host is local.
        // If the build host was ever going to be remote, like in a container,
        // this is not checked
        if !target_arch.compatible_host_linker() && !target_arch.is_static_linking() {
            return Err(BuildError::InvalidCompilerOption.into());
        }
    }

    if build.cargo_opts.release && !build.disable_optimizations {
        let release_optimizations =
            cargo_release_profile_config(manifest_path).map_err(BuildError::MetadataError)?;
        build.cargo_opts.config.extend(
            release_optimizations
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>(),
        );

        let build_flags = format!(
            "build.rustflags=[\"-C\", \"target-cpu={}\"]",
            target_arch.target_cpu()
        );
        build.cargo_opts.config.push(build_flags);

        debug!(config = ?build.cargo_opts.config, "release optimizations");
    }

    let profile = build_profile(&build.cargo_opts, &compiler_option);
    let skip_target_check = build.skip_target_check || which::which(rustup_cmd()).is_err();
    let cmd = build_command(
        &compiler_option,
        &build.cargo_opts,
        &target_arch,
        metadata,
        skip_target_check,
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
    let target_dir = target_dir_from_metadata(metadata).unwrap_or_else(|_| PathBuf::from("target"));
    let target_dir = Path::new(&target_dir);
    let lambda_dir = if let Some(dir) = &build.lambda_dir {
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

            let bootstrap_dir = if build.extension {
                lambda_dir.join("extensions")
            } else {
                match build.flatten {
                    Some(ref n) if n == name => lambda_dir.clone(),
                    _ => lambda_dir.join(name),
                }
            };
            create_dir_all(&bootstrap_dir)
                .into_diagnostic()
                .wrap_err_with(|| format!("error creating lambda directory {bootstrap_dir:?}"))?;

            let data = BinaryData::new(name.as_str(), build.extension, build.internal);

            match build.output_format() {
                OutputFormat::Binary => {
                    let output_location = bootstrap_dir.join(data.binary_name());
                    copy_and_replace(&binary, &output_location)
                        .into_diagnostic()
                        .wrap_err_with(|| {
                            format!("error moving the binary `{binary:?}` into the output location `{output_location:?}`")
                        })?;
                }
                OutputFormat::Zip => {
                    zip_binary(binary, bootstrap_dir, &data, build.include.clone())?;
                }
            }
        }
    }
    if !found_binaries {
        warn!(?base, "no binaries found in target directory after build, try using the --bin, --example, or --package options to build specific binaries");
    }

    Ok(())
}

fn downcasted_user_cancellation(err: &Report) -> bool {
    match err.root_cause().downcast_ref::<InquireError>() {
        Some(err) => is_user_cancellation_error(err),
        None => false,
    }
}
