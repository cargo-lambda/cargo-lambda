use cargo_test_macro::cargo_test;
use std::{
    fs::{create_dir_all, read_to_string, File},
    io::Write,
};
use zip::ZipArchive;

mod harness;
use harness::{cargo_lambda_build, cargo_lambda_init, cargo_lambda_new, LambdaProjectExt};

#[cargo_test]
fn test_build_basic_function() {
    let lp = cargo_lambda_new("test-basic-function");

    lp.new_cmd()
        .arg("--no-interactive")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin(&lp.name);
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_basic_zip_function() {
    let lp = cargo_lambda_new("test-basic-function");

    lp.new_cmd()
        .arg("--no-interactive")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root())
        .args(["--output-format", "zip"])
        .assert()
        .success();

    let bin = project.lambda_dir().join(&lp.name).join("bootstrap.zip");
    assert!(bin.exists(), "{:?} doesn't exist", bin);
    let file = File::open(bin).expect("failed to open zip file");
    let mut zip = ZipArchive::new(file).expect("failed to initialize the zip archive");
    assert!(
        zip.by_name("bootstrap").is_ok(),
        "bootstrap is not in the zip archive. Files in zip: {:?}",
        zip.file_names().collect::<Vec<&str>>().join(", ")
    );
}

#[cargo_test]
fn test_build_http_function() {
    let lp = cargo_lambda_new("test-http-function");

    lp.new_cmd().arg("--http").arg(&lp.name).assert().success();

    let project = lp.test_project();
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin(&lp.name);
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_http_feature_function() {
    let lp = cargo_lambda_new("test-http-function");

    lp.new_cmd()
        .arg("--http-feature")
        .arg("apigw_rest")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin(&lp.name);
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_event_type_function() {
    let lp = cargo_lambda_new("test-event-type-function");

    lp.new_cmd()
        .arg("--event-type")
        .arg("s3::S3Event")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin(&lp.name);
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_basic_extension() {
    let lp = cargo_lambda_new("test-basic-extension");

    lp.new_cmd()
        .arg("--extension")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root())
        .arg("--extension")
        .assert()
        .success();

    let bin = project.lambda_extension_bin(&lp.name);
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_logs_extension() {
    let lp = cargo_lambda_new("test-logs-extension");

    lp.new_cmd()
        .arg("--extension")
        .arg("--logs")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root())
        .arg("--extension")
        .assert()
        .success();

    let bin = project.lambda_extension_bin(&lp.name);
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_build_telemetry_extension() {
    let lp = cargo_lambda_new("test-telemetry-extension");

    lp.new_cmd()
        .arg("--extension")
        .arg("--telemetry")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root())
        .arg("--extension")
        .assert()
        .success();

    let bin = project.lambda_extension_bin(&lp.name);
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_init_subcommand() {
    let lp = cargo_lambda_init("test-basic-function");
    let root = lp.root();

    lp.init_cmd().arg("--no-interactive").assert().success();
    assert!(root.join("Cargo.toml").exists(), "missing Cargo.toml");
    assert!(
        root.join("src").join("main.rs").exists(),
        "missing src/main.rs"
    );

    let project = lp.test_project();
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin(&lp.name);
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}

#[cargo_test]
fn test_init_subcommand_without_override() {
    let lp = cargo_lambda_init("test-basic-function");
    let root = lp.root();

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

    lp.init_cmd().arg("--no-interactive").assert().success();

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
    let lp = cargo_lambda_new("test-basic-extension");

    lp.new_cmd()
        .arg("--extension")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root())
        .arg("--extension")
        .args(["--output-format", "zip"])
        .assert()
        .success();

    let bin = project.lambda_extension_bin(&lp.zip_name());
    assert!(bin.exists(), "{:?} doesn't exist", &bin);
    let file = File::open(bin).expect("failed to open zip file");
    let mut zip = ZipArchive::new(file).expect("failed to initialize the zip archive");
    assert!(
        zip.by_name(&lp.extension_path()).is_ok(),
        "{} is not in the zip archive. Files in zip: {}",
        &lp.extension_path(),
        zip.file_names().collect::<Vec<&str>>().join(", ")
    );
}

#[cargo_test]
fn test_build_internal_zip_extension() {
    let lp = cargo_lambda_new("test-internal-extension");

    lp.new_cmd()
        .arg("--extension")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root())
        .arg("--extension")
        .arg("--internal")
        .args(["--output-format", "zip"])
        .assert()
        .success();

    let bin = project.lambda_extension_bin(&lp.zip_name());
    assert!(bin.exists(), "{:?} doesn't exist", &bin);
    let file = File::open(bin).expect("failed to open zip file");
    let mut zip = ZipArchive::new(file).expect("failed to initialize the zip archive");
    assert!(
        zip.by_name(&lp.name).is_ok(),
        "{} is not in the zip archive. Files in zip: {}",
        &lp.name,
        zip.file_names().collect::<Vec<&str>>().join(", ")
    );
}
