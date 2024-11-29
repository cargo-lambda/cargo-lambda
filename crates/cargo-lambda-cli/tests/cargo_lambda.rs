use std::{
    fs::{create_dir_all, read_to_string, File},
    io::Write,
};
use zip::ZipArchive;

mod harness;
use harness::*;

#[test]
fn test_build_basic_function() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-basic-function", "function-template");

    lp.new_cmd()
        .arg("--no-interactive")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin(&lp.name);
    assert!(bin.exists(), "{:?} doesn't exist", bin);

    #[cfg(not(windows))]
    {
        cargo_lambda_dry_deploy(project.root()).assert().success();
        cargo_lambda_dry_deploy(project.root())
            .arg("--binary-name")
            .arg(&lp.name)
            .assert()
            .success();
    }
}

#[test]
fn test_build_basic_zip_function() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-basic-function", "function-template");

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

#[test]
fn test_build_basic_zip_function_with_include() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-basic-function", "function-template");

    lp.new_cmd()
        .arg("--no-interactive")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root())
        .args(["--output-format", "zip", "--include", "Cargo.toml"])
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
    assert!(
        zip.by_name("Cargo.toml").is_ok(),
        "Cargo.toml is not in the zip archive. Files in zip: {:?}",
        zip.file_names().collect::<Vec<&str>>().join(", ")
    );
}

#[test]
fn test_build_http_function() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-http-function", "function-template");

    lp.new_cmd().arg("--http").arg(&lp.name).assert().success();

    let project = lp.test_project();
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin(&lp.name);
    assert!(bin.exists(), "{:?} doesn't exist", bin);

    #[cfg(not(windows))]
    {
        cargo_lambda_dry_deploy(project.root()).assert().success();
        cargo_lambda_dry_deploy(project.root())
            .arg("--binary-name")
            .arg(&lp.name)
            .assert()
            .success();
    }
}

#[test]
fn test_build_http_feature_function() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-http-function", "function-template");

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

#[test]
fn test_build_event_type_function() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-event-type-function", "function-template");

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

#[test]
fn test_build_basic_extension() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-basic-extension", "extension-template");

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

    #[cfg(not(windows))]
    cargo_lambda_dry_deploy(project.root())
        .arg("--extension")
        .arg(&lp.name)
        .assert()
        .success();
}

#[test]
fn test_build_logs_extension() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-logs-extension", "extension-template");

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

#[test]
fn test_build_telemetry_extension() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-telemetry-extension", "extension-template");

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

#[test]
fn test_init_subcommand() {
    let _guard = init_root();
    let lp = cargo_lambda_init("test-basic-function", "function-template");
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

#[test]
fn test_init_subcommand_without_override() {
    let _guard = init_root();
    let lp = cargo_lambda_init("test-basic-function", "function-template");
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

#[test]
fn test_build_basic_zip_extension() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-basic-extension", "extension-template");

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

#[test]
fn test_build_internal_zip_extension() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-internal-extension", "extension-template");

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

#[test]
fn test_build_example() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-example", "function-template");
    lp.new_cmd()
        .arg("--no-interactive")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    let root = project.root();

    let examples_dir = root.join("examples");
    let example = examples_dir.join("example-lambda.rs");
    create_dir_all(examples_dir).expect("failed to create examples directory");

    let mut example_file = File::create(example).expect("failed to create main.rs file");
    let content = r#"fn main() {
        println!("Hello, world!");
    }"#;
    example_file
        .write_all(content.as_bytes())
        .expect("failed to create example content");
    example_file.flush().unwrap();

    // Build examples and check that only the example is in the Lambda directory.
    cargo_lambda_build(project.root())
        .arg("--examples")
        .assert()
        .success();

    let bin = project.lambda_function_bin("example-lambda");
    assert!(bin.exists(), "{:?} doesn't exist", bin);

    let bin = project.lambda_function_bin(&lp.name);
    assert!(!bin.exists(), "{:?} exists, but it shoudn't", bin);

    #[cfg(not(windows))]
    cargo_lambda_dry_deploy(project.root())
        .arg("--binary-name")
        .arg("example-lambda")
        .assert()
        .success();

    project.lambda_dir().rm_rf();

    // Build project and check that only the main binary is in the Lambda directory.
    cargo_lambda_build(project.root()).assert().success();

    let bin = project.lambda_function_bin("example-lambda");
    assert!(!bin.exists(), "{:?} exists, but it shouldn't", bin);

    let bin = project.lambda_function_bin(&lp.name);
    assert!(
        bin.exists(),
        "{:?} doesn't exist in directory: {:?}",
        &lp.name,
        project.lambda_dir().ls_r()
    );

    #[cfg(not(windows))]
    {
        cargo_lambda_dry_deploy(project.root()).assert().success();
        cargo_lambda_dry_deploy(project.root())
            .arg("--binary-name")
            .arg(&lp.name)
            .assert()
            .success();
    }
}

#[test]
fn test_deploy_workspace() {
    let _guard = init_root();
    let workspace = project().build();
    let crates = workspace.root().join("crates");
    crates.mkdir_p();

    let lp_1 = cargo_lambda_new_in_root("p1", "function-template", &crates);
    lp_1.new_cmd()
        .arg("--no-interactive")
        .arg(&lp_1.name)
        .assert()
        .success();

    let lp_2 = cargo_lambda_new_in_root("p2", "function-template", &crates);
    lp_2.new_cmd()
        .arg("--no-interactive")
        .arg(&lp_2.name)
        .assert()
        .success();

    let mut manifest = File::create(workspace.root().join("Cargo.toml"))
        .expect("failed to create Cargo.toml file");
    let content = format!(
        r#"[workspace]
resolver = "2"
members = ["crates/{}", "crates/{}"]
"#,
        &lp_1.name, &lp_2.name
    );
    manifest
        .write_all(content.as_bytes())
        .expect("failed to create manifest content");
    manifest.flush().unwrap();

    cargo_lambda_build(workspace.root())
        .arg("--package")
        .arg(&lp_1.name)
        .assert()
        .success();

    #[cfg(not(windows))]
    {
        cargo_lambda_dry_deploy(workspace.root())
            .arg(&lp_1.name)
            .assert()
            .success();
        cargo_lambda_dry_deploy(workspace.root())
            .arg("--binary-name")
            .arg(&lp_1.name)
            .assert()
            .success();
    }
}

#[test]
fn test_build_zip_workspace() {
    let _guard = init_root();
    let workspace = project().build();
    let crates = workspace.root().join("crates");
    crates.mkdir_p();

    let lp_1 = cargo_lambda_new_in_root("p1", "function-template", &crates);
    lp_1.new_cmd()
        .arg("--no-interactive")
        .arg(&lp_1.name)
        .assert()
        .success();

    let lp_2 = cargo_lambda_new_in_root("p2", "function-template", &crates);
    lp_2.new_cmd()
        .arg("--no-interactive")
        .arg(&lp_2.name)
        .assert()
        .success();

    let mut manifest = File::create(workspace.root().join("Cargo.toml"))
        .expect("failed to create Cargo.toml file");
    let content = format!(
        r#"[workspace]
resolver = "2"
members = ["crates/{}", "crates/{}"]
"#,
        &lp_1.name, &lp_2.name
    );
    manifest
        .write_all(content.as_bytes())
        .expect("failed to create manifest content");
    manifest.flush().unwrap();

    // Build all binaries first. The second build should zip only one of them.
    cargo_lambda_build(workspace.root()).assert().success();
    let lp_1_bin = workspace
        .lambda_dir()
        .join(&lp_1.name)
        .join("bootstrap.zip");
    assert!(!lp_1_bin.exists(), "{:?} exist", lp_1_bin);
    let lp_2_bin = workspace
        .lambda_dir()
        .join(&lp_2.name)
        .join("bootstrap.zip");
    assert!(!lp_2_bin.exists(), "{:?} exist", lp_2_bin);

    cargo_lambda_build(workspace.root())
        .args(["--bin", &lp_1.name, "--output-format", "zip"])
        .assert()
        .success();

    assert!(lp_1_bin.exists(), "{:?} doesn't exist", lp_1_bin);
    // The second zip file should not be created because we're using `--bin`.
    assert!(!lp_2_bin.exists(), "{:?} exist", lp_2_bin);
}

#[test]
fn test_config_template() {
    let _guard = init_root();
    let lp = cargo_lambda_new("test-config-template", "config-template");
    lp.new_cmd()
        .arg("--no-interactive")
        .arg(&lp.name)
        .assert()
        .success();

    let project = lp.test_project();
    cargo_lambda_build(project.root()).assert().success();

    let json_file = read_to_string(project.root().join("render-test.json")).unwrap();
    let json_data: serde_json::Value = serde_json::from_str(&json_file).unwrap();
    assert_eq!(json_data["description"], "My Lambda");
    assert_eq!(json_data["enable_tracing"], "false");
    assert_eq!(json_data["runtime"], "provided.al2023");
    assert_eq!(json_data["architecture"], "x86_64");
    assert_eq!(json_data["memory"], "128");
    assert_eq!(json_data["timeout"], "3");
    assert_eq!(json_data["ci_provider"], ".github");
    assert_eq!(json_data["github_actions"], "false");
    assert_eq!(json_data["license"], "Ignore license");

    assert!(project
        .root()
        .join(".github")
        .join("actions")
        .join("build.yml")
        .exists());

    assert!(!project.root().join("Apache.txt").exists());
    assert!(!project.root().join("MIT.txt").exists());
}
