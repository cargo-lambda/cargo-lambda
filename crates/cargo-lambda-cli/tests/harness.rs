use std::{
    fs,
    path::{Path, PathBuf},
};

use cargo_test_support::Project;
use snapbox::cmd::Command;

pub fn test_project<P: AsRef<Path>>(path: P) -> Project {
    let project = Project::from_template(path);
    let metadata = project.read_file("Cargo.toml");
    let metadata = format!("{metadata}\n\n[workspace]\n");
    project.change_file("Cargo.toml", &metadata);

    project
}

pub fn cargo_lambda_new(project_name: &str) -> (PathBuf, Command) {
    let project = project();

    let cwd = dunce::canonicalize(project.root()).expect("failed to create canonical path");

    let cmd = Command::cargo_lambda()
        .arg("lambda")
        .arg("new")
        .current_dir(cwd);

    let project_path = project.root().join(project_name);

    (project_path, cmd)
}

pub fn cargo_lambda_init(project_name: &str) -> (PathBuf, Command) {
    let project = project();

    let cwd = dunce::canonicalize(project.root()).expect("failed to create canonical path");
    fs::create_dir_all(&cwd).expect("failed to create project directory");

    let cmd = Command::cargo_lambda()
        .arg("lambda")
        .arg("init")
        .arg("--name")
        .arg(project_name)
        .current_dir(cwd);

    (project.root(), cmd)
}

pub fn cargo_lambda_build<P: AsRef<Path>>(path: P) -> Command {
    Command::cargo_lambda()
        .arg("lambda")
        .arg("build")
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
