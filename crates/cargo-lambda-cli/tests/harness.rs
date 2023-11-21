use std::{
    fs,
    path::{Path, PathBuf},
};

use cargo_test_support::Project;
use snapbox::cmd::Command;

pub struct LambdaProject {
    pub name: String,
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
            .current_dir(&self.cwd)
    }

    pub fn init_cmd(&self) -> Command {
        Command::cargo_lambda()
            .arg("lambda")
            .arg("init")
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
}

pub fn test_project<P: AsRef<Path>>(path: P) -> Project {
    let project = Project::from_template(path);
    let metadata = project.read_file("Cargo.toml");
    let metadata = format!("{metadata}\n\n[workspace]\n");
    project.change_file("Cargo.toml", &metadata);

    project
}

pub fn cargo_lambda_new(project_name: &str) -> LambdaProject {
    let project = project();

    let cwd = dunce::canonicalize(project.root()).expect("failed to create canonical path");
    let name = format!("{}-{}", project_name, uuid::Uuid::new_v4());
    let root = project.root().join(&name);

    LambdaProject { name, root, cwd }
}

pub fn cargo_lambda_init(project_name: &str) -> LambdaProject {
    let project = project();

    let cwd = dunce::canonicalize(project.root()).expect("failed to create canonical path");
    fs::create_dir_all(&cwd).expect("failed to create project directory");

    let name = format!("{}-{}", project_name, uuid::Uuid::new_v4());

    LambdaProject {
        name,
        root: project.root(),
        cwd,
    }
}

pub fn cargo_lambda_build<P: AsRef<Path>>(path: P) -> Command {
    let path = path.as_ref(); // /home/david/src/cargo-lambda/target/tmp/cit/t8/case
    let cache = path
        .parent()
        .unwrap() // /home/david/src/cargo-lambda/target/tmp/cit/t8
        .parent()
        .unwrap() // /home/david/src/cargo-lambda/target/tmp/cit
        .parent()
        .unwrap() // /home/david/src/cargo-lambda/target/tmp
        .parent()
        .unwrap() // /home/david/src/cargo-lambda/target
        .as_os_str();

    Command::cargo_lambda()
        .arg("lambda")
        .arg("build")
        .arg("-vv")
        .arg("--lambda-dir")
        .arg(path.join("target").join("lambda"))
        .env("RUST_BACKTRACE", "full")
        .env("CARGO_ZIGBUILD_CACHE_DIR", cache)
        .env("CARGO_TARGET_DIR", cache)
        .current_dir(path)
}

pub fn project() -> Project {
    cargo_test_support::project().no_manifest().build()
}

fn cargo_exe() -> std::path::PathBuf {
    snapbox::cmd::cargo_bin("cargo-lambda")
}

pub trait LambdaCommandExt {
    fn cargo_lambda() -> Self;
}

impl LambdaCommandExt for Command {
    fn cargo_lambda() -> Self {
        Self::new(cargo_exe()).with_assert(cargo_test_support::compare::assert_ui())
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
