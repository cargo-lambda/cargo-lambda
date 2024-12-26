use aws_smithy_types::retry::{RetryConfig, RetryMode};
use cargo_lambda_build::{create_binary_archive, zip_binary, BinaryArchive, BinaryData};
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_metadata::cargo::{
    deploy::{Deploy, OutputFormat},
    main_binary_from_metadata, CargoMetadata,
};
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Serialize;
use serde_json::ser::to_string_pretty;
use std::time::Duration;

mod dry;
mod extensions;
mod functions;
mod roles;

#[derive(Serialize)]
#[serde(untagged)]
#[allow(clippy::large_enum_variant)]
enum DeployResult {
    Extension(extensions::DeployOutput),
    Function(functions::DeployOutput),
    Dry(dry::DeployOutput),
}

impl std::fmt::Display for DeployResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeployResult::Extension(o) => o.fmt(f),
            DeployResult::Function(o) => o.fmt(f),
            DeployResult::Dry(o) => o.fmt(f),
        }
    }
}

#[tracing::instrument(target = "cargo_lambda")]
pub async fn run(config: &Deploy, metadata: &CargoMetadata) -> Result<()> {
    tracing::trace!("deploying project");

    if config.function_config.enable_function_url && config.function_config.disable_function_url {
        return Err(miette::miette!("invalid options: --enable-function-url and --disable-function-url cannot be set together"));
    }

    let progress = Progress::start("loading binary data");
    let (name, archive) = match load_archive(config, metadata) {
        Ok(arc) => arc,
        Err(err) => {
            progress.finish_and_clear();
            return Err(err);
        }
    };

    let retry = RetryConfig::standard()
        .with_retry_mode(RetryMode::Adaptive)
        .with_max_attempts(3)
        .with_initial_backoff(Duration::from_secs(5));

    let sdk_config = config.remote_config.sdk_config(Some(retry)).await;

    let result = if config.dry {
        dry::DeployOutput::new(config, &name, &archive).map(DeployResult::Dry)
    } else if config.extension {
        extensions::deploy(config, &name, &sdk_config, &archive, &progress)
            .await
            .map(DeployResult::Extension)
    } else {
        functions::deploy(config, &name, &sdk_config, &archive, &progress)
            .await
            .map(DeployResult::Function)
    };

    progress.finish_and_clear();
    let output = result?;

    match &config.output_format() {
        OutputFormat::Text => println!("{output}"),
        OutputFormat::Json => {
            let text = to_string_pretty(&output)
                .into_diagnostic()
                .wrap_err("failed to serialize output into json")?;
            println!("{text}")
        }
    }

    Ok(())
}

fn load_archive(config: &Deploy, metadata: &CargoMetadata) -> Result<(String, BinaryArchive)> {
    match &config.binary_path {
        Some(bp) if bp.is_dir() => Err(miette::miette!("invalid file {:?}", bp)),
        Some(bp) => {
            let name = match &config.name {
                Some(name) => name.clone(),
                None => bp
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(String::from)
                    .ok_or_else(|| miette::miette!("invalid binary path {:?}", bp))?,
            };

            let destination = bp
                .parent()
                .ok_or_else(|| miette::miette!("invalid binary path {:?}", bp))?;

            let data = BinaryData::new(&name, config.extension, config.internal);
            let arc = zip_binary(bp, destination, &data, config.include.clone())?;
            Ok((name, arc))
        }
        None => {
            let name = match (&config.name, &config.binary_name) {
                (Some(name), _) => name.clone(),
                (None, Some(bn)) => bn.clone(),
                (None, None) => main_binary_from_metadata(metadata)?,
            };
            let binary_name = binary_name_or_default(config, &name);
            let data = BinaryData::new(&binary_name, config.extension, config.internal);

            let arc = create_binary_archive(
                Some(metadata),
                &config.lambda_dir,
                &data,
                config.include.clone(),
            )?;
            Ok((name, arc))
        }
    }
}

pub(crate) fn binary_name_or_default(config: &Deploy, name: &str) -> String {
    config
        .binary_name
        .clone()
        .unwrap_or_else(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use assertables::assert_contains;
    use std::path::PathBuf;

    use cargo_lambda_metadata::cargo::load_metadata;

    use super::*;

    #[test]
    fn test_load_archive_from_binary_path() {
        let mut config = Deploy::default();
        config.binary_path = Some(PathBuf::from("../../tests/binaries/binary-x86-64"));
        config.include = Some(vec!["src".into()]);

        let metadata = load_metadata("../../tests/fixtures/examples-package/Cargo.toml").unwrap();
        let (name, archive) = load_archive(&config, &metadata).unwrap();
        assert_eq!(name, "binary-x86-64");

        let files = archive.list().unwrap();
        assert_contains!(files, &"bootstrap".to_string());
        assert_contains!(files, &"src/dry.rs".to_string());
        assert_contains!(files, &"src/extensions.rs".to_string());
        assert_contains!(files, &"src/functions.rs".to_string());
        assert_contains!(files, &"src/lib.rs".to_string());
        assert_contains!(files, &"src/roles.rs".to_string());
    }
}
