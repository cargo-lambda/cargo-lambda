use super::DeployResult;
use aws_sdk_s3::{types::ByteStream, Client as S3Client};
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_remote::{
    aws_sdk_config::SdkConfig,
    aws_sdk_lambda::{
        model::{Architecture, LayerVersionContentInput, Runtime},
        types::Blob,
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

pub(crate) async fn deploy(
    name: &str,
    sdk_config: &SdkConfig,
    binary_data: Vec<u8>,
    architecture: Architecture,
    s3_bucket: &Option<String>,
    progress: &Progress,
) -> Result<DeployResult> {
    let lambda_client = LambdaClient::new(sdk_config);

    let input = match s3_bucket {
        None => LayerVersionContentInput::builder()
            .zip_file(Blob::new(binary_data))
            .build(),
        Some(bucket) => {
            progress.set_message("uploading binary to S3");

            let s3_client = S3Client::new(sdk_config);
            s3_client
                .put_object()
                .bucket(bucket)
                .key(name)
                .body(ByteStream::from(binary_data))
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
        .compatible_runtimes(Runtime::Providedal2)
        .content(input)
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to publish extension")?;

    Ok(DeployResult::Extension(DeployOutput {
        extension_arn: output.layer_version_arn.expect("missing ARN"),
    }))
}
