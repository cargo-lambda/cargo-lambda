use cargo_lambda_interactive::{command::silent_command, progress::Progress};
use home::cargo_home;
use miette::{IntoDiagnostic, Result, WrapErr};
use platforms::{
    platform::{self, Platform},
    target::OS,
};
use reqwest::{get, StatusCode};
use std::{
    fs::{rename, File},
    io::{copy, Cursor},
};
use tar::Archive;
use tempfile::tempdir;
use xz2::read::XzDecoder;
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
    let format = if plat.target_os == OS::Windows {
        "zip"
    } else {
        "tar.xz"
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

    let mut writer = File::create(&tmp_file).into_diagnostic()?;
    copy(&mut bytes, &mut writer).into_diagnostic()?;

    let reader = File::open(tmp_file).into_diagnostic()?;
    if plat.target_os == OS::Windows {
        let mut archive = ZipArchive::new(reader).into_diagnostic()?;
        archive.extract(&tmp_path).into_diagnostic()?;
    } else {
        let tar = XzDecoder::new(reader);
        let mut archive = Archive::new(tar);
        archive.unpack(&tmp_path).into_diagnostic()?;
    }

    let target_file = cargo_home()
        .into_diagnostic()
        .wrap_err("missing cargo home")?
        .join("bin")
        .join("cargo-watch");

    let original_file = tmp_path.join(&format!(
        "cargo-watch-{}-{}/cargo-watch",
        &version, plat.target_triple
    ));

    rename(original_file, target_file).into_diagnostic()
}
