use std::{
    collections::HashMap,
    fmt::Debug,
    fs::{File, Metadata, read},
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use cargo_lambda_metadata::{
    cargo::{CargoMetadata, target_dir_from_metadata},
    fs::copy_and_replace,
};
use cargo_lambda_remote::aws_sdk_lambda::types::Architecture as CpuArchitecture;
use chrono::{DateTime, NaiveDateTime, Utc};
use chrono_humanize::HumanTime;
use miette::{Context, IntoDiagnostic, Result};
use object::{Architecture, Object, read::File as ObjectFile};
use serde::{Serialize, Serializer};
use sha2::{Digest, Sha256};
use tracing::{debug, trace};
use walkdir::WalkDir;
use zip::{HasZipMetadata, ZipArchive, ZipWriter, write::SimpleFileOptions};

use crate::error::BuildError;

#[derive(Clone, Debug)]
pub struct BinaryModifiedAt(Option<SystemTime>);

impl BinaryModifiedAt {
    pub fn now() -> Self {
        Self(Some(SystemTime::now()))
    }
}

impl From<NaiveDateTime> for BinaryModifiedAt {
    fn from(value: NaiveDateTime) -> Self {
        let dt = DateTime::<Utc>::from_naive_utc_and_offset(value, Utc);
        let Some(timestamp) = dt.timestamp_nanos_opt() else {
            return Self(None);
        };

        let system_time = SystemTime::UNIX_EPOCH + Duration::from_nanos(timestamp as u64);
        Self(Some(system_time))
    }
}

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
            BinaryData::ExternalExtension(name) | BinaryData::InternalExtension(name) => name,
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
            BinaryData::ExternalExtension(_) | BinaryData::InternalExtension(_) => "extensions",
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

    pub(crate) fn binary_path_in_zip(&self) -> Result<String, BuildError> {
        let file_name = if let Some(parent) = self.parent_dir() {
            Path::new(parent).join(self.binary_name())
        } else {
            PathBuf::from(self.binary_name())
        };

        convert_to_unix_path(&file_name)
            .ok_or_else(|| BuildError::InvalidUnixFileName(file_name.clone()))
    }
}

pub struct BinaryArchive {
    pub architecture: String,
    pub path: PathBuf,
    pub binary_modified_at: BinaryModifiedAt,
}

impl BinaryArchive {
    pub fn new(path: PathBuf, architecture: String, binary_modified_at: BinaryModifiedAt) -> Self {
        Self {
            path,
            architecture,
            binary_modified_at,
        }
    }

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

    /// List the files inside the zip archive
    pub fn list(&self) -> Result<Vec<String>> {
        let zipfile = File::open(&self.path).into_diagnostic()?;
        let mut archive = ZipArchive::new(zipfile).into_diagnostic()?;

        let mut files = Vec::new();
        for i in 0..archive.len() {
            let entry = archive.by_index(i).into_diagnostic()?;
            files.push(entry.name().to_string());
        }

        Ok(files)
    }

    /// Get the architecture of the binary archive
    pub fn architecture(&self) -> CpuArchitecture {
        CpuArchitecture::from(self.architecture.as_str())
    }
}

/// Search for the binary file for a function or extension inside the target directory.
/// If the binary file exists, it creates the zip archive and extracts its architecture by reading the binary.
/// If the zip file already exists, use it as the deployment archive.
/// If none of them exist, return an error.
pub fn create_binary_archive<P>(
    metadata: Option<&CargoMetadata>,
    base_dir: &Option<P>,
    data: &BinaryData,
    include: Option<Vec<String>>,
) -> Result<BinaryArchive>
where
    P: AsRef<Path>,
{
    let bootstrap_dir = if let Some(dir) = base_dir {
        dir.as_ref().join(data.binary_location())
    } else {
        let target_dir = metadata
            .and_then(|m| target_dir_from_metadata(m).ok())
            .unwrap_or_else(|| PathBuf::from("target"));

        target_dir.join("lambda").join(data.binary_location())
    };

    let binary_path = bootstrap_dir.join(data.binary_name());
    if binary_path.exists() {
        return zip_binary(binary_path, bootstrap_dir, data, include);
    } else {
        let zip_path = bootstrap_dir.join(data.zip_name());

        if zip_path.exists() {
            return use_zip_in_place(zip_path, data, include);
        }
    }

    Err(BuildError::BinaryMissing(data.binary_name().into(), data.build_help().into()).into())
}

fn use_zip_in_place(
    zip_path: PathBuf,
    data: &BinaryData<'_>,
    include: Option<Vec<String>>,
) -> Result<BinaryArchive> {
    let binary_path_in_zip = data.binary_path_in_zip()?;
    let (arch, binary_modified_at) =
        extract_data_from_zipped_binary(&zip_path, &binary_path_in_zip)?;

    if let Some(files) = include {
        let file = File::open(&zip_path)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to open zip file `{zip_path:?}`"))?;

        // Zip2 doesn't support updating zip files in place, so we need to create a new zip file
        // if we want to include files in the zip archive.
        // Before we do that, we need to read the existing zip file and copy its contents to a new zip file.
        let mut archive = ZipArchive::new(file).into_diagnostic()?;

        let tmp_dir = tempfile::tempdir().into_diagnostic()?;
        let tmp_path = tmp_dir.path().join(data.zip_name());
        let tmp = File::create(&tmp_path).into_diagnostic()?;
        let mut zip = ZipWriter::new(tmp);

        for i in 0..archive.len() {
            let file = archive.by_index_raw(i).into_diagnostic()?;
            zip.raw_copy_file(file).into_diagnostic()?;
        }

        include_files_in_zip(&mut zip, &files)?;

        zip.finish()
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to finish zip file `{}`", tmp_path.display()))?;

        drop(archive);
        copy_and_replace(&tmp_path, &zip_path).into_diagnostic()?;
    }

    Ok(BinaryArchive::new(
        zip_path.clone(),
        arch.to_string(),
        binary_modified_at,
    ))
}

/// Create a zip file from a function binary.
/// The binary inside the zip file is called `bootstrap` for function binaries.
/// The binary inside the zip file is called by its name, and put inside the `extensions`
/// directory, for extension binaries.
pub fn zip_binary<BP: AsRef<Path>, DD: AsRef<Path>>(
    binary_path: BP,
    destination_directory: DD,
    data: &BinaryData,
    include: Option<Vec<String>>,
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

    let file_metadata = file
        .metadata()
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to get metadata from file `{path:?}`"))?;

    let binary_modified_at = file_metadata
        .modified()
        .ok()
        .or_else(|| file_metadata.created().ok());

    let (binary_data, arch) = read_binary(&mut file, path)?;

    let mut zip = ZipWriter::new(zipped_binary);
    if let Some(files) = include {
        include_files_in_zip(&mut zip, &files)?;
    }

    if let Some(parent) = data.parent_dir() {
        let options = SimpleFileOptions::default();
        zip.add_directory(parent, options)
            .into_diagnostic()
            .wrap_err_with(|| {
                format!("failed to add directory `{parent}` to zip file `{zipped:?}`")
            })?;
    }

    let binary_path_in_zip = data.binary_path_in_zip()?;

    let options = zip_file_options(&file, path)?;

    zip.start_file(binary_path_in_zip.to_string(), options)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to start zip file `{binary_path_in_zip:?}`"))?;
    zip.write_all(&binary_data)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to write data into zip file `{binary_path_in_zip:?}`"))?;
    zip.finish()
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to finish zip file `{binary_path_in_zip:?}`"))?;

    Ok(BinaryArchive::new(
        zipped,
        arch.to_string(),
        BinaryModifiedAt(binary_modified_at),
    ))
}

fn extract_data_from_zipped_binary<'a>(
    zip_path: &'a Path,
    binary_path: &'a str,
) -> Result<(&'a str, BinaryModifiedAt)> {
    let zipfile = File::open(zip_path).into_diagnostic()?;
    let mut archive = ZipArchive::new(zipfile).into_diagnostic()?;

    let mut file = archive.by_name(binary_path).into_diagnostic()?;
    let (_, arch) = read_binary(&mut file, binary_path)?;

    let metadata = file.get_metadata();
    let mut last_modified_at = BinaryModifiedAt(None);
    if let Some(dt) = metadata.last_modified_time {
        let naive_dt: NaiveDateTime = dt.try_into().into_diagnostic()?;
        last_modified_at = naive_dt.into();
    }

    Ok((arch, last_modified_at))
}

fn read_binary<'a, R: Read, D: std::fmt::Debug>(
    entry: &mut R,
    binary_path: D,
) -> Result<(Vec<u8>, &'a str)> {
    let mut binary_data = Vec::new();

    entry
        .read_to_end(&mut binary_data)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to read binary file `{binary_path:?}`"))?;

    let object = ObjectFile::parse(&*binary_data)
        .into_diagnostic()
        .wrap_err("the provided function file is not a valid Linux binary")?;

    let arch = match object.architecture() {
        Architecture::Aarch64 => "arm64",
        Architecture::X86_64 => "x86_64",
        other => return Err(BuildError::InvalidBinaryArchitecture(other).into()),
    };

    Ok((binary_data, arch))
}

fn zip_file_options(file: &File, path: &Path) -> Result<SimpleFileOptions> {
    let meta = file
        .metadata()
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to get metadata from file `{path:?}`"))?;
    let perm = binary_permissions(&meta);
    let mut options = SimpleFileOptions::default().unix_permissions(perm);
    if let Some(mtime) = binary_mtime(&meta) {
        options = options.last_modified_time(mtime);
    }

    Ok(options)
}

fn include_files_in_zip<W: Write + Seek>(
    zip: &mut ZipWriter<W>,
    files: &Vec<String>,
) -> Result<()> {
    let mut file_map = HashMap::with_capacity(files.len());
    for file in files {
        match file.split_once(':') {
            None => file_map.insert(file.clone(), file.clone()),
            Some((name, path)) => file_map.insert(name.into(), path.into()),
        };
    }

    for (base, file) in file_map {
        for entry in WalkDir::new(&file).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            let base = base.clone();
            let file = file.clone();

            let unix_base = convert_to_unix_path(Path::new(&base))
                .ok_or_else(|| BuildError::InvalidUnixFileName(base.into()))?;
            let unix_file = convert_to_unix_path(Path::new(&file))
                .ok_or_else(|| BuildError::InvalidUnixFileName(file.into()))?;

            let source_name = convert_to_unix_path(path)
                .ok_or_else(|| BuildError::InvalidUnixFileName(path.into()))?;

            let destination_name = source_name.replace(&unix_file, &unix_base);

            if path.is_dir() {
                trace!(%destination_name, "creating directory in zip file");

                zip.add_directory(&destination_name, SimpleFileOptions::default())
                    .into_diagnostic()
                    .wrap_err_with(|| {
                        format!("failed to add directory `{destination_name}` to zip file")
                    })?;
            } else {
                trace!(%source_name, %destination_name, "including file in zip file");

                let mut content = Vec::new();
                let mut file = File::open(path)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("failed to open file `{path:?}`"))?;
                file.read_to_end(&mut content)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("failed to read file `{path:?}`"))?;

                let options = zip_file_options(&file, path)?;

                zip.start_file(destination_name.clone(), options)
                    .into_diagnostic()
                    .wrap_err_with(|| {
                        format!("failed to create zip content file `{destination_name:?}`")
                    })?;

                zip.write_all(&content)
                    .into_diagnostic()
                    .wrap_err_with(|| {
                        format!("failed to write data into zip content file `{destination_name:?}`")
                    })?;
            }
        }
    }
    Ok(())
}

fn binary_mtime(meta: &Metadata) -> Option<zip::DateTime> {
    let Ok(modified) = meta.modified() else {
        return None;
    };

    let dt: DateTime<Utc> = modified.into();
    if let Ok(dt) = zip::DateTime::try_from(dt.naive_utc()) {
        return Some(dt);
    }

    let Ok(created) = meta.created() else {
        return None;
    };

    let dt: DateTime<Utc> = created.into();
    zip::DateTime::try_from(dt.naive_utc()).ok()
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
fn convert_to_unix_path(path: &Path) -> Option<String> {
    let mut path_str = String::new();
    for component in path.components() {
        if let std::path::Component::Normal(os_str) = component {
            if !path_str.is_empty() {
                path_str.push('/');
            }
            path_str.push_str(os_str.to_str()?);
        }
    }
    Some(path_str)
}

#[cfg(not(target_os = "windows"))]
fn convert_to_unix_path(path: &Path) -> Option<String> {
    path.to_str().map(String::from)
}

impl BinaryModifiedAt {
    pub fn humanize(&self) -> String {
        match self.0 {
            Some(time) => HumanTime::from(time).to_string(),
            None => "at unknown time".to_string(),
        }
    }
}

impl Serialize for BinaryModifiedAt {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self.0 {
            Some(time) => serializer.serialize_str(&HumanTime::from(time).to_string()),
            None => serializer.serialize_none(),
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        fs::{create_dir_all, remove_dir_all},
        thread::sleep,
        time::Duration,
    };

    use cargo_lambda_metadata::{cargo::load_metadata, fs::copy_without_replace};
    use rstest::rstest;
    use tempfile::TempDir;
    use zip::ZipArchive;

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
    fn test_create_binary_archive_with_base_path() {
        let data = BinaryData::new("binary-x86-64", false, false);

        let bp = "../../tests/binaries/binary-x86-64";
        let dd = TempDir::with_prefix("cargo-lambda-").expect("failed to create temp dir");
        let bsp = dd.path().join("binary-x86-64");

        create_dir_all(&bsp).expect("failed to create dir");
        copy_without_replace(bp, bsp.join("bootstrap")).expect("failed to copy bootstrap file");

        let archive = create_binary_archive(None, &Some(dd.path()), &data, None)
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
        let metadata = load_metadata("Cargo.toml").unwrap();
        let target_dir =
            target_dir_from_metadata(&metadata).unwrap_or_else(|_| PathBuf::from("target"));

        let bsp = target_dir.join("lambda").join("binary-x86-64");

        create_dir_all(&bsp).expect("failed to create dir");
        copy_without_replace(bp, bsp.join("bootstrap")).expect("failed to copy bootstrap file");

        let base_dir: Option<&Path> = None;
        let archive = create_binary_archive(Some(&metadata), &base_dir, &data, None)
            .expect("failed to create binary archive");

        let arch_path = bsp.join("bootstrap.zip");
        assert_eq!(arch_path, archive.path);

        let file = File::open(arch_path).expect("failed to open zip file");
        let mut zip = ZipArchive::new(file).expect("failed to open zip archive");

        zip.by_name("bootstrap")
            .expect("failed to find bootstrap in zip archive");

        remove_dir_all(&bsp).expect("failed to delete dir");
    }

    #[test]
    fn test_zip_funcion_with_parent_directories() {
        let data = BinaryData::new("binary-x86-64", false, false);

        let bp = "../../tests/binaries/binary-x86-64".to_string();
        #[cfg(unix)]
        let extra = vec!["source:../../tests/fixtures/examples-package".into()];
        #[cfg(windows)]
        let extra = vec!["source:..\\..\\tests\\fixtures\\examples-package".into()];

        let dd = TempDir::with_prefix("cargo-lambda-").expect("failed to create temp dir");
        let archive =
            zip_binary(bp, dd.path(), &data, Some(extra)).expect("failed to create binary archive");

        let arch_path = dd.path().join("bootstrap.zip");
        assert_eq!(arch_path, archive.path);

        let file = File::open(arch_path).expect("failed to open zip file");
        let mut zip = ZipArchive::new(file).expect("failed to open zip archive");

        zip.by_name("bootstrap")
            .expect("failed to find bootstrap in zip archive");

        zip.by_name("source/Cargo.toml")
            .expect("failed to find Cargo.toml in zip archive");

        zip.by_name("source/Cargo.lock")
            .expect("failed to find Cargo.lock in zip archive");

        zip.by_name("source/src/main.rs")
            .expect("failed to find source/src/main.rs in zip archive");

        zip.by_name("source/examples/example-lambda.rs")
            .expect("failed to find source/examples/example-lambda.rs in zip archive");
    }

    #[rstest]
    #[case("bootstrap.zip", "bootstrap")]
    #[case("test-extension.zip", "extensions/test-extension")]
    fn test_extract_data_from_zipped_bootstrap(#[case] name: &str, #[case] binary_path: &str) {
        let zip_path = format!("../../tests/binaries/{name}");
        let zip_path = Path::new(&zip_path);
        let (arch, binary_modified_at) =
            extract_data_from_zipped_binary(zip_path, binary_path).unwrap();

        assert_eq!("x86_64", arch);
        assertables::assert_some!(binary_modified_at.0);
    }
}
