use cargo_test_macro::cargo_test;
use std::{
    fs::{create_dir_all, File},
    io::{read_to_string, Write},
};

mod harness;
use harness::{
    cargo_lambda_build, cargo_lambda_init, cargo_lambda_new, test_project, LambdaProjectExt,
};

#[cargo_test]
fn test_lambda_build() {
    test_build_basic_function();
    test_build_http_function();
    test_build_basic_extension();
    test_build_logs_extension();
    test_build_telemetry_extension();
    test_init_subcommand();
    test_init_subcommand_without_override();
}

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

fn test_build_http_function() {
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

fn test_init_subcommand_without_override() {
    let (root, cmd) = cargo_lambda_init("test-basic-function");
    let src = root.join("src");
    let main = src.join("main.rs");
    create_dir_all(src).expect("failed to create src directory");

    let mut main_file = File::create(main).expect("failed to create main.rs file");
    let content = r#"""fn main() {
        println!("Hello, world!");
    }"""#;
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

    let out = read_to_string(main_file).expect("failed to read main.rs file");
    assert_eq!(content, out);

    let project = test_project(root);
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin("test-basic-function");
    assert!(bin.exists(), "{:?} doesn't exist", bin);
}
