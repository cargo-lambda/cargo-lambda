use std::{
    borrow::Cow,
    fmt::Debug,
    fs::{read, File, Metadata},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
};

use cargo_lambda_metadata::{cargo::target_dir, fs::copy_and_replace};
use chrono::{DateTime, Utc};
use miette::{Context, IntoDiagnostic, Result};
use object::{read::File as ObjectFile, Architecture, Object};
use sha2::{Digest, Sha256};
use tracing::{debug, trace};
use walkdir::WalkDir;
use zip::{write::SimpleFileOptions, ZipArchive, ZipWriter};

use crate::error::BuildError;

#[derive(Debug)]
pub enum BinaryData<'a> {
    Function(&'a str),
    ExternalExtension(&'a str),
    InternalExtension(&'a str),
}

impl<'a> BinaryData<'a> {
    /// Create a BinaryData given the arguments of the CLI
    pub fn new(name: &'a str, extension: bool, internal: bool) -> Self {
        if extension {
            if internal {
                BinaryData::InternalExtension(name)
            } else {
                BinaryData::ExternalExtension(name)
            }
        } else {
            BinaryData::Function(name)
        }
    }

    /// Name of the binary to copy inside the zip archive
    pub fn binary_name(&self) -> &str {
        match self {
            BinaryData::Function(_) => "bootstrap",
            BinaryData::ExternalExtension(name) => name,
            BinaryData::InternalExtension(name) => name,
        }
    }

    /// Name of the zip archive
    pub fn zip_name(&self) -> String {
        format!("{}.zip", self.binary_name())
    }

    /// Location of the binary after building it
    pub fn binary_location(&self) -> &str {
        match self {
            BinaryData::Function(name) => name,
            BinaryData::ExternalExtension(_) => "extensions",
            BinaryData::InternalExtension(_) => "extensions",
        }
    }

    /// Name of the parent directory to copy the binary into
    pub fn parent_dir(&self) -> Option<&str> {
        match self {
            BinaryData::ExternalExtension(_) => Some("extensions"),
            _ => None,
        }
    }

    /// Command to use to build each kind of binary
    pub fn build_help(&self) -> &str {
        match self {
            BinaryData::Function(_) => "build",
            BinaryData::ExternalExtension(_) => "build --extension",
            BinaryData::InternalExtension(_) => "build --extension --internal",
        }
    }
}

pub struct BinaryArchive {
    pub architecture: String,
    pub path: PathBuf,
}

impl BinaryArchive {
    /// Read the content of the binary archive to the end
    pub fn read(&self) -> Result<Vec<u8>> {
        read(&self.path)
            .into_diagnostic()
            .wrap_err("failed to read binary archive")
    }

    /// Calculate the SHA256 hash of the zip binary file
    pub fn sha256(&self) -> Result<String> {
        let data = self.read()?;
        let mut hasher = Sha256::new();
        hasher.update(data);
        let sha256 = format!("{:X}", hasher.finalize());
        Ok(sha256)
    }

    /// Add files to the zip archive
    pub fn add_files(&self, files: &Vec<PathBuf>) -> Result<()> {
        trace!(?self.path, ?files, "adding files to zip file");
        let zipfile = File::open(&self.path).into_diagnostic()?;

        let mut archive = ZipArchive::new(zipfile).into_diagnostic()?;

        // Open a new, empty archive for writing to
        let tmp_dir = tempfile::tempdir().into_diagnostic()?;
        let tmp_path = tmp_dir
            .path()
            .join(self.path.file_name().expect("missing zip file name"));
        let tmp = File::create(&tmp_path).into_diagnostic()?;
        let mut new_archive = ZipWriter::new(tmp);

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
        copy_and_replace(&tmp_path, &self.path).into_diagnostic()?;
        Ok(())
    }
}

/// Search for the bootstrap file for a function inside the target directory.
/// If the binary file exists, it creates the zip archive and extracts its architecture by reading the binary.
pub fn create_binary_archive<M, P>(
    manifest_path: M,
    base_dir: &Option<P>,
    data: &BinaryData,
    include: Option<Vec<PathBuf>>,
) -> Result<BinaryArchive>
where
    M: AsRef<Path> + Debug,
    P: AsRef<Path>,
{
    let bootstrap_dir = if let Some(dir) = base_dir {
        dir.as_ref().join(data.binary_location())
    } else {
        let target_dir = target_dir(manifest_path).unwrap_or_else(|_| PathBuf::from("target"));
        target_dir.join("lambda").join(data.binary_location())
    };

    let binary_path = bootstrap_dir.join(data.binary_name());
    if !binary_path.exists() {
        return Err(
            BuildError::BinaryMissing(data.binary_name().into(), data.build_help().into()).into(),
        );
    }

    zip_binary(binary_path, bootstrap_dir, data, include)
}

/// Create a zip file from a function binary.
/// The binary inside the zip file is called `bootstrap` for function binaries.
/// The binary inside the zip file is called by its name, and put inside the `extensions`
/// directory, for extension binaries.
pub fn zip_binary<BP: AsRef<Path>, DD: AsRef<Path>>(
    binary_path: BP,
    destination_directory: DD,
    data: &BinaryData,
    include: Option<Vec<PathBuf>>,
) -> Result<BinaryArchive> {
    let path = binary_path.as_ref();
    let dir = destination_directory.as_ref();

    let zipped = dir.join(data.zip_name());
    debug!(?data, ?path, ?dir, ?zipped, "zipping binary");

    let zipped_binary = File::create(&zipped)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to create zip file `{zipped:?}`"))?;

    let mut file = File::open(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to open binary file `{path:?}`"))?;

    let mut binary_data = Vec::new();
    file.read_to_end(&mut binary_data)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to read binary file `{path:?}`"))?;

    let binary_data = &*binary_data;
    let object = ObjectFile::parse(binary_data)
        .into_diagnostic()
        .wrap_err("the provided function file is not a valid Linux binary")?;

    let arch = match object.architecture() {
        Architecture::Aarch64 => "arm64",
        Architecture::X86_64 => "x86_64",
        other => return Err(BuildError::InvalidBinaryArchitecture(other).into()),
    };

    let mut zip = ZipWriter::new(zipped_binary);
    if let Some(files) = include {
        include_files_in_zip(&mut zip, &files)?;
    }

    let file_name = if let Some(parent) = data.parent_dir() {
        let options = SimpleFileOptions::default();
        zip.add_directory(parent, options)
            .into_diagnostic()
            .wrap_err_with(|| {
                format!("failed to add directory `{parent}` to zip file `{zipped:?}`")
            })?;
        Path::new(parent).join(data.binary_name())
    } else {
        PathBuf::from(data.binary_name())
    };

    let zip_file_name = convert_to_unix_path(&file_name)
        .ok_or_else(|| BuildError::InvalidUnixFileName(file_name.clone()))?;

    let options = zip_file_options(&file, path)?;

    zip.start_file(zip_file_name.to_string(), options)
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
    })
}

fn zip_file_options(file: &File, path: &Path) -> Result<SimpleFileOptions> {
    let meta = file
        .metadata()
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to get metadata from file `{path:?}`"))?;
    let perm = binary_permissions(&meta);
    let mtime = binary_mtime(&meta)?;

    Ok(SimpleFileOptions::default()
        .unix_permissions(perm)
        .last_modified_time(mtime))
}

fn include_files_in_zip<W>(zip: &mut ZipWriter<W>, files: &Vec<PathBuf>) -> Result<()>
where
    W: Write + Seek,
{
    for file in files {
        for entry in WalkDir::new(file).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            let entry_name = convert_to_unix_path(path)
                .ok_or_else(|| BuildError::InvalidUnixFileName(path.to_path_buf()))?;

            if path.is_dir() {
                trace!(?entry_name, "creating directory in zip file");

                zip.add_directory(entry_name.to_string(), SimpleFileOptions::default())
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

                let options = zip_file_options(&file, path)?;

                zip.start_file(entry_name.to_string(), options)
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

fn binary_mtime(meta: &Metadata) -> Result<zip::DateTime> {
    let modified = meta
        .modified()
        .into_diagnostic()
        .wrap_err("failed to get modified time")?;

    let dt: DateTime<Utc> = modified.into();
    zip::DateTime::try_from(dt.naive_utc())
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to convert `{modified:?}` to zip time"))
}

#[cfg(unix)]
fn binary_permissions(meta: &Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode()
}

#[cfg(not(unix))]
fn binary_permissions(_meta: &Metadata) -> u32 {
    0o755
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

#[cfg(test)]
mod test {
    use std::{
        fs::{create_dir_all, remove_dir_all},
        thread::sleep,
        time::Duration,
    };

    use cargo_lambda_metadata::fs::copy_without_replace;
    use rstest::rstest;
    use tempfile::TempDir;

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

    #[rstest]
    #[case("binary-x86-64", "x86_64")]
    #[case("binary-arm64", "arm64")]
    fn test_zip_funcion(#[case] name: &str, #[case] arch: &str) {
        let data = BinaryData::new(name, false, false);
        let bp = &format!("../../tests/binaries/{name}");
        let dd = TempDir::with_prefix("cargo-lambda-").expect("failed to create temp dir");
        let archive =
            zip_binary(bp, dd.path(), &data, None).expect("failed to create binary archive");

        assert_eq!(arch, archive.architecture);

        let arch_path = dd.path().join("bootstrap.zip");
        assert_eq!(arch_path, archive.path);

        let file = File::open(arch_path).expect("failed to open zip file");
        let mut zip = ZipArchive::new(file).expect("failed to open zip archive");

        zip.by_name("bootstrap")
            .expect("failed to find bootstrap in zip archive");
    }

    #[rstest]
    #[case("binary-x86-64", "x86_64")]
    #[case("binary-arm64", "arm64")]
    fn test_zip_extension(#[case] name: &str, #[case] arch: &str) {
        let data = BinaryData::new(name, true, false);

        let bp = &format!("../../tests/binaries/{name}");
        let dd = TempDir::with_prefix("cargo-lambda-").expect("failed to create temp dir");
        let archive =
            zip_binary(bp, dd.path(), &data, None).expect("failed to create binary archive");

        assert_eq!(arch, archive.architecture);

        let arch_path = dd.path().join(format!("{name}.zip"));
        assert_eq!(arch_path, archive.path);

        let file = File::open(arch_path).expect("failed to open zip file");
        let mut zip = ZipArchive::new(file).expect("failed to open zip archive");

        zip.by_name(&format!("extensions/{name}"))
            .expect("failed to find bootstrap in zip archive");
    }

    #[rstest]
    #[case("binary-x86-64", "x86_64")]
    #[case("binary-arm64", "arm64")]
    fn test_zip_internal_extension(#[case] name: &str, #[case] arch: &str) {
        let data = BinaryData::new(name, true, true);

        let bp = &format!("../../tests/binaries/{name}");
        let dd = TempDir::with_prefix("cargo-lambda-").expect("failed to create temp dir");
        let archive =
            zip_binary(bp, dd.path(), &data, None).expect("failed to create binary archive");

        assert_eq!(arch, archive.architecture);

        let arch_path = dd.path().join(format!("{name}.zip"));
        assert_eq!(arch_path, archive.path);

        let file = File::open(arch_path).expect("failed to open zip file");
        let mut zip = ZipArchive::new(file).expect("failed to open zip archive");

        zip.by_name(name)
            .unwrap_or_else(|_| panic!("failed to find {name} in zip archive"));
    }

    #[rstest]
    #[case("binary-x86-64", "x86_64")]
    #[case("binary-arm64", "arm64")]
    fn test_zip_funcion_with_files(#[case] name: &str, #[case] arch: &str) {
        let data = BinaryData::new(name, false, false);

        let bp = &format!("../../tests/binaries/{name}");
        let extra = vec!["Cargo.toml".into()];
        let dd = TempDir::with_prefix("cargo-lambda-").expect("failed to create temp dir");
        let archive =
            zip_binary(bp, dd.path(), &data, Some(extra)).expect("failed to create binary archive");

        assert_eq!(arch, archive.architecture);

        let arch_path = dd.path().join("bootstrap.zip");
        assert_eq!(arch_path, archive.path);

        let file = File::open(arch_path).expect("failed to open zip file");
        let mut zip = ZipArchive::new(file).expect("failed to open zip archive");

        zip.by_name("bootstrap")
            .expect("failed to find bootstrap in zip archive");

        zip.by_name("Cargo.toml")
            .expect("failed to find Cargo.toml in zip archive");
    }

    #[test]
    fn test_consistent_hash() {
        let data = BinaryData::new("binary-x86-64", false, false);

        let bp = "../../tests/binaries/binary-x86-64";
        let dd = TempDir::with_prefix("cargo-lambda-").expect("failed to create temp dir");

        let archive1 =
            zip_binary(bp, dd.path(), &data, None).expect("failed to create binary archive");

        // Sleep to ensure that the mtime is different enough for the hash to change
        sleep(Duration::from_secs(2));

        let archive2 =
            zip_binary(bp, dd.path(), &data, None).expect("failed to create binary archive");

        assert_eq!(archive1.sha256().unwrap(), archive2.sha256().unwrap());
    }

    #[test]
    fn test_add_files() {
        let data = BinaryData::new("binary-x86-64", false, false);

        let bp = "../../tests/binaries/binary-x86-64";
        let dd = TempDir::with_prefix("cargo-lambda-").expect("failed to create temp dir");

        let archive =
            zip_binary(bp, dd.path(), &data, None).expect("failed to create binary archive");

        let extra = vec!["Cargo.toml".into()];
        archive.add_files(&extra).expect("failed to add files");

        let arch_path = dd.path().join("bootstrap.zip");
        assert_eq!(arch_path, archive.path);

        let file = File::open(arch_path).expect("failed to open zip file");
        let mut zip = ZipArchive::new(file).expect("failed to open zip archive");

        zip.by_name("bootstrap")
            .expect("failed to find bootstrap in zip archive");

        zip.by_name("Cargo.toml")
            .expect("failed to find Cargo.toml in zip archive");
    }

    #[test]
    fn test_create_binary_archive_with_base_path() {
        let data = BinaryData::new("binary-x86-64", false, false);

        let bp = "../../tests/binaries/binary-x86-64";
        let dd = TempDir::with_prefix("cargo-lambda-").expect("failed to create temp dir");
        let bsp = dd.path().join("binary-x86-64");

        create_dir_all(&bsp).expect("failed to create dir");
        copy_without_replace(bp, bsp.join("bootstrap")).expect("failed to copy bootstrap file");

        let archive = create_binary_archive("Cargo.toml", &Some(dd.path()), &data, None)
            .expect("failed to create binary archive");

        let arch_path = bsp.join("bootstrap.zip");
        assert_eq!(arch_path, archive.path);

        let file = File::open(arch_path).expect("failed to open zip file");
        let mut zip = ZipArchive::new(file).expect("failed to open zip archive");

        zip.by_name("bootstrap")
            .expect("failed to find bootstrap in zip archive");
    }

    #[test]
    fn test_create_binary_archive_from_target() {
        let data = BinaryData::new("binary-x86-64", false, false);

        let bp = "../../tests/binaries/binary-x86-64";
        let target_dir = target_dir("Cargo.toml").unwrap_or_else(|_| PathBuf::from("target"));

        let bsp = target_dir.join("lambda").join("binary-x86-64");

        create_dir_all(&bsp).expect("failed to create dir");
        copy_without_replace(bp, bsp.join("bootstrap")).expect("failed to copy bootstrap file");

        let base_dir: Option<&Path> = None;
        let archive = create_binary_archive("Cargo.toml", &base_dir, &data, None)
            .expect("failed to create binary archive");

        let arch_path = bsp.join("bootstrap.zip");
        assert_eq!(arch_path, archive.path);

        let file = File::open(arch_path).expect("failed to open zip file");
        let mut zip = ZipArchive::new(file).expect("failed to open zip archive");

        zip.by_name("bootstrap")
            .expect("failed to find bootstrap in zip archive");

        remove_dir_all(&bsp).expect("failed to delete dir");
    }
}
