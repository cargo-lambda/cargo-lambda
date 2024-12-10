// Jail tests are in a separate module because they mess with the current path
// where cargo runs from, and it makes other tests fail randomly because they
// cannot find the Cargo.toml file for test fixtures.

use figment::Jail;

use cargo_lambda_metadata::{
    cargo::load_metadata,
    config::{load_config_without_cli_flags, ConfigOptions},
    lambda::Memory,
};

#[test]
fn test_jail() {
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
        assert_eq!(config.deploy.function_config.memory, Some(Memory::Mb1024));
        assert_eq!(config.deploy.function_config.timeout, Some(60.into()));

        jail.clear_env();

        Ok(())
    });
}
