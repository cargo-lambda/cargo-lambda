use cargo_test_macro::cargo_test;

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
