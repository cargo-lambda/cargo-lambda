use cargo_lambda_interactive::{command::silent_command, progress::Progress};
use home::cargo_home;
use miette::{IntoDiagnostic, Result, WrapErr};
use platforms::platform::{self, Platform};
use reqwest::{get, StatusCode};
use std::{
    fs::{rename, File},
    io::{copy, Cursor},
    path::Path,
};
#[cfg(not(target_os = "windows"))]
use tar::Archive;
use tempfile::tempdir;
#[cfg(not(target_os = "windows"))]
use xz2::read::XzDecoder;
#[cfg(target_os = "windows")]
use zip::ZipArchive;

const DEFAULT_CARGO_WATCH_VERSION: &str = "v8.1.1";
const CARGO_WATCH_GITHUB_RELEASES_URL: &str =
    "https://github.com/watchexec/cargo-watch/releases/download";

pub(crate) async fn install() -> Result<()> {
    let pb = Progress::start("Installing cargo-watch...");

    let result = match Platform::guess_current() {
        Some(plat) if is_matching_platform(plat.target_triple) => {
            download_binary_release(plat).await
        }
        _ => silent_command("cargo", &["install", "cargo-watch"]).await,
    };

    let finish = if result.is_ok() {
        "cargo-watch installed"
    } else {
        "Failed to install cargo-watch"
    };
    pb.finish(finish);
    result
}

fn is_matching_platform(triple: &str) -> bool {
    triple == platform::AARCH64_APPLE_DARWIN.target_triple
        || triple == platform::X86_64_APPLE_DARWIN.target_triple
        || triple == platform::AARCH64_BE_UNKNOWN_LINUX_GNU.target_triple
        || triple == platform::X86_64_UNKNOWN_LINUX_GNU.target_triple
        || triple == platform::AARCH64_PC_WINDOWS_MSVC.target_triple
        || triple == platform::X86_64_PC_WINDOWS_MSVC.target_triple
}

async fn download_binary_release(plat: &Platform) -> Result<()> {
    let version =
        option_env!("CARGO_LAMBDA_CARGO_WATCH_VERSION").unwrap_or(DEFAULT_CARGO_WATCH_VERSION);
    let (bin_name, format) = if cfg!(target_os = "windows") {
        ("cargo-watch.exe", "zip")
    } else {
        ("cargo-watch", "tar.xz")
    };

    let name = format!("cargo-watch-{}-{}.{}", &version, plat.target_triple, format);
    let url = format!("{}/{}/{}", CARGO_WATCH_GITHUB_RELEASES_URL, version, &name);

    let response = get(&url).await.into_diagnostic()?;

    if response.status() != StatusCode::OK {
        return Err(miette::miette!(
            "error downloading cargo-watch binary from {} - {}",
            url,
            response.text().await.into_diagnostic()?
        ));
    }

    let mut bytes = Cursor::new(response.bytes().await.into_diagnostic()?);

    let tmp_dir = tempdir().into_diagnostic()?;
    let tmp_path = tmp_dir.path();
    let tmp_file = tmp_path.join(&name);

    let mut writer = File::create(&tmp_file)
        .into_diagnostic()
        .wrap_err_with(|| format!("unable to create file: {:?}", &tmp_file))?;
    copy(&mut bytes, &mut writer).into_diagnostic()?;

    let reader = File::open(&tmp_file)
        .into_diagnostic()
        .wrap_err_with(|| format!("unable to open file: {:?}", &tmp_file))?;

    extract_file(reader, tmp_path)?;

    let target_file = cargo_home()
        .into_diagnostic()
        .wrap_err("missing cargo home")?
        .join("bin")
        .join(bin_name);

    let original_file = tmp_path
        .join(&format!("cargo-watch-{}-{}", &version, plat.target_triple))
        .join(bin_name);

    rename(&original_file, &target_file)
        .into_diagnostic()
        .wrap_err_with(|| {
            format!(
                "unable to rename file {:?} to {:?}",
                original_file, target_file
            )
        })
}

#[cfg(target_os = "windows")]
fn extract_file(file: File, path: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(file).into_diagnostic()?;
    archive.extract(path).into_diagnostic()
}

#[cfg(not(target_os = "windows"))]
fn extract_file(file: File, path: &Path) -> Result<()> {
    let mut archive = Archive::new(XzDecoder::new(file));
    archive.unpack(path).into_diagnostic()
}
