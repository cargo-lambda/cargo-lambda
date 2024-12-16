use aws_smithy_types::retry::{RetryConfig, RetryMode};
use cargo_lambda_build::{create_binary_archive, zip_binary, BinaryArchive, BinaryData};
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_metadata::cargo::{
    deploy::{Deploy, OutputFormat},
    main_binary_from_metadata, CargoMetadata,
};
use cargo_lambda_remote::aws_sdk_lambda::types::Architecture;
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Serialize;
use serde_json::ser::to_string_pretty;
use std::{collections::HashMap, time::Duration};

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
pub async fn run(
    config: &Deploy,
    base_env: &HashMap<String, String>,
    metadata: &CargoMetadata,
) -> Result<()> {
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
    let architecture = Architecture::from(archive.architecture.as_str());

    let result = if config.dry {
        Ok(DeployResult::Dry(dry::DeployOutput::new(
            config, &name, &archive,
        )))
    } else if config.extension {
        extensions::deploy(
            config,
            &name,
            &sdk_config,
            &archive,
            architecture,
            &progress,
        )
        .await
    } else {
        functions::deploy(
            config,
            base_env,
            &name,
            &sdk_config,
            &archive,
            architecture,
            &progress,
        )
        .await
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
