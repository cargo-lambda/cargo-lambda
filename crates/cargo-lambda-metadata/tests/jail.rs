// Jail tests are in a separate module because they mess with the current path
// where cargo runs from, and it makes other tests fail randomly because they
// cannot find the Cargo.toml file for test fixtures.

use figment::Jail;

use cargo_lambda_metadata::{
    cargo::load_metadata,
    config::{ConfigOptions, load_config_without_cli_flags},
};

#[test]
fn test_env() {
    Jail::expect_with(|jail| {
        jail.set_env("CARGO_LAMBDA_BUILD.RELEASE", "true");
        jail.set_env("CARGO_LAMBDA_DEPLOY.MEMORY", "1024");
        jail.set_env("CARGO_LAMBDA_DEPLOY.TIMEOUT", "60");

        jail.create_file(
            "Cargo.toml",
            r#"
            [package]
            name = "test"

            [[bin]]
            name = "test"
            path = "src/main.rs"
        "#,
        )?;

        let metadata = load_metadata("Cargo.toml").unwrap();
        let config = load_config_without_cli_flags(&metadata, &ConfigOptions::default()).unwrap();

        assert!(config.build.cargo_opts.release);
        assert_eq!(config.deploy.function_config.memory, Some(1024.into()));
        assert_eq!(config.deploy.function_config.timeout, Some(60.into()));

        Ok(())
    });
}

#[test]
fn test_env_with_context() {
    Jail::expect_with(|jail| {
        jail.set_env("CARGO_LAMBDA_DEPLOY.MEMORY", "1024");

        jail.create_file(
            "Cargo.toml",
            r#"
                [package]
                name = "test"
    
                [[bin]]
                name = "test"
                path = "src/main.rs"
            "#,
        )?;

        let options = ConfigOptions {
            context: Some("production".to_string()),
            ..Default::default()
        };

        let metadata = load_metadata("Cargo.toml").unwrap();
        let config = load_config_without_cli_flags(&metadata, &options).unwrap();

        assert_eq!(config.deploy.function_config.memory, Some(1024.into()));

        Ok(())
    });
}

#[test]
fn test_env_with_arrays() {
    Jail::expect_with(|jail| {
        jail.set_env("CARGO_LAMBDA_BUILD.FEATURES", "[lambda, env]");
        jail.set_env("CARGO_LAMBDA_DEPLOY.ENV", "[FOO=BAR, BAZ=QUX]");

        jail.create_file(
            "Cargo.toml",
            r#"
                [package]
                name = "test"

                [[bin]]
                name = "test"
                path = "src/main.rs"
            "#,
        )?;

        let metadata = load_metadata("Cargo.toml").unwrap();
        let config = load_config_without_cli_flags(&metadata, &ConfigOptions::default()).unwrap();

        assert_eq!(config.build.cargo_opts.features, vec!["lambda", "env"]);

        let env = config
            .deploy
            .lambda_environment()
            .unwrap()
            .unwrap()
            .variables()
            .cloned()
            .unwrap_or_default();

        assert_eq!(env.get("FOO"), Some(&"BAR".to_string()));
        assert_eq!(env.get("BAZ"), Some(&"QUX".to_string()));

        Ok(())
    });
}
