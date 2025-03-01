use crate::roles::{self, FunctionRole};
use aws_sdk_cloudwatchlogs::operation::create_log_group::CreateLogGroupError;
use aws_sdk_s3::{Client as S3Client, primitives::ByteStream};
use cargo_lambda_build::{BinaryArchive, BinaryModifiedAt};
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_metadata::cargo::deploy::Deploy;
use cargo_lambda_remote::{
    aws_sdk_config::SdkConfig,
    aws_sdk_lambda::{
        Client as LambdaClient,
        error::SdkError,
        operation::{
            create_function::CreateFunctionError,
            delete_function_url_config::DeleteFunctionUrlConfigError,
            get_alias::GetAliasError,
            get_function::{GetFunctionError, GetFunctionOutput},
            get_function_url_config::GetFunctionUrlConfigError,
        },
        primitives::Blob,
        types::{
            FunctionCode, FunctionConfiguration, FunctionUrlAuthType, LastUpdateStatus, Runtime,
            State, VpcConfig as LambdaVpcConfig,
        },
    },
};
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Serialize;
use std::{collections::HashMap, str::FromStr};
use tokio::time::{Duration, sleep};
use tracing::debug;
use uuid::Uuid;

enum FunctionAction {
    Create,
    Update(Box<GetFunctionOutput>),
}

#[derive(Serialize)]
pub(crate) struct DeployOutput {
    function_arn: String,
    function_url: Option<String>,
    binary_modified_at: BinaryModifiedAt,
    version: String,
    alias: Option<String>,
}

impl std::fmt::Display for DeployOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "✅ function deployed successfully 🎉")?;
        writeln!(
            f,
            "🛠️  binary last compiled {}",
            self.binary_modified_at.humanize()
        )?;
        writeln!(f, "🔍 arn: {}", self.function_arn)?;
        write!(f, "🎭 version: {}", self.version)?;
        if let Some(alias) = &self.alias {
            write!(f, "\n🪢 alias: {alias}")?;
        }
        if let Some(url) = &self.function_url {
            write!(f, "\n🔗 url: {url}")?;
        }
        Ok(())
    }
}

pub(crate) async fn deploy(
    config: &Deploy,
    name: &str,
    sdk_config: &SdkConfig,
    binary_archive: &BinaryArchive,
    progress: &Progress,
) -> Result<DeployOutput> {
    let client = LambdaClient::new(sdk_config);

    let (function_arn, version) =
        upsert_function(config, name, &client, sdk_config, binary_archive, progress).await?;

    if let Some(alias) = &config.remote_config.alias {
        progress.set_message("updating alias version");

        upsert_alias(name, alias, &version, &client).await?;
    }

    let function_url = if config.function_config.enable_function_url {
        progress.set_message("configuring function url");

        Some(upsert_function_url_config(name, &config.remote_config.alias, &client).await?)
    } else {
        None
    };

    if config.function_config.disable_function_url {
        progress.set_message("deleting function url configuration");

        delete_function_url_config(name, &config.remote_config.alias, &client).await?;
    }

    if let Some(retention) = config.function_config.log_retention {
        progress.set_message("setting log retention");
        set_log_retention(sdk_config, retention, name).await?;
    }

    Ok(DeployOutput {
        function_arn,
        function_url,
        version,
        alias: config.remote_config.alias.clone(),
        binary_modified_at: binary_archive.binary_modified_at.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn upsert_function(
    config: &Deploy,
    name: &str,
    client: &LambdaClient,
    sdk_config: &SdkConfig,
    binary_archive: &BinaryArchive,
    progress: &Progress,
) -> Result<(String, String)> {
    let current_function = client.get_function().function_name(name).send().await;

    let action = match current_function {
        Ok(fun) => FunctionAction::Update(Box::new(fun)),
        Err(no_fun) if function_doesnt_exist_error(&no_fun) => FunctionAction::Create,
        Err(no_fun) => {
            return Err(no_fun)
                .into_diagnostic()
                .wrap_err("failed to fetch lambda function");
        }
    };

    let s3_client = S3Client::new(sdk_config);

    let (arn, version) = match action {
        FunctionAction::Create => {
            let function_role = match &config.function_config.role {
                None => roles::create(sdk_config, progress).await?,
                Some(role) => FunctionRole::from_existing(role.clone()),
            };

            create_function(
                config,
                name,
                client,
                &s3_client,
                binary_archive,
                progress,
                function_role,
            )
            .await?
        }
        FunctionAction::Update(fun) => {
            progress.set_message("deploying function");

            let conf = fun
                .configuration
                .ok_or_else(|| miette::miette!("missing function configuration"))?;

            let function_arn = update_function_config(config, name, client, progress, conf).await?;

            tag_function(client, config.lambda_tags(), function_arn).await?;

            update_function_code(config, name, client, &s3_client, binary_archive, progress).await?
        }
    };

    Ok((
        arn.expect("missing function ARN"),
        version.expect("missing function version"),
    ))
}

async fn tag_function(
    client: &LambdaClient,
    lambda_tags: Option<HashMap<String, String>>,
    function_arn: String,
) -> Result<()> {
    let Some(tags) = lambda_tags else {
        return Ok(());
    };

    if tags.is_empty() {
        return Ok(());
    }

    client
        .tag_resource()
        .resource(&function_arn)
        .set_tags(Some(tags))
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to tag function")
        .map(|_| ())
}

#[allow(clippy::too_many_arguments)]
async fn create_function(
    config: &Deploy,
    name: &str,
    lambda_client: &LambdaClient,
    s3_client: &S3Client,
    binary_archive: &BinaryArchive,
    progress: &Progress,
    function_role: FunctionRole,
) -> Result<(Option<String>, Option<String>)> {
    debug!(?function_role, ?config, "creating new function");
    progress.set_message("deploying function");

    let code = match &config.s3_bucket {
        None => {
            debug!("uploading zip to Lambda");
            let blob = Blob::new(binary_archive.read()?);
            FunctionCode::builder().zip_file(blob).build()
        }
        Some(bucket) => {
            let key = config.s3_key.as_deref().unwrap_or(name);
            debug!(bucket, key, "uploading zip to S3");
            s3_client
                .put_object()
                .bucket(bucket)
                .key(key)
                .body(ByteStream::from(binary_archive.read()?))
                .set_tagging(config.s3_tags())
                .send()
                .await
                .into_diagnostic()
                .wrap_err("failed to upload function code to S3")?;
            FunctionCode::builder()
                .s3_bucket(bucket)
                .s3_key(key)
                .build()
        }
    };

    let runtime = Runtime::from_str(&config.function_config.runtime()).unwrap();
    let mut output = None;

    for attempt in 2..5 {
        let memory = config.function_config.memory.clone().map(Into::into);
        let timeout = config
            .function_config
            .timeout
            .clone()
            .unwrap_or_default()
            .into();

        let mut function = lambda_client.create_function();
        if let Some(vpc) = &config.function_config.vpc {
            function = function.vpc_config(
                LambdaVpcConfig::builder()
                    .set_security_group_ids(vpc.security_group_ids.clone())
                    .set_subnet_ids(vpc.subnet_ids.clone())
                    .ipv6_allowed_for_dual_stack(vpc.ipv6_allowed_for_dual_stack)
                    .build(),
            );
        }

        let result = function
            .runtime(runtime.clone())
            .handler("bootstrap")
            .function_name(name)
            .role(function_role.arn())
            .architectures(binary_archive.architecture())
            .code(code.clone())
            .publish(config.publish_code_without_description())
            .set_memory_size(memory)
            .timeout(timeout)
            .set_tracing_config(config.tracing_config())
            .set_environment(config.lambda_environment()?)
            .set_layers(config.function_config.layer.clone())
            .set_tags(config.lambda_tags())
            .send()
            .await;

        match result {
            Ok(o) => {
                output = Some(o);
                break;
            }
            Err(err)
                if is_role_cannot_be_assumed_error(&err)
                    && function_role.is_new()
                    && attempt < 5 =>
            {
                let backoff = attempt * 5;
                progress.set_message(&format!(
                    "new role not full propagated, waiting {backoff} seconds before retrying"
                ));
                sleep(Duration::from_secs(backoff)).await;
                progress.set_message("trying to deploy function again");
            }
            Err(err) => {
                return Err(err)
                    .into_diagnostic()
                    .wrap_err("failed to create the new lambda function");
            }
        };
    }

    if let Some(description) = &config.function_config.description {
        wait_for_ready_state(lambda_client, name, &config.remote_config.alias, progress).await?;

        let result = lambda_client
            .publish_version()
            .function_name(name)
            .description(description)
            .send()
            .await;

        match result {
            Ok(o) => Ok((o.function_arn, o.version)),
            Err(err) => Err(err)
                .into_diagnostic()
                .wrap_err("failed to publish the new lambda version"),
        }
    } else {
        output
            .map(|o| (o.function_arn, o.version))
            .ok_or_else(|| miette::miette!("failed to create new lambda function"))
    }
}

async fn update_function_config(
    config: &Deploy,
    name: &str,
    client: &LambdaClient,
    progress: &Progress,
    conf: FunctionConfiguration,
) -> Result<String> {
    let function_arn = conf.function_arn.as_ref().expect("missing function arn");

    let mut wait_for_readiness = false;
    if conf.state.is_none() || conf.state == Some(State::Pending) {
        wait_for_readiness = true;
    }
    if conf
        .last_update_status()
        .is_some_and(|s| s == &LastUpdateStatus::InProgress)
    {
        wait_for_readiness = true;
    }
    if wait_for_readiness {
        wait_for_ready_state(client, name, &config.remote_config.alias, progress).await?;
        progress.set_message("deploying function");
    }

    let mut update_config = false;
    let mut builder = client.update_function_configuration().function_name(name);

    if config.function_config.should_update() {
        if let Some(iam_role) = &config.function_config.role {
            builder = builder.role(iam_role);
        }

        let memory = config.function_config.memory.clone().map(Into::into);
        if memory.is_some() && conf.memory_size != memory {
            update_config = true;
            builder = builder.set_memory_size(memory);
        }

        if let Some(timeout) = &config.function_config.timeout {
            let timeout: i32 = timeout.into();
            if conf.timeout.unwrap_or_default() != timeout {
                update_config = true;
                builder = builder.timeout(timeout);
            }
        }

        if should_update_layers(&config.function_config.layer, &conf) {
            update_config = true;
            builder = builder.set_layers(config.function_config.layer.clone());
        }

        if let Some(environment) = config.lambda_environment()? {
            if let Some(vars) = environment.variables() {
                if !vars.is_empty()
                    && vars
                        != &conf
                            .environment
                            .clone()
                            .and_then(|e| e.variables)
                            .unwrap_or_default()
                {
                    update_config = true;
                    builder = builder.environment(environment);
                }
            }
        }

        let tracing_config = config.tracing_config();
        if let Some(tracing_config) = tracing_config {
            let default_mode = conf.tracing_config.map(|t| t.mode).unwrap_or_default();
            if tracing_config.mode != default_mode {
                update_config = true;
                builder = builder.tracing_config(tracing_config);
            }
        }

        if let Some(vpc) = &config.function_config.vpc {
            if vpc.should_update() {
                update_config = true;
                builder = builder.vpc_config(
                    LambdaVpcConfig::builder()
                        .set_security_group_ids(vpc.security_group_ids.clone())
                        .set_subnet_ids(vpc.subnet_ids.clone())
                        .ipv6_allowed_for_dual_stack(vpc.ipv6_allowed_for_dual_stack)
                        .build(),
                );
            }
        }
    }

    if update_config {
        debug!("updating function's configuration");
        let result = builder
            .send()
            .await
            .into_diagnostic()
            .wrap_err("failed to update function configuration")?;

        if result.last_update_status() == Some(&LastUpdateStatus::InProgress) {
            wait_for_ready_state(client, name, &config.remote_config.alias, progress).await?;
        }
        progress.set_message("deploying function");
    }

    Ok(function_arn.clone())
}

async fn update_function_code(
    config: &Deploy,
    name: &str,
    lambda_client: &LambdaClient,
    s3_client: &S3Client,
    binary_archive: &BinaryArchive,
    progress: &Progress,
) -> Result<(Option<String>, Option<String>)> {
    let mut builder = lambda_client.update_function_code().function_name(name);

    match &config.s3_bucket {
        None => {
            debug!("uploading zip to Lambda");
            let blob = Blob::new(binary_archive.read()?);
            builder = builder.zip_file(blob)
        }
        Some(bucket) => {
            let key = config.s3_key.as_deref().unwrap_or(name);

            debug!(bucket, key, "uploading zip to S3");

            let mut operation = s3_client
                .put_object()
                .bucket(bucket)
                .key(key)
                .body(ByteStream::from(binary_archive.read()?));

            let s3_tags = config.s3_tags();
            if s3_tags.is_some() {
                operation = operation.set_tagging(s3_tags);
            }
            operation
                .send()
                .await
                .into_diagnostic()
                .wrap_err("failed to upload function code to S3")?;

            builder = builder.s3_bucket(bucket).s3_key(key);
        }
    }

    let output = builder
        .publish(config.publish_code_without_description())
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to update function code")?;

    if let Some(description) = &config.function_config.description {
        wait_for_ready_state(lambda_client, name, &config.remote_config.alias, progress).await?;
        let result = lambda_client
            .publish_version()
            .function_name(name)
            .description(description)
            .send()
            .await;

        match result {
            Ok(o) => Ok((o.function_arn, o.version)),
            Err(err) => Err(err)
                .into_diagnostic()
                .wrap_err("failed to publish the new lambda version"),
        }
    } else {
        Ok((output.function_arn, output.version))
    }
}

/// Wait until the function state has been completely propagated.
async fn wait_for_ready_state(
    client: &LambdaClient,
    name: &str,
    alias: &Option<String>,
    progress: &Progress,
) -> Result<()> {
    for attempt in 2..5 {
        let backoff = attempt * attempt;
        progress.set_message(&format!(
            "AWS Lambda is processing your function's configuration. Waiting {backoff} seconds before checking for status updates"
        ));
        sleep(Duration::from_secs(backoff)).await;

        let conf = client
            .get_function_configuration()
            .function_name(name)
            .set_qualifier(alias.clone())
            .send()
            .await
            .into_diagnostic()
            .wrap_err("failed to fetch the function configuration")?;

        debug!(function_state = ?conf.state, last_update_status = ?conf.last_update_status, "function state");

        let Some(state) = &conf.state else {
            return Err(miette::miette!("unknown function state"));
        };

        match (state, conf.last_update_status) {
            (State::Pending, _) => {} // wait for the function to be ready
            (
                State::Active | State::Inactive | State::Failed,
                Some(LastUpdateStatus::InProgress),
            ) => {} // wait for the function to be ready

            (
                State::Active | State::Inactive | State::Failed,
                None | Some(LastUpdateStatus::Failed | LastUpdateStatus::Successful),
            ) => break, // function is ready

            (State::Active | State::Inactive | State::Failed, other) => {
                return Err(miette::miette!(
                    "unexpected last update status: {:?}",
                    other
                ));
            }

            (other, _) => return Err(miette::miette!("unexpected function state: {:?}", other)),
        }

        if attempt == 5 {
            return Err(miette::miette!(
                "configuration update didn't finish in time, wait a few minutes and try again"
            ));
        }
    }

    Ok(())
}

pub(crate) fn should_update_layers(
    layer_arn: &Option<Vec<String>>,
    conf: &FunctionConfiguration,
) -> bool {
    match (conf.layers(), layer_arn) {
        ([], None) => false,
        (_cl, None) => true,
        ([], Some(_)) => true,
        (cl, Some(nl)) => {
            let mut c = cl
                .iter()
                .cloned()
                .map(|l| l.arn.unwrap_or_default())
                .collect::<Vec<_>>();
            c.sort();

            let mut n = nl.to_vec();
            n.sort();
            c != n
        }
    }
}

pub(crate) async fn upsert_alias(
    name: &str,
    alias: &str,
    version: &str,
    client: &LambdaClient,
) -> Result<()> {
    let current_alias = client
        .get_alias()
        .name(alias)
        .function_name(name)
        .send()
        .await;

    match current_alias {
        Ok(_) => {
            client
                .update_alias()
                .name(alias)
                .function_name(name)
                .function_version(version)
                .send()
                .await
                .into_diagnostic()
                .wrap_err("failed to update alias")?;
        }
        Err(no_fun) if alias_doesnt_exist_error(&no_fun) => {
            client
                .create_alias()
                .name(alias)
                .function_name(name)
                .function_version(version)
                .send()
                .await
                .into_diagnostic()
                .wrap_err("failed to create alias")?;
        }
        Err(no_fun) => {
            return Err(no_fun)
                .into_diagnostic()
                .wrap_err("failed to fetch alias");
        }
    };

    Ok(())
}

pub(crate) async fn upsert_function_url_config(
    name: &str,
    alias: &Option<String>,
    client: &LambdaClient,
) -> Result<String> {
    let result = client
        .get_function_url_config()
        .function_name(name)
        .set_qualifier(alias.clone())
        .send()
        .await;

    let url = match result {
        Ok(fun) => fun.function_url,
        Err(no_fun) if function_url_config_doesnt_exist_error(&no_fun) => {
            let statement = format!("FunctionUrlAllowPublicAccess-{}", Uuid::new_v4());
            client
                .add_permission()
                .function_name(name)
                .set_qualifier(alias.clone())
                .action("lambda:InvokeFunctionUrl")
                .principal("*")
                .statement_id(statement)
                .function_url_auth_type(FunctionUrlAuthType::None)
                .send()
                .await
                .into_diagnostic()
                .wrap_err("failed to enable function url invocations")?;

            let output = client
                .create_function_url_config()
                .function_name(name)
                .auth_type(FunctionUrlAuthType::None)
                .set_qualifier(alias.clone())
                .send()
                .await
                .into_diagnostic()
                .wrap_err("failed to create function url configuration")?;
            output.function_url
        }
        Err(no_fun) => {
            return Err(no_fun)
                .into_diagnostic()
                .wrap_err("failed to fetch function url configuration")?;
        }
    };

    Ok(url)
}

pub(crate) async fn delete_function_url_config(
    name: &str,
    alias: &Option<String>,
    client: &LambdaClient,
) -> Result<()> {
    let result = client
        .delete_function_url_config()
        .function_name(name)
        .set_qualifier(alias.clone())
        .send()
        .await;

    match result {
        Ok(_) => Ok(()),
        Err(no_fun) if delete_function_url_config_doesnt_exist_error(&no_fun) => Ok(()),
        Err(no_fun) => Err(no_fun)
            .into_diagnostic()
            .wrap_err("failed to delete function url configuration"),
    }
}

async fn set_log_retention(sdk_config: &SdkConfig, retention: i32, name: &str) -> Result<()> {
    let cw_client = aws_sdk_cloudwatchlogs::Client::new(sdk_config);
    let log_group_name = format!("/aws/lambda/{name}");

    match cw_client
        .create_log_group()
        .log_group_name(&log_group_name)
        .send()
        .await
    {
        Ok(_) => (),
        Err(err) if log_group_already_exists_error(&err) => (),
        Err(err) => {
            return Err(err)
                .into_diagnostic()
                .wrap_err("failed to create log group");
        }
    }

    cw_client
        .put_retention_policy()
        .log_group_name(log_group_name)
        .retention_in_days(retention)
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to set log retention")?;
    Ok(())
}

fn log_group_already_exists_error(err: &SdkError<CreateLogGroupError>) -> bool {
    match err {
        SdkError::ServiceError(e) => e.err().is_resource_already_exists_exception(),
        _ => false,
    }
}

pub(crate) fn function_doesnt_exist_error(err: &SdkError<GetFunctionError>) -> bool {
    match err {
        SdkError::ServiceError(e) => e.err().is_resource_not_found_exception(),
        _ => false,
    }
}

pub(crate) fn function_url_config_doesnt_exist_error(
    err: &SdkError<GetFunctionUrlConfigError>,
) -> bool {
    match err {
        SdkError::ServiceError(e) => e.err().is_resource_not_found_exception(),
        _ => false,
    }
}

pub(crate) fn delete_function_url_config_doesnt_exist_error(
    err: &SdkError<DeleteFunctionUrlConfigError>,
) -> bool {
    match err {
        SdkError::ServiceError(e) => e.err().is_resource_not_found_exception(),
        _ => false,
    }
}

pub(crate) fn alias_doesnt_exist_error(err: &SdkError<GetAliasError>) -> bool {
    match err {
        SdkError::ServiceError(e) => e.err().is_resource_not_found_exception(),
        _ => false,
    }
}

// There is no specific error type for this failure case, so
// we need to compare error messages and hope for the best :(
fn is_role_cannot_be_assumed_error(err: &SdkError<CreateFunctionError>) -> bool {
    err.to_string()
        == "InvalidParameterValueException: The role defined for the function cannot be assumed by Lambda."
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_credential_types::Credentials;
    use aws_sdk_s3::config::{Config as S3Config, Region, SharedCredentialsProvider};
    use aws_smithy_runtime::client::http::test_util::{ReplayEvent, StaticReplayClient};
    use aws_smithy_types::body::SdkBody;
    use base64::prelude::*;
    use cargo_lambda_metadata::lambda::Tracing;
    use cargo_lambda_remote::aws_sdk_lambda::config::Config as LambdaConfig;
    use http::{Request, Response};
    use std::io::Read;

    #[tokio::test]
    async fn test_update_function_config_no_changes() {
        // Create a mock client that fails if any requests are made
        let http_client = StaticReplayClient::new(vec![]);

        let config = LambdaConfig::builder()
            .http_client(http_client.clone())
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .build();
        let client = LambdaClient::from_conf(config);

        let config = Deploy::default();
        let name = "test-function";
        let progress = Progress::start("deploying function");

        // Create a function configuration that matches an active function that was deployed successfully
        let conf = FunctionConfiguration::builder()
            .function_arn("arn:aws:lambda:us-east-1:123456789012:function:test-function")
            .state(State::Active)
            .last_update_status(LastUpdateStatus::Successful)
            .build();

        // This should not make any requests since no config changes are needed
        let result = update_function_config(&config, name, &client, &progress, conf).await;

        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            "arn:aws:lambda:us-east-1:123456789012:function:test-function"
        );
        http_client.assert_requests_match(&[]);
    }

    #[tokio::test]
    async fn test_update_function_code_direct_upload() {
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let mut file = temp_file.as_file();
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).unwrap();
        let base64_contents = BASE64_STANDARD.encode(contents);

        let request_body = SdkBody::from(
            serde_json::json!({
                "Publish": true,
                "ZipFile": base64_contents
            })
            .to_string(),
        );

        let response_body = SdkBody::from(
            serde_json::json!({
                "FunctionArn": "arn:aws:lambda:us-east-1:123456789012:function:test-function",
                "Version": "1"
            })
            .to_string(),
        );

        let http_client = StaticReplayClient::new(vec![ReplayEvent::new(
            Request::builder()
                .uri("https://lambda.us-east-1.amazonaws.com/2015-03-31/functions/test-function/code")
                .body(request_body).unwrap(),
            Response::builder().status(200).body(response_body).unwrap(),
        )]);

        let lambda_config = LambdaConfig::builder()
            .http_client(http_client.clone())
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .build();
        let lambda_client = LambdaClient::from_conf(lambda_config);

        let s3_config = S3Config::builder()
            .http_client(http_client.clone())
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .build();
        let s3_client = S3Client::from_conf(s3_config);

        let deploy_config = Deploy::default();
        let name = "test-function";

        let binary_archive = BinaryArchive::new(
            temp_file.path().to_path_buf(),
            "x86_64".to_string(),
            BinaryModifiedAt::now(),
        );

        let progress = Progress::start("deploying function");

        let result = update_function_code(
            &deploy_config,
            name,
            &lambda_client,
            &s3_client,
            &binary_archive,
            &progress,
        )
        .await;

        let (arn, version) = result.unwrap();
        assert_eq!(
            arn.unwrap(),
            "arn:aws:lambda:us-east-1:123456789012:function:test-function"
        );
        assert_eq!(version.unwrap(), "1");
        http_client.assert_requests_match(&[]);
    }

    #[tokio::test]
    async fn test_update_function_code_with_s3() {
        // Setup mock responses
        let s3_request = Request::builder()
            .uri("https://test-bucket.s3.us-east-1.amazonaws.com/test-key?x-id=PutObject")
            .method("PUT")
            .header("x-amz-tagging", "env=test")
            .body(SdkBody::empty())
            .unwrap();
        let s3_response = Response::builder()
            .status(200)
            .body(SdkBody::from(r#"{"ETag": "test-etag"}"#))
            .unwrap();

        let lambda_request = Request::builder()
            .uri("https://lambda.us-east-1.amazonaws.com/2015-03-31/functions/test-function/code")
            .method("PUT")
            .body(SdkBody::from(
                serde_json::json!({
                    "S3Bucket": "test-bucket",
                    "S3Key": "test-key",
                    "Publish": true
                })
                .to_string(),
            ))
            .unwrap();
        let lambda_response = Response::builder()
            .status(200)
            .body(SdkBody::from(
                serde_json::json!({
                    "FunctionArn": "arn:aws:lambda:us-east-1:123456789012:function:test-function",
                    "Version": "2"
                })
                .to_string(),
            ))
            .unwrap();

        let http_client = StaticReplayClient::new(vec![
            ReplayEvent::new(s3_request, s3_response),
            ReplayEvent::new(lambda_request, lambda_response),
        ]);

        // Setup clients
        let lambda_config = LambdaConfig::builder()
            .http_client(http_client.clone())
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .build();
        let lambda_client = LambdaClient::from_conf(lambda_config);

        let s3_config = S3Config::builder()
            .http_client(http_client.clone())
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .build();
        let s3_client = S3Client::from_conf(s3_config);

        // Create test file
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let binary_archive = BinaryArchive::new(
            temp_file.path().to_path_buf(),
            "x86_64".to_string(),
            BinaryModifiedAt::now(),
        );

        // Setup deploy config with S3 bucket
        let mut deploy_config = Deploy::default();
        deploy_config.s3_bucket = Some("test-bucket".to_string());
        deploy_config.s3_key = Some("test-key".to_string());
        deploy_config.tag = Some(vec!["env=test".to_string()]);

        let progress = Progress::start("deploying function");

        let result = update_function_code(
            &deploy_config,
            "test-function",
            &lambda_client,
            &s3_client,
            &binary_archive,
            &progress,
        )
        .await;

        assert!(result.is_ok());
        let (arn, version) = result.unwrap();
        assert_eq!(
            arn.unwrap(),
            "arn:aws:lambda:us-east-1:123456789012:function:test-function"
        );
        assert_eq!(version.unwrap(), "2");

        // Verify all expected requests were made
        http_client.assert_requests_match(&[]);
    }

    #[tokio::test]
    async fn test_create_function_direct_upload() {
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let mut file = temp_file.as_file();
        let mut contents = Vec::new();
        file.read_to_end(&mut contents).unwrap();
        let base64_contents = BASE64_STANDARD.encode(contents);

        let request_body = SdkBody::from(
            serde_json::json!({
                "Code": {
                    "ZipFile": base64_contents
                },
                "FunctionName": "test-function",
                "Handler": "bootstrap",
                "Role": "arn:aws:iam::123456789012:role/test-role",
                "Runtime": "provided.al2023",
                "Architectures": ["x86_64"],
                "Publish": true,
                "Timeout": 30
            })
            .to_string(),
        );

        let response_body = SdkBody::from(
            serde_json::json!({
                "FunctionArn": "arn:aws:lambda:us-east-1:123456789012:function:test-function",
                "Version": "1",
            })
            .to_string(),
        );

        let http_client = StaticReplayClient::new(vec![ReplayEvent::new(
            Request::builder()
                .uri("https://lambda.us-east-1.amazonaws.com/2015-03-31/functions")
                .method("POST")
                .body(request_body)
                .unwrap(),
            Response::builder().status(200).body(response_body).unwrap(),
        )]);

        let lambda_config = LambdaConfig::builder()
            .http_client(http_client.clone())
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .build();
        let lambda_client = LambdaClient::from_conf(lambda_config);

        let s3_config = S3Config::builder()
            .http_client(http_client.clone())
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .build();
        let s3_client = S3Client::from_conf(s3_config);

        let deploy_config = Deploy::default();
        let name = "test-function";
        let progress = Progress::start("deploying function");
        let function_role =
            FunctionRole::from_existing("arn:aws:iam::123456789012:role/test-role".to_string());

        let binary_archive = BinaryArchive::new(
            temp_file.path().to_path_buf(),
            "x86_64".to_string(),
            BinaryModifiedAt::now(),
        );

        let result = create_function(
            &deploy_config,
            name,
            &lambda_client,
            &s3_client,
            &binary_archive,
            &progress,
            function_role,
        )
        .await;

        assert!(result.is_ok());
        let (arn, version) = result.unwrap();
        assert_eq!(
            arn.unwrap(),
            "arn:aws:lambda:us-east-1:123456789012:function:test-function"
        );
        assert_eq!(version.unwrap(), "1");
        http_client.assert_requests_match(&[]);
    }

    #[tokio::test]
    async fn test_create_function_s3_upload() {
        // Setup mock responses for S3 upload
        let s3_request = Request::builder()
            .uri("https://test-bucket.s3.us-east-1.amazonaws.com/test-key?x-id=PutObject")
            .method("PUT")
            .header("x-amz-tagging", "env=test")
            .body(SdkBody::empty())
            .unwrap();
        let s3_response = Response::builder()
            .status(200)
            .body(SdkBody::from(r#"{"ETag": "test-etag"}"#))
            .unwrap();

        // Setup mock responses for Lambda create
        let lambda_request = Request::builder()
            .uri("https://lambda.us-east-1.amazonaws.com/2015-03-31/functions")
            .method("POST")
            .body(SdkBody::from(
                serde_json::json!({
                    "Code": {
                        "S3Bucket": "test-bucket",
                        "S3Key": "test-key"
                    },
                    "FunctionName": "test-function",
                    "Handler": "bootstrap",
                    "Role": "arn:aws:iam::123456789012:role/test-role",
                    "Runtime": "provided.al2023",
                    "Architectures": ["x86_64"],
                    "Publish": true,
                    "Timeout": 30,
                    "Tags": {
                        "env": "test"
                    }
                })
                .to_string(),
            ))
            .unwrap();
        let lambda_response = Response::builder()
            .status(200)
            .body(SdkBody::from(
                serde_json::json!({
                    "FunctionArn": "arn:aws:lambda:us-east-1:123456789012:function:test-function",
                    "Version": "1"
                })
                .to_string(),
            ))
            .unwrap();

        let http_client = StaticReplayClient::new(vec![
            ReplayEvent::new(s3_request, s3_response),
            ReplayEvent::new(lambda_request, lambda_response),
        ]);

        // Setup AWS clients
        let lambda_config = LambdaConfig::builder()
            .http_client(http_client.clone())
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .build();
        let lambda_client = LambdaClient::from_conf(lambda_config);

        let s3_config = S3Config::builder()
            .http_client(http_client.clone())
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .build();
        let s3_client = S3Client::from_conf(s3_config);

        // Create test file
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let binary_archive = BinaryArchive::new(
            temp_file.path().to_path_buf(),
            "x86_64".to_string(),
            BinaryModifiedAt::now(),
        );

        // Setup deploy config with S3 bucket
        let mut deploy_config = Deploy::default();
        deploy_config.s3_bucket = Some("test-bucket".to_string());
        deploy_config.s3_key = Some("test-key".to_string());
        deploy_config.tag = Some(vec!["env=test".to_string()]);

        let name = "test-function";
        let progress = Progress::start("deploying function");
        let function_role =
            FunctionRole::from_existing("arn:aws:iam::123456789012:role/test-role".to_string());

        let result = create_function(
            &deploy_config,
            name,
            &lambda_client,
            &s3_client,
            &binary_archive,
            &progress,
            function_role,
        )
        .await;

        assert!(result.is_ok());
        let (arn, version) = result.unwrap();
        assert_eq!(
            arn.unwrap(),
            "arn:aws:lambda:us-east-1:123456789012:function:test-function"
        );
        assert_eq!(version.unwrap(), "1");
        http_client.assert_requests_match(&[]);
    }

    #[tokio::test]
    async fn test_update_function_config() {
        let request_body = SdkBody::from(
            serde_json::json!({
                "Timeout": 120,
                "TracingConfig": {
                    "Mode": "Active"
                }
            })
            .to_string(),
        );

        let response_body = SdkBody::from(
            serde_json::json!({
                "FunctionArn": "arn:aws:lambda:us-east-1:123456789012:function:test-function",
                "LastUpdateStatus": "Successful"
            })
            .to_string(),
        );

        let http_client = StaticReplayClient::new(vec![ReplayEvent::new(
            Request::builder()
                .uri("https://lambda.us-east-1.amazonaws.com/2015-03-31/functions/test-function/configuration")
                .method("PUT")
                .body(request_body)
                .unwrap(),
            Response::builder().status(200).body(response_body).unwrap(),
        )]);

        let config = LambdaConfig::builder()
            .http_client(http_client.clone())
            .credentials_provider(Credentials::for_tests())
            .region(Region::new("us-east-1"))
            .build();
        let client = LambdaClient::from_conf(config);

        let mut deploy_config = Deploy::default();
        deploy_config.function_config.timeout = Some(120.into());
        deploy_config.function_config.tracing = Some(Tracing::Active);
        let name = "test-function";
        let progress = Progress::start("deploying function");

        // Create a function configuration that matches an active function
        let conf = FunctionConfiguration::builder()
            .function_arn("arn:aws:lambda:us-east-1:123456789012:function:test-function")
            .state(State::Active)
            .last_update_status(LastUpdateStatus::Successful)
            .timeout(30)
            .build();

        let result = update_function_config(&deploy_config, name, &client, &progress, conf).await;

        assert!(result.is_ok());
        assert_eq!(
            result.unwrap(),
            "arn:aws:lambda:us-east-1:123456789012:function:test-function"
        );
        http_client.assert_requests_match(&[]);
    }

    #[tokio::test]
    async fn test_set_log_retention() {
        // Setup mock responses for creating log group and setting retention
        let create_log_group_request = Request::builder()
            .uri("https://logs.us-east-1.amazonaws.com/")
            .method("POST")
            .header("x-amz-target", "Logs_20140328.CreateLogGroup")
            .body(SdkBody::from(
                serde_json::json!({
                    "logGroupName": "/aws/lambda/test-function"
                })
                .to_string(),
            ))
            .unwrap();

        let create_log_group_response = Response::builder()
            .status(200)
            .body(SdkBody::from("{}"))
            .unwrap();

        let put_retention_request = Request::builder()
            .uri("https://logs.us-east-1.amazonaws.com/")
            .method("POST")
            .header("x-amz-target", "Logs_20140328.PutRetentionPolicy")
            .body(SdkBody::from(
                serde_json::json!({
                    "logGroupName": "/aws/lambda/test-function",
                    "retentionInDays": 14
                })
                .to_string(),
            ))
            .unwrap();

        let put_retention_response = Response::builder()
            .status(200)
            .body(SdkBody::from("{}"))
            .unwrap();

        let http_client = StaticReplayClient::new(vec![
            ReplayEvent::new(create_log_group_request, create_log_group_response),
            ReplayEvent::new(put_retention_request, put_retention_response),
        ]);

        // Setup SDK config with mock client
        let sdk_config = SdkConfig::builder()
            .credentials_provider(SharedCredentialsProvider::new(Credentials::for_tests()))
            .region(Region::new("us-east-1"))
            .http_client(http_client.clone())
            .build();

        let result = set_log_retention(&sdk_config, 14, "test-function").await;

        assert!(result.is_ok());
        http_client.assert_requests_match(&[]);
    }

    #[tokio::test]
    async fn test_set_log_retention_existing_group() {
        // Setup mock response for when log group already exists
        let create_log_group_request = Request::builder()
            .uri("https://logs.us-east-1.amazonaws.com/")
            .method("POST")
            .header("x-amz-target", "Logs_20140328.CreateLogGroup")
            .body(SdkBody::from(
                serde_json::json!({
                    "logGroupName": "/aws/lambda/test-function"
                })
                .to_string(),
            ))
            .unwrap();

        let create_log_group_response = Response::builder()
            .status(400)
            .body(SdkBody::from(
                serde_json::json!({
                    "__type": "ResourceAlreadyExistsException",
                    "message": "The specified log group already exists"
                })
                .to_string(),
            ))
            .unwrap();

        let put_retention_request = Request::builder()
            .uri("https://logs.us-east-1.amazonaws.com/")
            .method("POST")
            .header("x-amz-target", "Logs_20140328.PutRetentionPolicy")
            .body(SdkBody::from(
                serde_json::json!({
                    "logGroupName": "/aws/lambda/test-function",
                    "retentionInDays": 14
                })
                .to_string(),
            ))
            .unwrap();

        let put_retention_response = Response::builder()
            .status(200)
            .body(SdkBody::from("{}"))
            .unwrap();

        let http_client = StaticReplayClient::new(vec![
            ReplayEvent::new(create_log_group_request, create_log_group_response),
            ReplayEvent::new(put_retention_request, put_retention_response),
        ]);

        // Setup SDK config with mock client
        let sdk_config = SdkConfig::builder()
            .credentials_provider(SharedCredentialsProvider::new(Credentials::for_tests()))
            .region(Region::new("us-east-1"))
            .http_client(http_client.clone())
            .build();

        let result = set_log_retention(&sdk_config, 14, "test-function").await;

        assert!(result.is_ok());
        http_client.assert_requests_match(&[]);
    }
}
