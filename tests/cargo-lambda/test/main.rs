use std::path::{Path, PathBuf};

use cargo_test_support::{paths::CargoPathExt, Project};
use snapbox::cmd::Command;

mod lambda_build;

pub fn test_project<P: AsRef<Path>>(path: P) -> PathBuf {
    let project = Project::from_template(path);
    let metadata = project.read_file("Cargo.toml");
    let metadata = format!("{metadata}\n\n[workspace]\n");
    project.change_file("Cargo.toml", &metadata);

    project.root()
}

pub fn cargo_lambda_new(project_name: &str) -> (PathBuf, Command) {
    let project = project();

    let cmd = Command::cargo_lambda()
        .arg("lambda")
        .arg("new")
        .env("RUST_BACKTRACE", "1")
        .current_dir(project.root());

    let project_path = project.root().join(project_name);
    project_path.rm_rf();

    (project_path, cmd)
}

pub fn cargo_lambda_build<P: AsRef<Path>>(path: P) -> Command {
    Command::cargo_lambda()
        .arg("lambda")
        .arg("build")
        .env("RUST_BACKTRACE", "1")
        .current_dir(path)
}

pub fn project() -> Project {
    cargo_test_support::project().no_manifest().build()
}

fn cargo_exe() -> std::path::PathBuf {
    snapbox::cmd::cargo_bin("cargo-lambda")
}

pub trait CargoCommand {
    fn cargo_lambda() -> Self;
}

impl CargoCommand for snapbox::cmd::Command {
    fn cargo_lambda() -> Self {
        Self::new(cargo_exe()).with_assert(cargo_test_support::compare::assert_ui())
    }
}
