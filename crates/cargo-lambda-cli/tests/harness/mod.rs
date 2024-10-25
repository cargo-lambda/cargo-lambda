use std::path::{Path, PathBuf};

use snapbox::cmd::Command;

mod project;
use project::Project;
pub use project::{paths::init_root, project, CargoPathExt};

pub struct LambdaProject {
    pub name: String,
    template: PathBuf,
    root: PathBuf,
    cwd: PathBuf,
}

impl LambdaProject {
    pub fn zip_name(&self) -> String {
        format!("{}.zip", &self.name)
    }

    pub fn extension_path(&self) -> String {
        format!("extensions/{}", &self.name)
    }

    pub fn new_cmd(&self) -> Command {
        Command::cargo_lambda()
            .arg("lambda")
            .arg("new")
            .arg("--template")
            .arg(self.template_path().as_os_str())
            .current_dir(&self.cwd)
    }

    pub fn init_cmd(&self) -> Command {
        Command::cargo_lambda()
            .arg("lambda")
            .arg("init")
            .arg("--template")
            .arg(self.template_path().as_os_str())
            .arg("--name")
            .arg(&self.name)
            .current_dir(&self.cwd)
    }

    pub fn root(&self) -> &PathBuf {
        &self.root
    }

    pub fn test_project(&self) -> Project {
        test_project(&self.root)
    }

    fn template_path(&self) -> PathBuf {
        let path = Path::new("..")
            .join("..")
            .join("tests")
            .join("templates")
            .join(&self.template);

        dunce::realpath(path).expect("failed to create real template path")
    }
}

pub fn test_project<P: AsRef<Path>>(path: P) -> Project {
    let project = Project::from_template(path);
    let metadata = project.read_file("Cargo.toml");
    let metadata = format!("{metadata}\n\n[workspace]\n");
    project.change_file("Cargo.toml", &metadata);

    project
}

pub fn cargo_lambda_new<P: AsRef<Path>>(project_name: &str, template: P) -> LambdaProject {
    let project = project::project().build();
    cargo_lambda_new_in_root(project_name, template, &project.root())
}

pub fn cargo_lambda_new_in_root<P: AsRef<Path>, R: AsRef<Path>>(
    project_name: &str,
    template: P,
    root: R,
) -> LambdaProject {
    let root = root.as_ref();

    let cwd = dunce::canonicalize(root).expect("failed to create canonical path");
    let name = format!("{}-{}", project_name, uuid::Uuid::new_v4());
    let root = root.join(&name);

    LambdaProject {
        name,
        template: template.as_ref().to_path_buf(),
        root: root.to_path_buf(),
        cwd,
    }
}

pub fn cargo_lambda_init<P: AsRef<Path>>(project_name: &str, template: P) -> LambdaProject {
    let project = project::project().build();

    let cwd = dunce::canonicalize(project.root()).expect("failed to create canonical path");
    cwd.mkdir_p();

    let name = format!("{}-{}", project_name, uuid::Uuid::new_v4());

    LambdaProject {
        name,
        template: template.as_ref().to_path_buf(),
        root: project.root(),
        cwd,
    }
}

pub fn cargo_lambda_build<P: AsRef<Path>>(path: P) -> Command {
    let path = path.as_ref();

    Command::cargo_lambda()
        .arg("lambda")
        .arg("build")
        .arg("-vv")
        .env("RUST_BACKTRACE", "full")
        .env("CARGO_ZIGBUILD_CACHE_DIR", path.as_os_str())
        .current_dir(path)
}

pub fn cargo_lambda_dry_deploy<P: AsRef<Path>>(path: P) -> Command {
    let path = path.as_ref();

    Command::cargo_lambda()
        .arg("lambda")
        .arg("deploy")
        .arg("--dry")
        .current_dir(path)
}

fn cargo_exe() -> std::path::PathBuf {
    snapbox::cmd::cargo_bin("cargo-lambda")
}

pub trait LambdaCommandExt {
    fn cargo_lambda() -> Self;
}

impl LambdaCommandExt for Command {
    fn cargo_lambda() -> Self {
        Self::new(cargo_exe()).with_assert(project::assert_ui())
    }
}

pub trait LambdaProjectExt {
    fn lambda_dir(&self) -> PathBuf;
    fn lambda_function_bin(&self, name: &str) -> PathBuf;
    fn lambda_extension_bin(&self, name: &str) -> PathBuf;
}

impl LambdaProjectExt for Project {
    fn lambda_dir(&self) -> PathBuf {
        self.build_dir().join("lambda")
    }

    fn lambda_function_bin(&self, name: &str) -> PathBuf {
        self.lambda_dir().join(name).join("bootstrap")
    }

    fn lambda_extension_bin(&self, name: &str) -> PathBuf {
        self.lambda_dir().join("extensions").join(name)
    }
}
