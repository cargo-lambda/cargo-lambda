use crate::{cargo_lambda_build, cargo_lambda_new, test_project};
use cargo_test_macro::cargo_test;

#[cargo_test]
fn test_build_basic_function() {
    let (root, cmd) = cargo_lambda_new();

    cmd.arg("--no-interactive")
        .arg("test-basic-function")
        .assert()
        .success();

    let project = test_project(root.join("test-basic-function"));
    cargo_lambda_build(project).assert().success();
}

#[cargo_test]
fn test_build_http_function() {
    let (root, cmd) = cargo_lambda_new();

    cmd.arg("--http-feature")
        .arg("apigw_rest")
        .arg("test-http-function")
        .assert()
        .success();

    let project = test_project(root.join("test-http-function"));
    cargo_lambda_build(project).assert().success();
}

#[cargo_test]
fn test_build_basic_extension() {
    let (root, cmd) = cargo_lambda_new();

    cmd.arg("--extension")
        .arg("test-basic-extension")
        .assert()
        .success();

    let project = test_project(root.join("test-basic-extension"));
    cargo_lambda_build(project)
        .arg("--extension")
        .assert()
        .success();
}

#[cargo_test]
fn test_build_logs_extension() {
    let (root, cmd) = cargo_lambda_new();

    cmd.arg("--extension")
        .arg("--logs")
        .arg("test-logs-extension")
        .assert()
        .success();

    let project = test_project(root.join("test-logs-extension"));
    cargo_lambda_build(project)
        .arg("--extension")
        .assert()
        .success();
}

#[cargo_test]
fn test_build_telemetry_extension() {
    let (root, cmd) = cargo_lambda_new();

    cmd.arg("--extension")
        .arg("--telemetry")
        .arg("test-telemetry-extension")
        .assert()
        .success();

    let project = test_project(root.join("test-telemetry-extension"));
    cargo_lambda_build(project)
        .arg("--extension")
        .assert()
        .success();
}
