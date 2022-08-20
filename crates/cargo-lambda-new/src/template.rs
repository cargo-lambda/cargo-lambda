use std::{
    fs::{read_dir, rename, File},
    io::{copy, Cursor},
    path::{Path, PathBuf},
};

use miette::{Context, IntoDiagnostic, Result};
use tempfile::{tempdir, TempDir};
use walkdir::WalkDir;
use zip::ZipArchive;

/// Enum describing the various places a template can come from.  Implements the
/// logic to expand the template onto the local filesystem, downloading and
/// unzipping where necessary.
pub(crate) enum TemplateSource {
    /// ZIP stored remotely at the provided URL
    RemoteZip(String),
    /// ZIP stored locally at the provided path
    LocalZip(PathBuf),
    /// Local directory structure rooted at the provided path
    LocalDir(PathBuf),
}

impl TemplateSource {
    pub(crate) async fn expand(&self) -> Result<TemplateRoot> {
        match self {
            Self::RemoteZip(url) => {
                let tmp_dir = tempdir().into_diagnostic()?;

                let local_zip = download_template(url, tmp_dir.path()).await?;
                unzip_template(&local_zip, tmp_dir.path())?;
                Ok(TemplateRoot::Tmp(tmp_dir))
            }
            Self::LocalZip(path) => {
                let tmp_dir = tempdir().into_diagnostic()?;

                unzip_template(path, tmp_dir.path())?;
                Ok(TemplateRoot::Tmp(tmp_dir))
            }
            Self::LocalDir(path) => Ok(TemplateRoot::Dir(path.clone())),
        }
    }
}

impl TryFrom<&str> for TemplateSource {
    type Error = miette::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if is_remote_zip_file(value) {
            return Ok(Self::RemoteZip(value.into()));
        }

        if let Some(url) = match_github_url(value) {
            return Ok(Self::RemoteZip(url));
        }

        if is_local_zip_file(value) {
            return Ok(Self::LocalZip(value.into()));
        }

        if is_local_directory(value) {
            return Ok(Self::LocalDir(value.into()));
        }

        Err(miette::miette!("invalid template: {value}"))
    }
}

/// Represents the local filesystem root of the template, downloaded
/// and unzipped.  We model this as its own thing because we need to
/// pass the root directory back to the caller and optionally keep
/// the tempdir reference alive, dropping it and deleting it when
/// it goes out of the caller's scope.
pub(crate) enum TemplateRoot {
    Tmp(TempDir),
    Dir(PathBuf),
}

impl AsRef<Path> for TemplateRoot {
    fn as_ref(&self) -> &Path {
        match self {
            Self::Tmp(d) => d.path(),
            Self::Dir(d) => d,
        }
    }
}

#[tracing::instrument(level = "debug", target = "cargo_lambda")]
async fn download_template(url: &str, template_root: &Path) -> Result<PathBuf> {
    tracing::debug!("downloading template");

    let response = reqwest::get(url).await.into_diagnostic()?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(miette::miette!(
            "error downloading template from {} - {}",
            url,
            response.text().await.into_diagnostic()?
        ));
    }

    let mut bytes = Cursor::new(response.bytes().await.into_diagnostic()?);

    let tmp_file = template_root.join("cargo-lambda-template.zip");
    let mut writer = File::create(&tmp_file)
        .into_diagnostic()
        .wrap_err_with(|| format!("unable to create file: {:?}", &tmp_file))?;
    copy(&mut bytes, &mut writer).into_diagnostic()?;

    Ok(tmp_file)
}

#[tracing::instrument(level = "debug", target = "cargo_lambda")]
fn unzip_template(file: &Path, path: &Path) -> Result<PathBuf> {
    tracing::debug!("extracting template from ZIP file");

    let reader = File::open(file)
        .into_diagnostic()
        .wrap_err_with(|| format!("unable to open ZIP file: {:?}", file))?;

    let mut archive = ZipArchive::new(reader).into_diagnostic()?;
    archive.extract(path).into_diagnostic()?;

    if !path.join("Cargo.toml").exists() {
        // Try to find the template files in a subdirectory.
        // GitHub puts all the files inside a subdirectory
        // named after the repository and the branch that you're downloading.
        let mut base_path = None;
        let walk_dir = WalkDir::new(path).follow_links(false);
        for entry in walk_dir {
            let entry = entry.into_diagnostic()?;
            let entry_path = entry.path();
            if entry_path.is_dir() && entry_path.join("Cargo.toml").exists() {
                base_path = Some(entry_path.to_path_buf());
                break;
            }
        }

        if let Some(base_path) = base_path {
            for entry in read_dir(base_path).into_diagnostic()? {
                let entry = entry.into_diagnostic()?;
                let entry_path = entry.path();
                let entry_name = entry_path
                    .file_name()
                    .ok_or_else(|| miette::miette!("invalid entry: {:?}", &entry_path))?;
                let new_path = path.join(entry_name);
                rename(&entry_path, &new_path)
                    .into_diagnostic()
                    .wrap_err_with(|| {
                        format!(
                            "failed to move template file: from {:?} to {:?}",
                            &entry_path, &new_path
                        )
                    })?;
            }
        }
    }

    Ok(path.into())
}

fn is_local_directory(path: &str) -> bool {
    let path = Path::new(path);
    path.exists() && path.is_dir()
}

fn is_remote_zip_file(path: &str) -> bool {
    path.starts_with("https://") && path.ends_with(".zip")
}

fn is_local_zip_file(path: &str) -> bool {
    let path = Path::new(path);
    path.exists() && path.is_file() && path.extension().unwrap_or_default() == "zip"
}

fn match_github_url(path: &str) -> Option<String> {
    let github_repo_regex = regex::Regex::new(
        r"https://github.com/(?P<repo>[a-zA-Z0-9][a-zA-Z0-9_-]+/[a-zA-Z0-9][a-zA-Z0-9_-]+)/?((?P<kind>branch|tag)/(?P<ref>.+))?$",
    )
    .into_diagnostic()
    .unwrap();

    github_repo_regex.captures(path).map(|caps| {
        format!(
            "https://github.com/{}/archive/refs/{}/{}.zip",
            &caps["repo"],
            &caps
                .name("kind")
                .map(|m| m.as_str())
                .map(|m| if m == "tag" { "tags" } else { "heads" })
                .unwrap_or("heads"),
            &caps
                .name("ref")
                .map(|m| m.as_str().trim_end_matches('/').replace('/', "-"))
                .unwrap_or_else(|| "main".into())
        )
    })
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_is_github_url() {
        assert_eq!(
            Some("https://github.com/cargo-lambda/cargo-lambda/archive/refs/heads/main.zip".into()),
            match_github_url("https://github.com/cargo-lambda/cargo-lambda")
        );
        assert_eq!(
            Some("https://github.com/cargo-lambda/cargo-lambda/archive/refs/heads/main.zip".into()),
            match_github_url("https://github.com/cargo-lambda/cargo-lambda/")
        );
        assert_eq!(
            Some("https://github.com/cargo-lambda/cargo-lambda/archive/refs/heads/branch-with-slashes.zip".into()),
            match_github_url("https://github.com/cargo-lambda/cargo-lambda/branch/branch/with/slashes")
        );
        assert_eq!(
            Some(
                "https://github.com/cargo-lambda/cargo-lambda/archive/refs/tags/v0.1.0.zip".into()
            ),
            match_github_url("https://github.com/cargo-lambda/cargo-lambda/tag/v0.1.0")
        );
        assert_eq!(None, match_github_url("https://github.com/cargo-lambda"));
        assert_eq!(
            None,
            match_github_url("https://gitlab.com/cargo-lambda/cargo-lambda")
        );
    }
}
