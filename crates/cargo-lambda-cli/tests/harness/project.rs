// Code extracted from Cargo: https://github.com/rust-lang/cargo/tree/master/crates/cargo-test-support
// Under MIT License: https://github.com/rust-lang/cargo/blob/master/LICENSE-MIT

use std::{
    env,
    fmt::Write,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

/// A cargo project to run tests against.
///
/// See [`ProjectBuilder`] or [`Project::from_template`] to get started.
pub struct Project {
    root: PathBuf,
}

/// Create a project to run tests against
///
/// The project can be constructed programmatically or from the filesystem with [`Project::from_template`]
#[must_use]
pub struct ProjectBuilder {
    root: Project,
}

impl ProjectBuilder {
    pub fn new(root: PathBuf) -> ProjectBuilder {
        ProjectBuilder {
            root: Project { root },
        }
    }

    /// Creates the project.
    pub fn build(self) -> Project {
        // First, clean the directory if it already exists
        self.rm_root();

        // Create the empty directory
        self.root.root().mkdir_p();

        let ProjectBuilder { root, .. } = self;
        root
    }

    fn rm_root(&self) {
        self.root.root().rm_rf()
    }
}

impl Project {
    /// Copy the test project from a fixed state
    pub fn from_template(template_path: impl AsRef<std::path::Path>) -> Self {
        let root = paths::root();
        let project_root = root.join("case");
        snapbox::path::copy_template(template_path.as_ref(), &project_root).unwrap();
        Self { root: project_root }
    }

    /// Root of the project, ex: `/path/to/cargo/target/cit/t0/foo`
    pub fn root(&self) -> PathBuf {
        self.root.clone()
    }

    /// Project's target dir, ex: `/path/to/cargo/target/cit/t0/foo/target`
    pub fn build_dir(&self) -> PathBuf {
        self.root().join("target")
    }

    // /// Changes the contents of an existing file.
    pub fn change_file(&self, path: &str, body: &str) {
        FileBuilder::new(self.root().join(path), body, false).mk()
    }

    /// Returns the contents of a path in the project root
    pub fn read_file(&self, path: &str) -> String {
        let full = self.root().join(path);
        fs::read_to_string(&full)
            .unwrap_or_else(|e| panic!("could not read file {}: {}", full.display(), e))
    }
}

// Generates a project layout
pub fn project() -> ProjectBuilder {
    ProjectBuilder::new(paths::root().join("foo"))
}

#[derive(PartialEq, Clone)]
struct FileBuilder {
    path: PathBuf,
    body: String,
    executable: bool,
}

impl FileBuilder {
    pub fn new(path: PathBuf, body: &str, executable: bool) -> FileBuilder {
        FileBuilder {
            path,
            body: body.to_string(),
            executable,
        }
    }

    fn mk(&mut self) {
        if self.executable {
            let mut path = self.path.clone().into_os_string();
            write!(path, "{}", env::consts::EXE_SUFFIX).unwrap();
            self.path = path.into();
        }

        self.dirname().mkdir_p();
        fs::write(&self.path, &self.body)
            .unwrap_or_else(|e| panic!("could not create file {}: {}", self.path.display(), e));

        #[cfg(unix)]
        if self.executable {
            use std::os::unix::fs::PermissionsExt;

            let mut perms = fs::metadata(&self.path).unwrap().permissions();
            let mode = perms.mode();
            perms.set_mode(mode | 0o111);
            fs::set_permissions(&self.path, perms).unwrap();
        }
    }

    fn dirname(&self) -> &Path {
        self.path.parent().unwrap()
    }
}

pub trait CargoPathExt {
    fn rm_rf(&self);
    fn mkdir_p(&self);

    /// Returns a list of all files and directories underneath the given
    /// directory, recursively, including the starting path.
    fn ls_r(&self) -> Vec<PathBuf>;
}

impl CargoPathExt for Path {
    fn rm_rf(&self) {
        let meta = match self.symlink_metadata() {
            Ok(meta) => meta,
            Err(e) => {
                if e.kind() == ErrorKind::NotFound {
                    return;
                }
                panic!("failed to remove {:?}, could not read: {:?}", self, e);
            }
        };
        // There is a race condition between fetching the metadata and
        // actually performing the removal, but we don't care all that much
        // for our tests.
        if meta.is_dir() {
            if let Err(e) = fs::remove_dir_all(self) {
                panic!("failed to remove {:?}: {:?}", self, e)
            }
        } else if let Err(e) = fs::remove_file(self) {
            panic!("failed to remove {:?}: {:?}", self, e)
        }
    }

    fn mkdir_p(&self) {
        fs::create_dir_all(self)
            .unwrap_or_else(|e| panic!("failed to mkdir_p {}: {}", self.display(), e))
    }

    fn ls_r(&self) -> Vec<PathBuf> {
        walkdir::WalkDir::new(self)
            .sort_by_file_name()
            .into_iter()
            .filter_map(|e| e.map(|e| e.path().to_owned()).ok())
            .collect()
    }
}

pub mod paths {
    use super::CargoPathExt;
    use std::{
        cell::RefCell,
        path::PathBuf,
        sync::{
            atomic::{AtomicUsize, Ordering},
            Mutex, OnceLock,
        },
    };

    static CARGO_INTEGRATION_TEST_DIR: &str = "cit";

    static GLOBAL_ROOT: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

    thread_local! {
        static TEST_ID: RefCell<Option<usize>> = RefCell::new(None);
    }

    pub struct TestIdGuard {
        _private: (),
    }

    impl Drop for TestIdGuard {
        fn drop(&mut self) {
            TEST_ID.with(|n| *n.borrow_mut() = None);
        }
    }

    pub fn init_root() -> TestIdGuard {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);

        let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
        TEST_ID.with(|n| *n.borrow_mut() = Some(id));

        let guard = TestIdGuard { _private: () };

        set_global_root();
        let r = root();
        r.rm_rf();
        r.mkdir_p();

        guard
    }

    pub fn root() -> PathBuf {
        let id = TEST_ID.with(|n| {
            n.borrow()
                .expect("Tests must call `init_root` to use the crate root.")
        });

        let mut root = global_root();
        root.push(&format!("t{}", id));
        root
    }

    fn set_global_root() {
        let mut lock = GLOBAL_ROOT.get_or_init(Default::default).lock().unwrap();
        if lock.is_none() {
            let mut root = global_root_legacy();

            root.push(CARGO_INTEGRATION_TEST_DIR);
            *lock = Some(root);
        }
    }

    pub fn global_root() -> PathBuf {
        let lock = GLOBAL_ROOT.get_or_init(Default::default).lock().unwrap();
        match lock.as_ref() {
            Some(p) => p.clone(),
            None => unreachable!("GLOBAL_ROOT not set yet"),
        }
    }

    fn global_root_legacy() -> PathBuf {
        let mut path = std::env::current_exe().expect("");
        path.pop(); // chop off exe name
        path.pop(); // chop off "deps"
        path.push("tmp");
        path.mkdir_p();
        path
    }
}

pub(crate) fn assert_ui() -> snapbox::Assert {
    let root = paths::root();
    // Use `from_file_path` instead of `from_dir_path` so the trailing slash is
    // put in the users output, rather than hidden in the variable
    let root_url = url::Url::from_file_path(&root).unwrap().to_string();
    let root = root.display().to_string();

    let mut subs = snapbox::Substitutions::new();
    subs.extend([
        (
            "[EXE]",
            std::borrow::Cow::Borrowed(std::env::consts::EXE_SUFFIX),
        ),
        ("[ROOT]", std::borrow::Cow::Owned(root)),
        ("[ROOTURL]", std::borrow::Cow::Owned(root_url)),
    ])
    .unwrap();
    snapbox::Assert::new()
        .action_env(snapbox::DEFAULT_ACTION_ENV)
        .substitutions(subs)
}
