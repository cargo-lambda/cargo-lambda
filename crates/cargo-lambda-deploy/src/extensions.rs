use super::DeployResult;
use aws_sdk_s3::{primitives::ByteStream, Client as S3Client};
use cargo_lambda_build::BinaryArchive;
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_metadata::cargo::deploy::Deploy;
use cargo_lambda_remote::{
    aws_sdk_config::SdkConfig,
    aws_sdk_lambda::{
        primitives::Blob,
        types::{Architecture, LayerVersionContentInput, Runtime},
        Client as LambdaClient,
    },
};
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Serialize;
use tracing::debug;

#[derive(Serialize)]
pub(crate) struct DeployOutput {
    extension_arn: String,
}

impl std::fmt::Display for DeployOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "🔍 extension arn: {}", self.extension_arn)
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn deploy(
    config: &Deploy,
    name: &str,
    sdk_config: &SdkConfig,
    binary_archive: &BinaryArchive,
    architecture: Architecture,
    progress: &Progress,
) -> Result<DeployResult> {
    let lambda_client = LambdaClient::new(sdk_config);

    if let Some(extra_files) = &config.include {
        binary_archive.add_files(extra_files)?;
    }

    let compatible_runtimes = config
        .compatible_runtimes()
        .iter()
        .map(|runtime| Runtime::from(runtime.as_str()))
        .collect::<Vec<_>>();

    let input = match &config.s3_bucket {
        None => LayerVersionContentInput::builder()
            .zip_file(Blob::new(binary_archive.read()?))
            .build(),
        Some(bucket) => {
            progress.set_message("uploading binary to S3");

            let key = config.s3_key.as_deref().unwrap_or(name);
            debug!(bucket, key, "uploading zip to S3");

            let s3_client = S3Client::new(sdk_config);
            let mut operation = s3_client
                .put_object()
                .bucket(bucket)
                .key(key)
                .body(ByteStream::from(binary_archive.read()?));

            if let Some(tags) = config.s3_tags() {
                operation = operation.tagging(tags);
            }

            operation
                .send()
                .await
                .into_diagnostic()
                .wrap_err("failed to upload extension code to S3")?;

            LayerVersionContentInput::builder()
                .s3_bucket(bucket)
                .s3_key(key)
                .build()
        }
    };

    progress.set_message("publishing new layer version");

    let output = lambda_client
        .publish_layer_version()
        .layer_name(name)
        .compatible_architectures(architecture)
        .set_compatible_runtimes(Some(compatible_runtimes))
        .content(input)
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to publish extension")?;

    Ok(DeployResult::Extension(DeployOutput {
        extension_arn: output.layer_version_arn.expect("missing ARN"),
    }))
}
