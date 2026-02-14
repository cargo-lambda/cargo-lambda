use std::{
    path::{Path, PathBuf},
    sync::Once,
};

use snapbox::cmd::{Command, OutputAssert};

mod project;
use project::Project;
pub use project::{CargoPathExt, paths::init_root, project};

static WARMUP: Once = Once::new();

/// Pre-compile common dependencies to warm the shared build cache.
/// This is called once before running tests to reduce compilation time.
pub fn warmup_build_cache() {
    WARMUP.call_once(|| {
        let shared_target = std::env::temp_dir().join("cargo-lambda-test-shared-target");

        // Create a minimal warmup project
        let warmup_dir = std::env::temp_dir().join("cargo-lambda-warmup-project");

        // Clean up any existing warmup directory
        if warmup_dir.exists() {
            let _ = std::fs::remove_dir_all(&warmup_dir);
        }

        // Create a basic Rust project structure
        std::fs::create_dir_all(&warmup_dir).ok();

        let cargo_toml = r#"[package]
name = "warmup"
version = "0.1.0"
edition = "2021"

[dependencies]
lambda_runtime = "0.13"
tokio = { version = "1", features = ["macros"] }
serde = { version = "1", features = ["derive"] }
"#;

        let main_rs = r#"use lambda_runtime::{service_fn, LambdaEvent, Error};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Request {}

#[derive(Serialize)]
struct Response {}

async fn handler(_event: LambdaEvent<Request>) -> Result<Response, Error> {
    Ok(Response {})
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    lambda_runtime::run(service_fn(handler)).await
}
"#;

        std::fs::write(warmup_dir.join("Cargo.toml"), cargo_toml).ok();
        let src_dir = warmup_dir.join("src");
        std::fs::create_dir_all(&src_dir).ok();
        std::fs::write(src_dir.join("main.rs"), main_rs).ok();

        // Pre-compile the project to warm the cache
        let _ = std::process::Command::new("cargo")
            .arg("check")
            .arg("--target=x86_64-unknown-linux-gnu")
            .current_dir(&warmup_dir)
            .env("CARGO_BUILD_TARGET_DIR", &shared_target)
            .output();
    });
}

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
    cargo_lambda_new_in_root(project_name, template, project.root())
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
    let lambda_dir = path.join("target").join("lambda");

    Command::cargo_lambda()
        .arg("lambda")
        .arg("build")
        .arg("-vv")
        .arg("--lambda-dir")
        .arg(lambda_dir.as_os_str())
        .env("RUST_BACKTRACE", "full")
        .env("CARGO_ZIGBUILD_CACHE_DIR", path.as_os_str())
        .current_dir(path)
}

pub fn cargo_lambda_dry_deploy<P: AsRef<Path>>(path: P) -> Command {
    let path = path.as_ref();
    let lambda_dir = path.join("target").join("lambda");

    Command::cargo_lambda()
        .arg("lambda")
        .arg("deploy")
        .arg("--dry")
        .arg("--output-format")
        .arg("json")
        .arg("--lambda-dir")
        .arg(lambda_dir.as_os_str())
        .current_dir(path)
}

fn cargo_exe() -> std::path::PathBuf {
    snapbox::cmd::cargo_bin!("cargo-lambda").to_path_buf()
}

pub trait LambdaCommandExt {
    fn cargo_lambda() -> Self;
}

impl LambdaCommandExt for Command {
    fn cargo_lambda() -> Self {
        Self::new(cargo_exe()).with_assert(project::assert_ui())
    }
}

pub fn deploy_output_json(output: &OutputAssert) -> Result<serde_json::Value, serde_json::Error> {
    let output = output.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let (_log, json) = stdout.split_once("loading binary data").unwrap();
    serde_json::from_str(json)
}
