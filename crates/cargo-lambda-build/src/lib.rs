use cargo_lambda_interactive::{error::InquireError, is_user_cancellation_error};
use cargo_lambda_metadata::{
    cargo::{
        binary_targets_from_metadata, function_build_metadata, load_metadata,
        target_dir_from_metadata, CompilerOptions,
    },
    fs::copy_and_replace,
};
use cargo_options::Build as CargoBuild;
use clap::{Args, ValueHint};
use miette::{IntoDiagnostic, Report, Result, WrapErr};
use object::{read::File as ObjectFile, Architecture, Object};
use sha2::{Digest, Sha256};
use std::{
    borrow::Cow,
    env,
    fs::{create_dir_all, read, File},
    io::Write,
    path::{Path, PathBuf},
    str::FromStr,
};
use strum_macros::EnumString;
use target_arch::TargetArch;
use tracing::{debug, warn};
use zip::{write::FileOptions, ZipWriter};

pub use cargo_zigbuild::Zig;

mod compiler;
use compiler::new_compiler;

mod error;
use error::BuildError;

mod target_arch;
use target_arch::validate_linux_target;

mod toolchain;
mod zig;

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

    /// Put a bootstrap file in the root of the lambda directory.
    /// Use the name of the compiled binary to choose which file to move.
    #[arg(long)]
    flatten: Option<String>,

    #[arg(short, long, default_value_t = CompilerFlag::CargoZigbuild, env = "CARGO_LAMBDA_COMPILER")]
    compiler: CompilerFlag,

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

        let manifest_path = self
            .build
            .manifest_path
            .as_deref()
            .unwrap_or_else(|| Path::new("Cargo.toml"));

        let metadata = load_metadata(manifest_path)?;
        let mut build_config = function_build_metadata(&metadata)?;
        build_config.compiler = CompilerOptions::from(self.compiler.to_string());

        if (self.arm64 || self.x86_64) && !self.build.target.is_empty() {
            Err(BuildError::InvalidTargetOptions)?;
        }

        let mut target_arch = if self.arm64 {
            TargetArch::arm64()
        } else if self.x86_64 {
            TargetArch::x86_64()
        } else {
            let build_target = self.build.target.get(0);
            match build_target {
                Some(target) => {
                    validate_linux_target(target)?;
                    TargetArch::from_str(target)?
                }
                None => TargetArch::from_host()?,
            }
        };

        self.build.target = vec![target_arch.to_string()];

        let binaries = binary_targets_from_metadata(&metadata)?;
        debug!(binaries = ?binaries, "found new target binaries to build");

        if !self.build.bin.is_empty() {
            for name in &self.build.bin {
                if !binaries.contains(name) {
                    return Err(BuildError::FunctionBinaryMissing(name.into()).into());
                }
            }
        }

        if build_config.is_local_compiler() && !build_config.is_zig_enabled() {
            // This check only makes sense when the build host is local.
            // If the build host was ever going to be remote, like in a container,
            // this is not checked
            if target_arch.compatible_host_linker() {
                target_arch.set_al2_glibc_version();
            } else {
                return Err(BuildError::InvalidCompilerOption.into());
            }
        }

        let rust_flags = if self.build.release {
            let mut rust_flags = env::var("RUSTFLAGS").unwrap_or_default();
            if !rust_flags.contains("-C strip=") {
                if !rust_flags.is_empty() {
                    rust_flags += " ";
                }
                rust_flags += "-C strip=symbols";
            }
            if !rust_flags.contains("-C target-cpu=") {
                if !rust_flags.is_empty() {
                    rust_flags += " ";
                }
                let target_cpu = target_arch.target_cpu();
                rust_flags += "-C target-cpu=";
                rust_flags += target_cpu.as_str();
            }

            debug!(rust_flags = ?rust_flags, "release RUSTFLAGS");
            Some(rust_flags)
        } else {
            None
        };

        let compiler = new_compiler(build_config.compiler);
        let profile = compiler.build_profile(&self.build);
        let cmd = compiler.command(&self.build, &target_arch).await;

        let mut cmd = match cmd {
            Ok(cmd) => cmd,
            Err(err) if downcasted_user_cancellation(&err) => return Ok(()),
            Err(err) => return Err(err),
        };

        if let Some(rust_flags) = rust_flags {
            cmd.env("RUSTFLAGS", rust_flags);
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

        // extract resolved target dir from cargo metadata
        let target_dir =
            target_dir_from_metadata(&metadata).unwrap_or_else(|_| PathBuf::from("target"));
        let target_dir = Path::new(&target_dir);
        let lambda_dir = if let Some(dir) = &self.lambda_dir {
            dir.clone()
        } else {
            target_dir.join("lambda")
        };

        let base = target_dir
            .join(target_arch.rustc_target_without_glibc_version)
            .join(profile);

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
                create_dir_all(&bootstrap_dir).into_diagnostic()?;

                let bin_name = if self.extension {
                    name.as_str()
                } else {
                    "bootstrap"
                };

                match self.output_format {
                    OutputFormat::Binary => {
                        copy_and_replace(binary, bootstrap_dir.join(bin_name)).into_diagnostic()?;
                    }
                    OutputFormat::Zip => {
                        let parent = if self.extension {
                            Some("extensions")
                        } else {
                            None
                        };
                        zip_binary(bin_name, binary, bootstrap_dir, parent)?;
                    }
                }
            }
        }
        if !found_binaries {
            warn!("no binaries found in target after build, try using the --bin or --package options to build specific binaries");
        }

        Ok(())
    }
}

pub struct BinaryArchive {
    pub architecture: String,
    pub sha256: String,
    pub path: PathBuf,
}

/// Search for the bootstrap file for a function inside the target directory.
/// If the binary file exists, it creates the zip archive and extracts its architecture by reading the binary.
pub fn find_binary_archive<P: AsRef<Path>>(
    name: &str,
    base_dir: &Option<P>,
    is_extension: bool,
) -> Result<BinaryArchive> {
    let target_dir = Path::new("target");
    let (dir_name, binary_name, parent) = if is_extension {
        ("extensions", name, Some("extensions"))
    } else {
        (name, "bootstrap", None)
    };

    let bootstrap_dir = if let Some(dir) = base_dir {
        dir.as_ref().join(dir_name)
    } else {
        target_dir.join("lambda").join(dir_name)
    };

    let binary_path = bootstrap_dir.join(binary_name);
    if !binary_path.exists() {
        let build_cmd = if is_extension {
            "build --extension"
        } else {
            "build"
        };
        return Err(BuildError::BinaryMissing(name.into(), build_cmd.into()).into());
    }

    zip_binary(binary_name, binary_path, bootstrap_dir, parent)
}

/// Create a zip file from a function binary.
/// The binary inside the zip file is called `bootstrap` for function binaries.
/// The binary inside the zip file is called by its name, and put inside the `extensions`
/// directory, for extension binaries.
pub fn zip_binary<BP: AsRef<Path>, DD: AsRef<Path>>(
    name: &str,
    binary_path: BP,
    destination_directory: DD,
    parent: Option<&str>,
) -> Result<BinaryArchive> {
    let path = binary_path.as_ref();
    let dir = destination_directory.as_ref();
    let zipped = dir.join(format!("{name}.zip"));

    let zipped_binary = File::create(&zipped).into_diagnostic()?;
    let binary_data = read(path).into_diagnostic()?;
    let binary_perm = binary_permissions(path)?;
    let binary_data = &*binary_data;
    let object = ObjectFile::parse(binary_data).into_diagnostic()?;

    let arch = match object.architecture() {
        Architecture::Aarch64 => "arm64",
        Architecture::X86_64 => "x86_64",
        other => return Err(BuildError::InvalidBinaryArchitecture(other).into()),
    };

    let mut hasher = Sha256::new();
    hasher.update(binary_data);
    let sha256 = format!("{:X}", hasher.finalize());

    let mut zip = ZipWriter::new(zipped_binary);
    let file_name = if let Some(parent) = parent {
        zip.add_directory(parent, FileOptions::default())
            .into_diagnostic()?;
        Path::new(parent).join(name)
    } else {
        PathBuf::from(name)
    };

    zip.start_file(
        convert_to_unix_path(&file_name).expect("failed to convert file path"),
        FileOptions::default().unix_permissions(binary_perm),
    )
    .into_diagnostic()?;
    zip.write_all(binary_data).into_diagnostic()?;
    zip.finish().into_diagnostic()?;

    Ok(BinaryArchive {
        architecture: arch.into(),
        path: zipped,
        sha256,
    })
}

#[cfg(unix)]
fn binary_permissions(path: &Path) -> Result<u32> {
    use std::os::unix::prelude::PermissionsExt;
    let meta = std::fs::metadata(path).into_diagnostic()?;
    Ok(meta.permissions().mode())
}

#[cfg(not(unix))]
fn binary_permissions(_path: &Path) -> Result<u32> {
    Ok(0o755)
}

#[cfg(target_os = "windows")]
fn convert_to_unix_path(path: &Path) -> Option<Cow<'_, str>> {
    let mut path_str = String::new();
    for component in path.components() {
        if let std::path::Component::Normal(os_str) = component {
            if !path_str.is_empty() {
                path_str.push('/');
            }
            path_str.push_str(os_str.to_str()?);
        }
    }
    Some(Cow::Owned(path_str))
}

#[cfg(not(target_os = "windows"))]
fn convert_to_unix_path(path: &Path) -> Option<Cow<'_, str>> {
    path.to_str().map(Cow::Borrowed)
}

fn downcasted_user_cancellation(err: &Report) -> bool {
    match err.root_cause().downcast_ref::<InquireError>() {
        Some(err) => is_user_cancellation_error(err),
        None => false,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_convert_to_unix_path() {
        // On Windows, a PathBuff constructed from Path::join will have "\" as separator, while on Unix-like systems it will have "/"
        let path = Path::new("extensions").join("test").join("filename");
        assert_eq!(
            "extensions/test/filename",
            convert_to_unix_path(&path).expect("failed to convert file path")
        );
    }

    #[test]
    fn test_convert_to_unix_path_keep_original() {
        let path = Path::new("extensions/test/filename");
        assert_eq!(
            "extensions/test/filename",
            convert_to_unix_path(path).expect("failed to convert file path")
        );
    }

    #[test]
    fn test_convert_to_unix_path_empty_path() {
        let path = Path::new("");
        assert_eq!(
            "",
            convert_to_unix_path(&path).expect("failed to convert file path")
        );
    }
}
