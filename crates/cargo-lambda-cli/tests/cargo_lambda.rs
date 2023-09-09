use cargo_test_macro::cargo_test;
use std::{
    fs::{create_dir_all, read_to_string, File},
    io::Write,
};
use zip::ZipArchive;

mod harness;
use harness::{
    cargo_lambda_build, cargo_lambda_init, cargo_lambda_new, test_project, LambdaProjectExt,
};

#[cargo_test]
fn test_build_basic_function() {
    let (root, cmd) = cargo_lambda_new("test-basic-function");

    cmd.arg("--no-interactive")
        .arg("test-basic-function")
        .assert()
        .success();

    let project = test_project(root);
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin("test-basic-function");
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_basic_zip_function() {
    let (root, cmd) = cargo_lambda_new("test-basic-function");

    cmd.arg("--no-interactive")
        .arg("test-basic-function")
        .assert()
        .success();

    let project = test_project(root);
    cargo_lambda_build(project.root())
        .args(["--output-format", "zip"])
        .assert()
        .success();

    let bin = project
        .lambda_dir()
        .join("test-basic-function")
        .join("bootstrap.zip");
    assert!(bin.exists(), "{:?} doesn't exist", bin);
    let file = File::open(bin).expect("failed to open zip file");
    let mut zip = ZipArchive::new(file).expect("failed to initialize the zip archive");
    assert!(
        zip.by_name("bootstrap").is_ok(),
        "bootstrap is not in the zip archive. Files in zip: {:?}",
        zip.file_names()
            .collect::<Vec<&str>>()
            .join(", ")
    );
}

#[cargo_test]
fn test_build_http_function() {
    let (root, cmd) = cargo_lambda_new("test-http-function");

    cmd.arg("--http")
        .arg("test-http-function")
        .assert()
        .success();

    let project = test_project(root);
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin("test-http-function");
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_http_feature_function() {
    let (root, cmd) = cargo_lambda_new("test-http-function");

    cmd.arg("--http-feature")
        .arg("apigw_rest")
        .arg("test-http-function")
        .assert()
        .success();

    let project = test_project(root);
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin("test-http-function");
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_event_type_function() {
    let (root, cmd) = cargo_lambda_new("test-event-type-function");

    cmd.arg("--event-type")
        .arg("s3::S3Event")
        .arg("test-event-type-function")
        .assert()
        .success();

    let project = test_project(root);
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin("test-event-type-function");
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_basic_extension() {
    let (root, cmd) = cargo_lambda_new("test-basic-extension");

    cmd.arg("--extension")
        .arg("test-basic-extension")
        .assert()
        .success();

    let project = test_project(root);
    cargo_lambda_build(project.root())
        .arg("--extension")
        .assert()
        .success();

    let bin = project.lambda_extension_bin("test-basic-extension");
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_logs_extension() {
    let (root, cmd) = cargo_lambda_new("test-logs-extension");

    cmd.arg("--extension")
        .arg("--logs")
        .arg("test-logs-extension")
        .assert()
        .success();

    let project = test_project(root);
    cargo_lambda_build(project.root())
        .arg("--extension")
        .assert()
        .success();

    let bin = project.lambda_extension_bin("test-logs-extension");
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_telemetry_extension() {
    let (root, cmd) = cargo_lambda_new("test-telemetry-extension");

    cmd.arg("--extension")
        .arg("--telemetry")
        .arg("test-telemetry-extension")
        .assert()
        .success();

    let project = test_project(root);
    cargo_lambda_build(project.root())
        .arg("--extension")
        .assert()
        .success();

    let bin = project.lambda_extension_bin("test-telemetry-extension");
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_init_subcommand() {
    let (root, cmd) = cargo_lambda_init("test-basic-function");

    cmd.arg("--no-interactive").assert().success();
    assert!(root.join("Cargo.toml").exists(), "missing Cargo.toml");
    assert!(
        root.join("src").join("main.rs").exists(),
        "missing src/main.rs"
    );

    let project = test_project(root);
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin("test-basic-function");
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_init_subcommand_without_override() {
    let (root, cmd) = cargo_lambda_init("test-basic-function");
    let src = root.join("src");
    let main = src.join("main.rs");
    create_dir_all(src).expect("failed to create src directory");

    let mut main_file = File::create(&main).expect("failed to create main.rs file");
    let content = r#"fn main() {
        println!("Hello, world!");
    }"#;
    main_file
        .write_all(content.as_bytes())
        .expect("failed to create main content");
    main_file.flush().unwrap();

    cmd.arg("--no-interactive").assert().success();
    assert!(root.join("Cargo.toml").exists(), "missing Cargo.toml");
    assert!(
        root.join("src").join("main.rs").exists(),
        "missing src/main.rs"
    );

    let out = read_to_string(main).expect("failed to read main.rs file");
    assert_eq!(content, out);
}

#[cargo_test]
fn test_build_basic_zip_extension() {
    let (root, cmd) = cargo_lambda_new("test-basic-extension");

    cmd.arg("--extension")
        .arg("test-basic-extension")
        .assert()
        .success();

    let project = test_project(root);
    cargo_lambda_build(project.root())
        .arg("--extension")
        .args(["--output-format", "zip"])
        .assert()
        .success();

    let bin = project.lambda_extension_bin("test-basic-extension.zip");
    assert!(bin.exists(), "{:?} doesn't exist", &bin);
    let file = File::open(bin).expect("failed to open zip file");
    let mut zip = ZipArchive::new(file).expect("failed to initialize the zip archive");
    assert!(
        zip.by_name("extensions/test-basic-extension").is_ok(),
        "test-basic-extension is not in the zip archive. Files in zip: {:?}",
        zip.file_names()
            .collect::<Vec<&str>>()
            .join(", ")
    );
}

#[cargo_test]
fn test_build_internal_zip_extension() {
    let (root, cmd) = cargo_lambda_new("test-internal-extension");

    cmd.arg("--extension")
        .arg("test-internal-extension")
        .assert()
        .success();

    let project = test_project(root);
    cargo_lambda_build(project.root())
        .arg("--extension")
        .arg("--internal")
        .args(["--output-format", "zip"])
        .assert()
        .success();

    let bin = project.lambda_extension_bin("test-internal-extension.zip");
    assert!(bin.exists(), "{:?} doesn't exist", &bin);
    let file = File::open(bin).expect("failed to open zip file");
    let mut zip = ZipArchive::new(file).expect("failed to initialize the zip archive");
    assert!(
        zip.by_name("test-internal-extension").is_ok(),
        "test-internal-extension is not in the zip archive. Files in zip: {:?}",
        zip.file_names()
            .collect::<Vec<&str>>()
            .join(", ")
    );
}
