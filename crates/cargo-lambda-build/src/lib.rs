use cargo_lambda_interactive::{error::InquireError, is_user_cancellation_error};
use cargo_lambda_metadata::{
    cargo::{
        binary_targets_from_metadata, cargo_release_profile_config, function_build_metadata,
        load_metadata, target_dir, target_dir_from_metadata, CompilerOptions,
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
    env, fmt,
    fs::{create_dir_all, read, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
};
use strum_macros::EnumString;
use target_arch::TargetArch;
use toolchain::rustup_cmd;
use tracing::{debug, trace, warn};
use walkdir::WalkDir;
use zip::{write::FileOptions, ZipWriter};

pub use cargo_zigbuild::Zig;

mod compiler;

mod error;
use error::BuildError;

mod target_arch;
use target_arch::validate_linux_target;

use crate::compiler::{build_command, build_profile};

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
    include: Option<Vec<PathBuf>>,

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

        if !self.build.bin.is_empty() {
            for name in &self.build.bin {
                if !binaries.contains(name) {
                    return Err(BuildError::FunctionBinaryMissing(name.into()).into());
                }
            }
        }

        if compiler_option.is_local_cargo() {
            // This check only makes sense when the build host is local.
            // If the build host was ever going to be remote, like in a container,
            // this is not checked
            if !target_arch.compatible_host_linker() && !target_arch.is_static_linking() {
                return Err(BuildError::InvalidCompilerOption.into());
            }
        }

        let rust_flags = if self.build.release && !self.disable_optimizations {
            let release_optimizations =
                cargo_release_profile_config(manifest_path).map_err(BuildError::MetadataError)?;
            self.build.config.extend(
                release_optimizations
                    .into_iter()
                    .map(String::from)
                    .collect::<Vec<_>>(),
            );

            let mut rust_flags = env::var("RUSTFLAGS").unwrap_or_default();
            if !rust_flags.contains("-C target-cpu=") {
                if !rust_flags.is_empty() {
                    rust_flags += " ";
                }
                rust_flags += "-C target-cpu=";
                rust_flags += target_arch.target_cpu();
            }

            debug!(?rust_flags, config = ?self.build.config, "release optimizations");
            Some(rust_flags)
        } else {
            None
        };

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

        if let Some(rust_flags) = rust_flags {
            cmd.env("RUSTFLAGS", rust_flags);
        }

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

                let bin_name = if self.extension {
                    name.as_str()
                } else {
                    "bootstrap"
                };

                match self.output_format {
                    OutputFormat::Binary => {
                        let output_location = bootstrap_dir.join(bin_name);
                        copy_and_replace(&binary, &output_location)
                            .into_diagnostic()
                            .wrap_err_with(|| {
                                format!("error moving the binary `{binary:?}` into the output location `{output_location:?}`")
                            })?;
                    }
                    OutputFormat::Zip => {
                        let parent = if self.extension && !self.internal {
                            Some("extensions")
                        } else {
                            None
                        };
                        let extra_files = self.include.clone().or_else(|| build_config.include.clone());
                        zip_binary(bin_name, binary, bootstrap_dir, parent, extra_files)?;
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

pub struct BinaryArchive {
    pub architecture: String,
    pub sha256: String,
    pub path: PathBuf,
}

impl BinaryArchive {
    pub fn read(&self) -> Result<Vec<u8>> {
        read(&self.path)
            .into_diagnostic()
            .wrap_err("failed to read binary archive")
    }

    pub fn add_files(&self, files: &Vec<PathBuf>) -> Result<()> {
        trace!(?self.path, ?files, "adding files to zip file");
        let zipfile = std::fs::File::open(&self.path).into_diagnostic()?;

        let mut archive = zip::ZipArchive::new(zipfile).into_diagnostic()?;

        // Open a new, empty archive for writing to
        let tmp_dir = tempfile::tempdir().into_diagnostic()?;
        let tmp_path = tmp_dir
            .path()
            .join(self.path.file_name().expect("missing zip file name"));
        let tmp = File::create(&tmp_path).into_diagnostic()?;
        let mut new_archive = zip::ZipWriter::new(tmp);

        for i in 0..archive.len() {
            let file = archive.by_index_raw(i).into_diagnostic()?;
            new_archive.raw_copy_file(file).into_diagnostic()?;
        }

        include_files_in_zip(&mut new_archive, files)?;

        new_archive
            .finish()
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to finish zip file `{}`", self.path.display()))?;

        drop(archive);
        drop(new_archive);
        copy_and_replace(&tmp_path, &self.path).into_diagnostic()?;
        Ok(())
    }
}

/// Search for the bootstrap file for a function inside the target directory.
/// If the binary file exists, it creates the zip archive and extracts its architecture by reading the binary.
pub fn find_binary_archive<M, P>(
    name: &str,
    manifest_path: M,
    base_dir: &Option<P>,
    is_extension: bool,
    is_internal: bool,
    include: Option<Vec<PathBuf>>,
) -> Result<BinaryArchive>
where
    M: AsRef<Path> + fmt::Debug,
    P: AsRef<Path>,
{
    let target_dir = target_dir(manifest_path).unwrap_or_else(|_| PathBuf::from("target"));
    let (dir_name, binary_name, parent) = if is_extension && is_internal {
        ("extensions", name, None)
    } else if is_extension && !is_internal {
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
        let build_cmd = if is_extension && is_internal {
            "build --extension --internal"
        } else if is_extension && !is_internal {
            "build --extension"
        } else {
            "build"
        };
        return Err(BuildError::BinaryMissing(name.into(), build_cmd.into()).into());
    }

    zip_binary(binary_name, binary_path, bootstrap_dir, parent, include)
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
    include: Option<Vec<PathBuf>>,
) -> Result<BinaryArchive> {
    let path = binary_path.as_ref();
    let dir = destination_directory.as_ref();
    let zipped = dir.join(format!("{name}.zip"));
    debug!(name, parent, ?path, ?dir, ?zipped, "zipping binary");

    let zipped_binary = File::create(&zipped)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to create zip file `{zipped:?}`"))?;
    let binary_data = read(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to read binary file `{path:?}`"))?;
    let binary_perm = binary_permissions(path)?;
    let binary_data = &*binary_data;
    let object = ObjectFile::parse(binary_data)
        .into_diagnostic()
        .wrap_err("the provided function file is not a valid Linux binary")?;

    let arch = match object.architecture() {
        Architecture::Aarch64 => "arm64",
        Architecture::X86_64 => "x86_64",
        other => return Err(BuildError::InvalidBinaryArchitecture(other).into()),
    };

    let mut hasher = Sha256::new();
    hasher.update(binary_data);
    let sha256 = format!("{:X}", hasher.finalize());

    let mut zip = ZipWriter::new(zipped_binary);
    if let Some(files) = include {
        include_files_in_zip(&mut zip, &files)?;
    }

    let file_name = if let Some(parent) = parent {
        zip.add_directory(parent, FileOptions::default())
            .into_diagnostic()
            .wrap_err_with(|| {
                format!("failed to add directory `{parent}` to zip file `{zipped:?}`")
            })?;
        Path::new(parent).join(name)
    } else {
        PathBuf::from(name)
    };

    let zip_file_name = convert_to_unix_path(&file_name)
        .ok_or_else(|| BuildError::InvalidUnixFileName(file_name.clone()))?;
    zip.start_file(
        zip_file_name.to_string(),
        FileOptions::default().unix_permissions(binary_perm),
    )
    .into_diagnostic()
    .wrap_err_with(|| format!("failed to start zip file `{zip_file_name:?}`"))?;
    zip.write_all(binary_data)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to write data into zip file `{zip_file_name:?}`"))?;
    zip.finish()
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to finish zip file `{zip_file_name:?}`"))?;

    Ok(BinaryArchive {
        architecture: arch.into(),
        path: zipped,
        sha256,
    })
}

#[cfg(unix)]
fn binary_permissions(path: &Path) -> Result<u32> {
    use std::os::unix::prelude::PermissionsExt;
    let meta = std::fs::metadata(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to get binary permissions from file `{path:?}`"))?;
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

fn include_files_in_zip<W>(zip: &mut ZipWriter<W>, files: &Vec<PathBuf>) -> Result<()>
where
    W: Write + io::Seek,
{
    for file in files {
        for entry in WalkDir::new(file).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            let entry_name = convert_to_unix_path(path)
                .ok_or_else(|| BuildError::InvalidUnixFileName(path.to_path_buf()))?;

            if path.is_dir() {
                trace!(?entry_name, "creating directory in zip file");

                zip.add_directory(entry_name.to_string(), FileOptions::default())
                    .into_diagnostic()
                    .wrap_err_with(|| {
                        format!("failed to add directory `{entry_name}` to zip file")
                    })?;
            } else {
                let mut content = Vec::new();
                let mut file = File::open(path)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("failed to open file `{path:?}`"))?;
                file.read_to_end(&mut content)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("failed to read file `{path:?}`"))?;

                trace!(?entry_name, "including file in zip file");

                zip.start_file(entry_name.to_string(), FileOptions::default())
                    .into_diagnostic()
                    .wrap_err_with(|| format!("failed to start zip file `{entry_name:?}`"))?;

                zip.write_all(&content)
                    .into_diagnostic()
                    .wrap_err_with(|| {
                        format!("failed to write data into zip file `{entry_name:?}`")
                    })?;
            }
        }
    }
    Ok(())
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
            convert_to_unix_path(path).expect("failed to convert file path")
        );
    }
}
