use std::path::PathBuf;

use crate::extract_tags;

use super::DeployResult;
use aws_sdk_s3::{primitives::ByteStream, Client as S3Client};
use cargo_lambda_build::BinaryArchive;
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_metadata::cargo::function_deploy_metadata;
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
    name: &str,
    manifest_path: &PathBuf,
    sdk_config: &SdkConfig,
    binary_archive: &BinaryArchive,
    architecture: Architecture,
    compatible_runtimes: Vec<Runtime>,
    s3_bucket: &Option<String>,
    tags: &Option<Vec<String>>,
    progress: &Progress,
) -> Result<DeployResult> {
    let lambda_client = LambdaClient::new(sdk_config);

    let mut deploy_metadata = function_deploy_metadata(manifest_path, name)
        .into_diagnostic()?
        .unwrap_or_default();

    if let Some(extra_files) = &deploy_metadata.include {
        binary_archive.add_files(extra_files)?;
    }

    let deploy_bucket = deploy_metadata
        .s3_bucket
        .as_ref()
        .or(s3_bucket.as_ref())
        .cloned();

    let input = match &deploy_bucket {
        None => LayerVersionContentInput::builder()
            .zip_file(Blob::new(binary_archive.read()?))
            .build(),
        Some(bucket) => {
            progress.set_message("uploading binary to S3");

            if let Some(tags) = tags {
                deploy_metadata.append_tags(extract_tags(tags));
            }

            let s3_client = S3Client::new(sdk_config);
            let mut operation = s3_client
                .put_object()
                .bucket(bucket)
                .key(name)
                .body(ByteStream::from(binary_archive.read()?));

            if tags.is_some() {
                operation = operation.set_tagging(deploy_metadata.s3_tags());
            }

            operation
                .send()
                .await
                .into_diagnostic()
                .wrap_err("failed to upload extension code to S3")?;

            LayerVersionContentInput::builder()
                .s3_bucket(bucket)
                .s3_key(name)
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
