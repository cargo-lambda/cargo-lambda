use std::{
    fmt,
    fs::{remove_dir_all, remove_file, File},
    io::{copy, Cursor},
    path::{Path, PathBuf},
};

use gix::refs::PartialName;
use miette::{Context, IntoDiagnostic, Result};
use tempfile::{tempdir, TempDir};
use zip::ZipArchive;

pub(crate) const PROMPT_WITH_OPTIONS_HELP_MESSAGE: &str =
    "use arrows (↑↓) to move, tab to auto-complete, enter to submit";

pub(crate) mod config;

#[derive(Debug, Default, PartialEq)]
pub(crate) enum GitProtocol {
    #[default]
    Http,
    Ssh,
}

impl fmt::Display for GitProtocol {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Http => write!(f, "https"),
            Self::Ssh => write!(f, "ssh"),
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct GitRepo {
    host: String,
    repo: String,
    reference: Option<String>,
    auth_user: Option<String>,
    protocol: GitProtocol,
}

impl GitRepo {
    pub(crate) fn to_url(&self) -> String {
        format!("{}://{}/{}", self.protocol, self.host, self.repo)
    }
}

/// Enum describing the various places a template can come from.  Implements the
/// logic to expand the template onto the local filesystem, downloading and
/// unzipping where necessary.
#[derive(Debug, PartialEq)]
pub(crate) enum TemplateSource {
    /// ZIP stored remotely at the provided URL
    RemoteZip(String),
    /// Remote repository
    RemoteRepo(GitRepo),
    /// ZIP stored locally at the provided path
    LocalZip(PathBuf),
    /// Local directory structure rooted at the provided path
    LocalDir(PathBuf),
}

impl TemplateSource {
    #[tracing::instrument(target = "cargo_lambda")]
    pub(crate) async fn expand(&self) -> Result<TemplateRoot> {
        tracing::debug!("expanding template");

        let base = match self {
            Self::RemoteZip(url) => {
                let tmp_dir = tempdir().into_diagnostic()?;

                let local_zip = download_zip_template(url, tmp_dir.path()).await?;
                unzip_template(&local_zip, tmp_dir.path())?;

                let base = adjust_remote_zip_base(url, tmp_dir.path());
                tracing::trace!(?base, "adjusted remote zip base");
                TemplateRoot::TempDir((tmp_dir, base))
            }
            Self::LocalZip(path) => {
                let tmp_dir = tempdir().into_diagnostic()?;

                unzip_template(path, tmp_dir.path())?;
                TemplateRoot::TempDir((tmp_dir, None))
            }
            Self::LocalDir(path) => TemplateRoot::LocalDir(path.clone()),
            Self::RemoteRepo(repo) => {
                let tmp_dir = tempdir().into_diagnostic()?;

                clone_git_repo(repo, tmp_dir.path())?;
                TemplateRoot::TempDir((tmp_dir, None))
            }
        };

        Ok(base)
    }
}

impl TryFrom<&str> for TemplateSource {
    type Error = miette::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if is_remote_zip_file(value) {
            return Ok(Self::RemoteZip(value.into()));
        }

        if let Some(repo) = match_git_http_url(value) {
            return Ok(Self::RemoteRepo(repo));
        }

        if let Some(repo) = match_git_ssh_url(value) {
            return Ok(Self::RemoteRepo(repo));
        }

        if !(value.starts_with("https://")) {
            if let Some(path) = find_local_zip_file(value) {
                return Ok(Self::LocalZip(path));
            }

            let path = find_local_directory(value)?;
            return Ok(Self::LocalDir(path));
        }

        Err(miette::miette!(
            "the given template option is not a valid Git URL or local directory: {value}"
        ))
    }
}

pub(crate) enum TemplateRoot {
    TempDir((TempDir, Option<PathBuf>)),
    LocalDir(PathBuf),
}

impl TemplateRoot {
    fn path(&self) -> &Path {
        match self {
            Self::TempDir((_tmp, Some(base))) => base,
            Self::TempDir((tmp, None)) => tmp.path(),
            Self::LocalDir(path) => path,
        }
    }

    pub(crate) fn final_path(&self) -> PathBuf {
        let path = self.path();
        Self::template_path(path)
    }

    /// Cleans up the temporary directories if they exist.
    pub(crate) fn cleanup(&self) {
        match self {
            Self::TempDir((tmp, _)) => cleanup_tmp_dir(tmp.path()),
            Self::LocalDir(_) => {}
        }
    }

    /// Returns the path to the directory containing the CargoLambda.toml configuration file.
    /// If the file exists at the root level, returns the root path.
    /// Otherwise returns the template subdirectory path.
    pub(crate) fn config_path(&self) -> PathBuf {
        let path = self.path();
        let config = path.join("CargoLambda.toml");
        if config.exists() && config.is_file() {
            return path.to_path_buf();
        }

        Self::template_path(path)
    }

    /// Returns the path to the template subdirectory if it exists.
    /// Otherwise returns the root path.
    fn template_path(path: &Path) -> PathBuf {
        let template_subdir = path.join("template");
        if template_subdir.exists() && template_subdir.is_dir() {
            return template_subdir;
        }

        path.into()
    }
}

#[tracing::instrument(target = "cargo_lambda")]
async fn download_zip_template(url: &str, template_root: &Path) -> Result<PathBuf> {
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

#[tracing::instrument(target = "cargo_lambda")]
fn unzip_template(file: &Path, path: &Path) -> Result<PathBuf> {
    tracing::debug!("extracting template from ZIP file");

    let reader = File::open(file)
        .into_diagnostic()
        .wrap_err_with(|| format!("unable to open ZIP file: {file:?}"))?;

    let mut archive = ZipArchive::new(reader).into_diagnostic()?;
    archive.extract(path).into_diagnostic()?;

    Ok(path.into())
}

/// When downloading a ZIP archive from GitHub or GitLab, the contents of the repository
/// are placed inside a directory named after the repository and reference (e.g. "repo-name-main").
/// This function detects that directory structure and returns the path to the inner directory
/// if it exists, otherwise returns the original path.
#[tracing::instrument(target = "cargo_lambda")]
fn adjust_remote_zip_base(url: &str, path: &Path) -> Option<PathBuf> {
    let archive_regex = regex::Regex::new(
        r"https://(?P<host>[a-zA-Z0-9.-]+)/[a-zA-Z0-9][a-zA-Z0-9_-]+/(?P<repo>[a-zA-Z0-9][a-zA-Z0-9_-]+)(/-)?/archive/(refs/heads|[a-zA-Z0-9]+)/(?P<ref>[^/]+)\.zip$"
    ).into_diagnostic()
    .expect("invalid zip url regex");

    if let Some(caps) = archive_regex.captures(url) {
        let repo = caps.name("repo")?.as_str();
        let reference = caps.name("ref")?.as_str();
        let reference = reference.replace(&format!("{}-", repo), "");
        let base_dir = format!("{}-{}", repo, reference);

        let base_path = path.join(&base_dir);
        tracing::trace!(
            ?base_path,
            exists = base_path.exists(),
            "looking for base directory"
        );

        if base_path.exists() && base_path.is_dir() {
            return Some(base_path);
        }
    }

    None
}

fn find_local_directory(value: &str) -> Result<PathBuf> {
    let path = dunce::realpath(value)
        .map_err(|err| miette::miette!("invalid template option {value}: {err}"))?;

    if path.is_dir() {
        Ok(path)
    } else {
        Err(miette::miette!(
            "invalid template option {value}: No such directory"
        ))
    }
}

fn is_remote_zip_file(path: &str) -> bool {
    path.starts_with("https://") && path.ends_with(".zip")
}

fn find_local_zip_file(value: &str) -> Option<PathBuf> {
    // ignore error to fallback to other template options.
    if let Ok(path) = dunce::realpath(value) {
        if path.exists() && path.is_file() && path.extension().unwrap_or_default() == "zip" {
            return Some(path);
        }
    }

    None
}

fn match_git_http_url(path: &str) -> Option<GitRepo> {
    let repo_regex = regex::Regex::new(
        r"https://(?P<host>[a-zA-Z0-9.-]+)/(?P<repo>[a-zA-Z0-9][a-zA-Z0-9_-]+/[a-zA-Z0-9][a-zA-Z0-9_-]+)/?((branch|tag|tree)/(?P<ref>.+))?$",
    )
    .into_diagnostic()
    .expect("invalid HTTP regex");

    let caps = repo_regex.captures(path)?;

    let host = caps.name("host")?;
    let repo = caps.name("repo")?;
    let reference = caps
        .name("ref")
        .map(|m| m.as_str().trim_end_matches('/').replace('/', "-"));

    Some(GitRepo {
        host: host.as_str().into(),
        repo: repo.as_str().into(),
        reference,
        auth_user: None,
        protocol: GitProtocol::Http,
    })
}

fn match_git_ssh_url(value: &str) -> Option<GitRepo> {
    let ssh_regex = regex::Regex::new(
        r"ssh://(?P<host>[a-zA-Z0-9.-]+)/(?P<repo>[a-zA-Z0-9][a-zA-Z0-9_-]+/[a-zA-Z0-9][a-zA-Z0-9_-]+)(\.git)?$",
    )
    .into_diagnostic()
    .expect("invalid SSH regex");

    let git_regex = regex::Regex::new(
        r"git@(?P<host>[a-zA-Z0-9.-]+):(?P<repo>[a-zA-Z0-9][a-zA-Z0-9_-]+/[a-zA-Z0-9][a-zA-Z0-9_-]+)(\.git)?$",
    )
    .into_diagnostic()
    .expect("invalid Git SSH regex");

    let (auth_user, caps) = match ssh_regex.captures(value) {
        None => match git_regex.captures(value) {
            None => return None,
            Some(caps) => (Some("git".into()), caps),
        },
        Some(caps) => (None, caps),
    };

    let host = caps.name("host")?;
    let repo = caps.name("repo")?;

    Some(GitRepo {
        host: host.as_str().into(),
        repo: repo.as_str().into(),
        protocol: GitProtocol::Ssh,
        auth_user,
        ..Default::default()
    })
}

#[tracing::instrument(target = "cargo_lambda")]
fn clone_git_repo(repo: &GitRepo, path: &Path) -> Result<()> {
    let git_url = repo.to_url();
    let mut url = gix::url::parse(git_url.as_str().into()).into_diagnostic()?;
    url.set_user(repo.auth_user.clone());

    let mut prepare_clone = gix::prepare_clone(url, path).into_diagnostic()?;
    if let Some(ref_name) = &repo.reference {
        let name = PartialName::try_from(ref_name.as_str()).into_diagnostic()?;
        prepare_clone = prepare_clone.with_ref_name(Some(&name)).into_diagnostic()?;
    }

    let (mut prepare_checkout, _) = prepare_clone
        .fetch_then_checkout(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .into_diagnostic()?;

    prepare_checkout
        .main_worktree(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .into_diagnostic()?;

    Ok(())
}

fn cleanup_tmp_dir(path: &Path) {
    let _ = remove_dir_all(path.join(".git"));
    let _ = remove_file(path.join("cargo-lambda-template.zip"));
}

#[cfg(test)]
mod test {
    use super::*;
    use assertables::*;

    #[test]
    fn test_is_remote_zip_file() {
        assert!(is_remote_zip_file(
            "https://github.com/cargo-lambda/cargo-lambda/archive/refs/heads/main.zip"
        ));
        assert!(!is_remote_zip_file(
            "https://github.com/cargo-lambda/cargo-lambda"
        ));
        assert!(!is_remote_zip_file(
            "https://github.com/cargo-lambda/cargo-lambda/archive/refs/heads/main"
        ));
        assert!(!is_remote_zip_file("c:\\path\\to\\file.zip"));
    }

    #[test]
    fn test_find_local_zip_file() {
        let tmp_dir = tempdir().unwrap();
        let zip_file = tmp_dir.path().join("file.zip");
        std::fs::write(&zip_file, "").unwrap();
        assert_eq!(
            find_local_zip_file(zip_file.to_str().unwrap()),
            Some(dunce::realpath(zip_file).unwrap())
        );

        assert_eq!(find_local_zip_file("missing.zip"), None);
    }

    #[test]
    fn test_match_git_http_url() {
        assert_eq!(None, match_git_http_url("https://github.com"));
        assert_eq!(None, match_git_http_url("https://github.com/"));
        assert_eq!(None, match_git_http_url("https://github.com/cargo-lambda"));
        assert_eq!(None, match_git_http_url("https://github.com/cargo-lambda/"));

        let repo = match_git_http_url("https://github.com/cargo-lambda/cargo-lambda").unwrap();
        assert_eq!("github.com", repo.host);
        assert_eq!("cargo-lambda/cargo-lambda", repo.repo);
        assert_eq!(None, repo.reference);
        assert_eq!(None, repo.auth_user);
        assert_eq!(GitProtocol::Http, repo.protocol);

        let repo =
            match_git_http_url("https://github.com/cargo-lambda/cargo-lambda/tag/v0.1.0").unwrap();
        assert_eq!("github.com", repo.host);
        assert_eq!("cargo-lambda/cargo-lambda", repo.repo);
        assert_eq!(Some("v0.1.0".into()), repo.reference);
        assert_eq!(None, repo.auth_user);
        assert_eq!(GitProtocol::Http, repo.protocol);

        let repo = match_git_http_url(
            "https://github.com/cargo-lambda/cargo-lambda/branch/branch-with-slashes",
        )
        .unwrap();
        assert_eq!("github.com", repo.host);
        assert_eq!("cargo-lambda/cargo-lambda", repo.repo);
        assert_eq!(Some("branch-with-slashes".into()), repo.reference);
        assert_eq!(GitProtocol::Http, repo.protocol);
        assert_eq!(None, repo.auth_user);

        let repo = match_git_http_url(
            "https://gitlab.com/cargo-lambda/cargo-lambda/branch/branch-with-slashes",
        )
        .unwrap();
        assert_eq!("gitlab.com", repo.host);
        assert_eq!("cargo-lambda/cargo-lambda", repo.repo);
        assert_eq!(Some("branch-with-slashes".into()), repo.reference);
        assert_eq!(GitProtocol::Http, repo.protocol);
        assert_eq!(None, repo.auth_user);

        let repo =
            match_git_http_url("https://github.com/cargo-lambda/cargo-lambda/tree/main").unwrap();
        assert_eq!(Some("main".into()), repo.reference);
    }

    #[test]
    fn test_match_git_ssh_url() {
        assert_eq!(None, match_git_ssh_url("ssh://github.com"));
        assert_eq!(None, match_git_ssh_url("ssh://github.com/cargo-lambda"));

        assert_eq!(None, match_git_ssh_url("git@github.com"));
        assert_eq!(None, match_git_ssh_url("git@github.com:"));
        assert_eq!(None, match_git_ssh_url("git@github.com:/"));
        assert_eq!(None, match_git_ssh_url("git@github.com:cargo-lambda"));
        assert_eq!(None, match_git_ssh_url("git@github.com:cargo-lambda/"));

        let repo = match_git_ssh_url("ssh://github.com/cargo-lambda/cargo-lambda").unwrap();
        assert_eq!("github.com", repo.host);
        assert_eq!("cargo-lambda/cargo-lambda", repo.repo);
        assert_eq!(None, repo.reference);
        assert_eq!(GitProtocol::Ssh, repo.protocol);
        assert_eq!(None, repo.auth_user);
        let repo = match_git_ssh_url("git@github.com:cargo-lambda/cargo-lambda").unwrap();
        assert_eq!("github.com", repo.host);
        assert_eq!("cargo-lambda/cargo-lambda", repo.repo);
        assert_eq!(None, repo.reference);
        assert_eq!(GitProtocol::Ssh, repo.protocol);
        assert_eq!(Some("git".into()), repo.auth_user);
        let repo = match_git_ssh_url("ssh://github.com/cargo-lambda/cargo-lambda.git").unwrap();
        assert_eq!("github.com", repo.host);
        assert_eq!("cargo-lambda/cargo-lambda", repo.repo);
        assert_eq!(None, repo.reference);
        assert_eq!(GitProtocol::Ssh, repo.protocol);
        assert_eq!(None, repo.auth_user);

        let repo = match_git_ssh_url("git@github.com:cargo-lambda/cargo-lambda.git").unwrap();
        assert_eq!("github.com", repo.host);
        assert_eq!("cargo-lambda/cargo-lambda", repo.repo);
        assert_eq!(None, repo.reference);
        assert_eq!(GitProtocol::Ssh, repo.protocol);
        assert_eq!(Some("git".into()), repo.auth_user);
    }

    #[test]
    fn test_template_source() {
        let source = TemplateSource::try_from("https://github.com/cargo-lambda/cargo-lambda")
            .expect("failed to parse root GitHub URL");
        let expected = TemplateSource::RemoteRepo(GitRepo {
            host: "github.com".into(),
            repo: "cargo-lambda/cargo-lambda".into(),
            protocol: GitProtocol::Http,
            ..Default::default()
        });
        assert_eq!(expected, source);

        let source = TemplateSource::try_from(
            "https://github.com/cargo-lambda/cargo-lambda/archive/refs/heads/main.zip",
        )
        .expect("failed to parse zip file GitHub URL");
        assert_eq!(
            TemplateSource::RemoteZip(
                "https://github.com/cargo-lambda/cargo-lambda/archive/refs/heads/main.zip".into()
            ),
            source
        );

        let source = TemplateSource::try_from("../../tests/templates/function-template.zip")
            .expect("failed to parse relative path to zip file");
        let destination = dunce::realpath("../../tests/templates/function-template.zip")
            .expect("failed to parse real path");
        assert_eq!(TemplateSource::LocalZip(destination), source);

        let source = TemplateSource::try_from("../../tests/templates/function-template")
            .expect("failed to parse relative directory path");
        let destination = dunce::realpath("../../tests/templates/function-template")
            .expect("failed to parse real path");
        assert_eq!(TemplateSource::LocalDir(destination), source);

        let source = TemplateSource::try_from("../../tests/templates/MISSING-template")
            .expect_err("failed to return an error looking for a missing directory");

        #[cfg(not(windows))]
        assert_contains!(source.to_string(), "invalid template option ../../tests/templates/MISSING-template: No such file or directory");
        #[cfg(windows)]
        assert_contains!(source.to_string(), "invalid template option ../../tests/templates/MISSING-template: The system cannot find the file specified.");

        let source = TemplateSource::try_from("../../tests/templates/function-template/Cargo.toml")
            .expect_err("failed to return an error looking for a missing directory");
        assert_contains!(source.to_string(), "invalid template option ../../tests/templates/function-template/Cargo.toml: No such directory");
    }

    #[test]
    fn test_adjust_remote_zip_base() {
        let tmp_dir = tempdir().unwrap();
        let expected = tmp_dir.path().join("default-template-main");
        std::fs::create_dir(&expected).unwrap();

        let url = "https://github.com/cargo-lambda/default-template/archive/refs/heads/main.zip";
        let base = adjust_remote_zip_base(url, &tmp_dir.path());

        assert_eq!(Some(expected.clone()), base);

        let url =
            "https://gitlab.com/calavera/default-template/-/archive/main/default-template-main.zip";
        let base = adjust_remote_zip_base(url, &tmp_dir.path());
        assert_eq!(Some(expected), base);
    }

    #[test]
    fn test_template_root_config_path() {
        let root = TemplateRoot::LocalDir(PathBuf::from("../../tests/templates/config-template"));
        assert_eq!(root.config_path(), root.path());
        assert_eq!(root.path().join("template"), root.final_path());
    }

    #[test]
    fn test_template_root_config_path_in_template_dir() {
        let tmpdir = tempdir().unwrap();
        std::fs::create_dir_all(tmpdir.path().join("template")).unwrap();
        std::fs::write(tmpdir.path().join("template").join("CargoLambda.toml"), "").unwrap();

        let root = TemplateRoot::TempDir((tmpdir, None));
        assert_eq!(root.config_path(), root.path().join("template"));

        assert_eq!(root.path().join("template"), root.final_path());
    }

    #[test]
    fn test_template_root_with_base_dir() {
        let tmpdir = tempdir().unwrap();
        let base = tmpdir.path().join("base");
        std::fs::create_dir_all(&base).unwrap();

        let root = TemplateRoot::TempDir((tmpdir, Some(base.clone())));
        assert_eq!(root.final_path(), base);
    }

    #[test]
    fn test_template_root_with_base_dir_and_template_dir() {
        let tmpdir = tempdir().unwrap();
        let base = tmpdir.path().join("base");

        std::fs::create_dir_all(&base).unwrap();
        std::fs::create_dir_all(base.join("template")).unwrap();

        let root = TemplateRoot::TempDir((tmpdir, Some(base.clone())));
        assert_eq!(root.final_path(), base.join("template"));
    }
}
