use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::Arc,
};

use cargo_lambda_metadata::env::EnvVarExtractor;
use ignore::Match;
use ignore_files::{IgnoreFile, IgnoreFilter};
use tracing::{debug, trace, trace_span};
use watchexec::{
    error::RuntimeError,
    event::{Event, FileType, Priority},
    filter::Filterer,
};

use crate::error::ServerError;

/// Collect ignore files from various sources:
/// - Files in the system using the keywords `CARGO_LAMBDA` and `cargo-lambda`:
///   - $HOME/.cargo-lambda/ignore
///   - $XDG_CONFIG_HOME/cargo-lambda/ignore
///   - $HOME/.CARGO_LAMBDA/ignore
///   - $XDG_CONFIG_HOME/CARGO_LAMBDA/ignore
///
/// - Origin-based ignore files (like `.gitignore`)
/// - Project-specific `.cargolambdaignore` file
/// - Custom ignore file specified via `CARGO_LAMBDA_IGNORE_FILE` environment variable
///
/// # Arguments
///
/// * `base` - The base path to start searching for ignore files from
///
/// # Returns
///
/// A vector of [`IgnoreFile`]s discovered from all sources
pub(crate) async fn discover_files(base: &Path, env: impl EnvVarExtractor) -> Vec<IgnoreFile> {
    let mut ignore_files = HashSet::new();

    let (env_ignore, env_ignore_errs) = ignore_files::from_environment(Some("CARGO_LAMBDA")).await;
    if !env_ignore.is_empty() {
        trace!(ignore_files = ?env_ignore, errors = ?env_ignore_errs, "discovered ignore files from environment variable");
        ignore_files.extend(env_ignore);
    }

    let (env_ignore, env_ignore_errs) = ignore_files::from_environment(Some("cargo-lambda")).await;
    if !env_ignore.is_empty() {
        trace!(ignore_files = ?env_ignore, errors = ?env_ignore_errs, "discovered ignore files from environment variable");
        ignore_files.extend(env_ignore);
    }

    let (origin_ignore, origin_ignore_errs) = ignore_files::from_origin(base).await;
    if !origin_ignore.is_empty() {
        trace!(ignore_files = ?origin_ignore, errors = ?origin_ignore_errs, "discovered ignore files from origin");
        ignore_files.extend(origin_ignore);
    }

    let mut ignore_files_vec = Vec::new();
    let mut ignore_files_vec_errs = Vec::new();

    let ignore_repo_rules_file = base.join(".cargolambdaignore");
    if ignore_repo_rules_file.is_file() {
        ignore_files::discover_file(
            &mut ignore_files_vec,
            &mut ignore_files_vec_errs,
            None,
            None,
            ignore_repo_rules_file,
        )
        .await;
    }

    if let Ok(ignore_env_file) = env.var("CARGO_LAMBDA_IGNORE_FILE") {
        let path = PathBuf::from(ignore_env_file);
        if path.is_file() {
            ignore_files::discover_file(
                &mut ignore_files_vec,
                &mut ignore_files_vec_errs,
                None,
                None,
                path,
            )
            .await;
        }
    }

    if !ignore_files_vec.is_empty() {
        trace!(ignore_files = ?ignore_files_vec, errors = ?ignore_files_vec_errs, "discovered ignore files");
        ignore_files.extend(ignore_files_vec);
    }

    let mut origins = HashSet::new();
    let mut current = base;
    if base.is_dir() && base.join("Cargo.toml").is_file() {
        origins.insert(base.to_owned());
    }

    while let Some(parent) = current.parent() {
        current = parent;
        if current.is_dir() && current.join("Cargo.toml").is_file() {
            origins.insert(current.to_owned());
        } else {
            break;
        }
    }

    for parent in origins {
        let (parent_ignore, parent_ignore_errs) = ignore_files::from_origin(&parent).await;
        trace!(parent = ?parent, ignore_files = ?parent_ignore, errors = ?parent_ignore_errs, "discovered ignore files from parent origin");
        ignore_files.extend(parent_ignore);
    }

    ignore_files.into_iter().collect()
}

pub(crate) async fn create_filter(
    base: &Path,
    files: &[IgnoreFile],
    ignore_changes: bool,
) -> Result<Arc<IgnoreFilterer>, ServerError> {
    trace!(?files, "creating watcher ignore filterer");

    if ignore_changes {
        let mut filter = IgnoreFilter::empty(base);

        filter
            .add_globs(&["**/*"], Some(&base.to_path_buf()))
            .map_err(ServerError::InvalidIgnoreFiles)?;

        return Ok(Arc::new(IgnoreFilterer(vec![filter])));
    }

    let mut filters = Vec::new();
    let mut filter = IgnoreFilter::empty(base);
    filter
        .add_globs(&["target/*", "target*"], Some(&base.to_path_buf()))
        .map_err(ServerError::InvalidIgnoreFiles)?;
    filters.push(filter);

    for file in files {
        let base = file
            .applies_in
            .clone()
            .unwrap_or_else(|| base.to_path_buf());
        let filter = IgnoreFilter::new(&base, std::slice::from_ref(file))
            .await
            .map_err(ServerError::InvalidIgnoreFiles)?;
        filters.push(filter);
    }

    debug!(?filters, "using ignore filter");

    Ok(Arc::new(IgnoreFilterer(filters)))
}

/// A Watchexec [`Filterer`] implementation for a list of [`IgnoreFilter`].
/// This is a fork of the [`IgnoreFilterer`] implementation in the `watchexec-filterer-ignore` crate,
/// but it allows for multiple ignore filters to be applied to an event.
///
/// We need this custom implementation because the official implementation
/// has a problem where it doesn't correctly handle the case where a path
/// matches an ignore file but is not a child of the base path where the
/// ignore file is located. In those cases, the matching process stops at the
/// first ignore match that matches the path, but then the path is not ignored.
/// We want to go through all the ignore globs to make sure we don't miss any
/// ignore matches.
#[derive(Clone, Debug)]
pub struct IgnoreFilterer(pub Vec<IgnoreFilter>);

impl Filterer for IgnoreFilterer {
    /// Filter an event.
    ///
    /// This implementation never errors. It returns `Ok(false)` if the event is ignored according
    /// to the ignore files, and `Ok(true)` otherwise. It ignores event priority.
    fn check_event(&self, event: &Event, _priority: Priority) -> Result<bool, RuntimeError> {
        let _span = trace_span!("filterer_check").entered();

        for (path, file_type) in event.paths() {
            let _span = trace_span!("checking_against_compiled", ?path, ?file_type).entered();
            let is_dir = file_type.is_some_and(|t| matches!(t, FileType::Dir));

            for filter in &self.0 {
                let mut pass = true;

                match filter.match_path(path, is_dir) {
                    Match::None => {
                        trace!("no match (pass)");
                        pass &= true;
                    }
                    Match::Ignore(glob) => {
                        if glob.from().is_none_or(|f| path.strip_prefix(f).is_ok()) {
                            trace!(?glob, "positive match (fail)");
                            pass &= false;
                        } else {
                            trace!(?glob, "positive match, but not in scope (ignore)");
                        }
                    }
                    Match::Whitelist(glob) => {
                        trace!(?glob, "negative match (pass)");
                        pass = true;
                    }
                }

                if !pass {
                    // If any of the filters fail, the event is ignored.
                    //
                    // This means that the server will not restart when
                    // a file is modified in a directory that is ignored
                    // by any of the ignore files.
                    return Ok(false);
                }
            }
        }

        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs::File, io::Write, path::PathBuf};

    use cargo_lambda_metadata::env::{HashMapEnvExtractor, SystemEnvExtractor};
    use watchexec::event::Tag;

    use super::*;

    #[test]
    fn test_ignore_filterer_without_filters() {
        let filter = IgnoreFilterer(vec![]);
        let event = Event {
            tags: vec![Tag::Path {
                path: "src/main.rs".into(),
                file_type: Some(FileType::File),
            }],
            ..Default::default()
        };
        assert!(filter.check_event(&event, Priority::Normal).unwrap());
    }

    #[test]
    fn test_ignore_filterer_with_filters() {
        let mut filter = IgnoreFilter::empty(Path::new("src"));
        filter
            .add_globs(&["**/*"], Some(&PathBuf::from("src")))
            .unwrap();
        let filterer = IgnoreFilterer(vec![filter]);
        let event = Event {
            tags: vec![Tag::Path {
                path: "src/main.rs".into(),
                file_type: Some(FileType::File),
            }],
            ..Default::default()
        };
        assert!(!filterer.check_event(&event, Priority::Normal).unwrap());
    }

    #[test]
    fn test_ignore_filterer_with_multiple_filters() {
        let mut filter = IgnoreFilter::empty(Path::new("src"));
        filter
            .add_globs(&["**/*"], Some(&PathBuf::from("src")))
            .unwrap();
        let mut filter2 = IgnoreFilter::empty(Path::new("foo"));
        filter2
            .add_globs(&["**/*"], Some(&PathBuf::from("foo")))
            .unwrap();

        let filterer = IgnoreFilterer(vec![filter, filter2]);
        let event = Event {
            tags: vec![Tag::Path {
                path: "foo/main.rs".into(),
                file_type: Some(FileType::File),
            }],
            ..Default::default()
        };
        assert!(!filterer.check_event(&event, Priority::Normal).unwrap());
    }

    #[tokio::test]
    async fn test_create_filter_with_default_target_dir() {
        let filter = create_filter(Path::new("."), &[], false).await.unwrap();
        assert_eq!(filter.0.len(), 1);

        let event = Event {
            tags: vec![Tag::Path {
                path: "./target/debug/Cargo.lock".into(),
                file_type: Some(FileType::File),
            }],
            ..Default::default()
        };
        assert!(!filter.check_event(&event, Priority::Normal).unwrap());
    }

    #[tokio::test]
    async fn test_create_filter_with_ignore_files() {
        let mut tempfile = tempfile::NamedTempFile::new().unwrap();
        writeln!(tempfile, "*").unwrap();

        let ignore_file = IgnoreFile {
            path: tempfile.path().to_path_buf(),
            applies_in: Some(PathBuf::from("./foo")),
            applies_to: None,
        };

        let filter = create_filter(Path::new("."), &[ignore_file], false)
            .await
            .unwrap();
        assert_eq!(filter.0.len(), 2);

        let event = Event {
            tags: vec![Tag::Path {
                path: "./target/debug/Cargo.lock".into(),
                file_type: Some(FileType::File),
            }],
            ..Default::default()
        };
        assert!(!filter.check_event(&event, Priority::Normal).unwrap());

        let event = Event {
            tags: vec![Tag::Path {
                path: "./foo/main.rs".into(),
                file_type: Some(FileType::File),
            }],
            ..Default::default()
        };
        assert!(!filter.check_event(&event, Priority::Normal).unwrap());
    }

    #[tokio::test]
    async fn test_discover_project_specific_files() {
        let tempdir = tempfile::tempdir().unwrap();
        let ignore_file = tempdir.path().join(".cargolambdaignore");
        writeln!(File::create(&ignore_file).unwrap(), "*").unwrap();

        let files = discover_files(tempdir.path(), SystemEnvExtractor).await;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, ignore_file);
    }

    #[tokio::test]
    async fn test_discover_project_specific_files_with_env_var() {
        let tempdir = tempfile::tempdir().unwrap();
        let ignore_file = tempdir.path().join("clignore");
        writeln!(File::create(&ignore_file).unwrap(), "*").unwrap();

        let env = HashMapEnvExtractor::from(vec![(
            "CARGO_LAMBDA_IGNORE_FILE",
            ignore_file.to_str().unwrap(),
        )]);

        let files = discover_files(tempdir.path(), env).await;
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, ignore_file);
    }

    #[tokio::test]
    async fn test_discover_project_specific_files_with_env_var_not_found() {
        let tempdir = tempfile::tempdir().unwrap();
        let env = HashMapEnvExtractor::from(vec![]);
        let files = discover_files(tempdir.path(), env).await;
        assert_eq!(files.len(), 0);
    }
}
