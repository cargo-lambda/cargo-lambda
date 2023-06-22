use super::DeployResult;
use crate::{extract_tags, roles};
use aws_sdk_s3::{types::ByteStream, Client as S3Client};
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_metadata::{
    cargo::{function_deploy_metadata, DeployConfig},
    env::EnvOptions,
    lambda::{Memory, Timeout, Tracing},
};
use cargo_lambda_remote::{
    aws_sdk_config::SdkConfig,
    aws_sdk_lambda::{
        error::{
            CreateFunctionError, DeleteFunctionUrlConfigError, GetAliasError, GetFunctionError,
            GetFunctionUrlConfigError,
        },
        model::{
            Architecture, Environment, FunctionCode, FunctionConfiguration, FunctionUrlAuthType,
            LastUpdateStatus, Runtime, State, TracingConfig, VpcConfig,
        },
        output::GetFunctionOutput,
        types::{Blob, SdkError},
        Client as LambdaClient,
    },
    RemoteConfig,
};
use clap::Args;
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Serialize;
use std::path::PathBuf;
use tokio::time::{sleep, Duration};
use tracing::debug;
use uuid::Uuid;

enum FunctionAction {
    Create,
    Update(Box<GetFunctionOutput>),
}

#[derive(Args, Clone, Debug, Default)]
pub struct FunctionDeployConfig {
    /// Enable function URL for this function
    #[arg(long)]
    pub enable_function_url: bool,

    /// Disable function URL for this function
    #[arg(long)]
    pub disable_function_url: bool,

    /// Memory allocated for the function
    #[arg(long, alias = "memory-size")]
    pub memory: Option<Memory>,

    /// How long the function can be running for, in seconds
    #[arg(long)]
    pub timeout: Option<Timeout>,

    #[command(flatten)]
    pub env_options: Option<EnvOptions>,

    /// Tracing mode with X-Ray
    #[arg(long)]
    pub tracing: Option<Tracing>,

    /// IAM Role associated with the function
    #[arg(long, visible_alias = "iam-role")]
    pub role: Option<String>,

    /// Lambda Layer ARN to associate the deployed function with
    #[arg(long, visible_alias = "layer-arn")]
    pub layer: Option<Vec<String>>,

    /// Subnet IDs to associate the deployed function with a VPC
    #[arg(long, value_delimiter = ',')]
    pub subnet_ids: Option<Vec<String>>,

    /// Security Group IDs to associate the deployed function
    #[arg(long, value_delimiter = ',')]
    pub security_group_ids: Option<Vec<String>>,
}

impl FunctionDeployConfig {
    fn to_deploy_config(&self) -> DeployConfig {
        DeployConfig {
            iam_role: self.role.clone(),
            tracing: self.tracing.clone().unwrap_or_default(),
            layers: self.layer.clone(),
            subnet_ids: self.subnet_ids.clone(),
            security_group_ids: self.security_group_ids.clone(),
            ..Default::default()
        }
    }
}

#[derive(Serialize)]
pub(crate) struct DeployOutput {
    function_arn: String,
    function_url: Option<String>,
}

impl std::fmt::Display for DeployOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "🔍 function arn: {}", self.function_arn)?;
        if let Some(url) = &self.function_url {
            write!(f, "🔗 function url: {url}")?;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn deploy(
    name: &str,
    binary_name: &str,
    manifest_path: &PathBuf,
    function_config: &FunctionDeployConfig,
    remote_config: &RemoteConfig,
    sdk_config: &SdkConfig,
    s3_bucket: &Option<String>,
    tags: &Option<Vec<String>>,
    binary_data: Vec<u8>,
    architecture: Architecture,
    progress: &Progress,
) -> Result<DeployResult> {
    let client = LambdaClient::new(sdk_config);

    let (function_arn, version) = upsert_function(
        name,
        binary_name,
        manifest_path,
        &client,
        function_config,
        remote_config,
        sdk_config,
        s3_bucket,
        tags,
        binary_data,
        architecture,
        progress,
    )
    .await?;

    if let Some(alias) = &remote_config.alias {
        progress.set_message("updating alias version");

        upsert_alias(name, alias, &version, &client).await?;
    }

    let function_url = if function_config.enable_function_url {
        progress.set_message("configuring function url");

        upsert_function_url_config(name, &remote_config.alias, &client).await?
    } else {
        None
    };

    if function_config.disable_function_url {
        progress.set_message("deleting function url configuration");

        delete_function_url_config(name, &remote_config.alias, &client).await?;
    }

    Ok(DeployResult::Function(DeployOutput {
        function_arn,
        function_url,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn upsert_function(
    name: &str,
    binary_name: &str,
    manifest_path: &PathBuf,
    client: &LambdaClient,
    function_config: &FunctionDeployConfig,
    remote_config: &RemoteConfig,
    sdk_config: &SdkConfig,
    s3_bucket: &Option<String>,
    tags: &Option<Vec<String>>,
    binary_data: Vec<u8>,
    architecture: Architecture,
    progress: &Progress,
) -> Result<(String, String)> {
    let current_function = client.get_function().function_name(name).send().await;

    let (environment, deploy_metadata) =
        load_deploy_environment(manifest_path, binary_name, function_config, tags)?;

    let action = match current_function {
        Ok(fun) => FunctionAction::Update(Box::new(fun)),
        Err(no_fun) if function_doesnt_exist_error(&no_fun) => FunctionAction::Create,
        Err(no_fun) => {
            return Err(no_fun)
                .into_diagnostic()
                .wrap_err("failed to fetch lambda function")
        }
    };

    let tracing = deploy_metadata.tracing.clone();
    let tracing_config = TracingConfig::builder()
        .mode(tracing.to_string().as_str().into())
        .build();

    let lambda_tags = deploy_metadata.tags.clone();
    let s3_tags = deploy_metadata.s3_tags();

    let (arn, version) = match action {
        FunctionAction::Create => {
            let (iam_role, is_new_role) = match &deploy_metadata.iam_role {
                None => (roles::create(sdk_config, progress).await?, true),
                Some(role) => (role.clone(), false),
            };

            debug!(role_arn = ?iam_role, config = ?deploy_metadata, "creating new function");
            progress.set_message("deploying function");

            let code = match &s3_bucket {
                None => {
                    debug!("uploading zip to Lambda");
                    let blob = Blob::new(binary_data);
                    FunctionCode::builder().zip_file(blob).build()
                }
                Some(bucket) => {
                    debug!(bucket = bucket, "uploading zip to S3");
                    let client = S3Client::new(sdk_config);
                    client
                        .put_object()
                        .bucket(bucket)
                        .key(name)
                        .body(ByteStream::from(binary_data))
                        .set_tagging(s3_tags)
                        .send()
                        .await
                        .into_diagnostic()
                        .wrap_err("failed to upload function code to S3")?;
                    FunctionCode::builder()
                        .s3_bucket(bucket)
                        .s3_key(name)
                        .build()
                }
            };

            let mut output = None;
            for attempt in 2..5 {
                let memory = deploy_metadata.memory.clone().map(Into::into);
                let timeout = deploy_metadata.timeout.clone().unwrap_or_default().into();

                let result = client
                    .create_function()
                    .vpc_config(
                        VpcConfig::builder()
                            .set_security_group_ids(deploy_metadata.security_group_ids.clone())
                            .set_subnet_ids(deploy_metadata.subnet_ids.clone())
                            .build(),
                    )
                    .runtime(Runtime::Providedal2)
                    .handler("bootstrap")
                    .function_name(name)
                    .role(iam_role.clone())
                    .architectures(architecture.clone())
                    .code(code.clone())
                    .publish(true)
                    .set_memory_size(memory)
                    .timeout(timeout)
                    .tracing_config(tracing_config.clone())
                    .environment(environment.clone())
                    .set_layers(deploy_metadata.layers.clone())
                    .set_tags(lambda_tags.clone())
                    .send()
                    .await;

                match result {
                    Ok(o) => {
                        output = Some(o);
                        break;
                    }
                    Err(err)
                        if is_role_cannot_be_assumed_error(&err) && is_new_role && attempt < 5 =>
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
                            .wrap_err("failed to create new lambda function");
                    }
                };
            }

            output
                .map(|o| (o.function_arn, o.version))
                .ok_or_else(|| miette::miette!("failed to create new lambda function"))?
        }
        FunctionAction::Update(fun) => {
            progress.set_message("deploying function");

            let conf = fun
                .configuration
                .ok_or_else(|| miette::miette!("missing function configuration"))?;

            let function_arn = conf.function_arn.as_ref().expect("missing function arn");

            let mut wait_for_readiness = false;
            if conf.state.is_none() || conf.state == Some(State::Pending) {
                wait_for_readiness = true;
            }

            if let Some(status) = conf.last_update_status() {
                if status == &LastUpdateStatus::InProgress {
                    wait_for_readiness = true;
                }
            }

            if wait_for_readiness {
                wait_for_ready_state(client, name, &remote_config.alias, progress).await?;
                progress.set_message("deploying function");
            }

            let mut update_config = false;
            let mut builder = client.update_function_configuration().function_name(name);

            if deploy_metadata.use_for_update {
                if let Some(iam_role) = &deploy_metadata.iam_role {
                    builder = builder.role(iam_role);
                }

                let memory = deploy_metadata.memory.clone().map(Into::into);
                if memory.is_some() && conf.memory_size != memory {
                    update_config = true;
                    builder = builder.set_memory_size(memory);
                }

                if let Some(timeout) = deploy_metadata.timeout {
                    let timeout: i32 = timeout.into();
                    if conf.timeout.unwrap_or_default() != timeout {
                        update_config = true;
                        builder = builder.timeout(timeout);
                    }
                }

                if should_update_layers(&deploy_metadata.layers, &conf) {
                    update_config = true;
                    builder = builder.set_layers(deploy_metadata.layers);
                }

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

                if tracing_config.mode != conf.tracing_config.map(|t| t.mode).unwrap_or_default() {
                    update_config = true;
                    builder = builder.tracing_config(tracing_config);
                }
            }

            if update_config {
                debug!(config = ?builder, "updating function's configuration");
                builder
                    .send()
                    .await
                    .into_diagnostic()
                    .wrap_err("failed to update function configuration")?;

                wait_for_ready_state(client, name, &remote_config.alias, progress).await?;
                progress.set_message("deploying function");
            }

            if let Some(tags) = lambda_tags {
                if !tags.is_empty() {
                    client
                        .tag_resource()
                        .resource(function_arn)
                        .set_tags(Some(tags))
                        .send()
                        .await
                        .into_diagnostic()
                        .wrap_err("failed to tag function")?;
                }
            }

            let mut builder = client.update_function_code().function_name(name);

            match &s3_bucket {
                None => {
                    debug!("uploading zip to Lambda");
                    let blob = Blob::new(binary_data);
                    builder = builder.zip_file(blob)
                }
                Some(bucket) => {
                    debug!(bucket = bucket, "uploading zip to S3");

                    let client = S3Client::new(sdk_config);
                    let mut operation = client
                        .put_object()
                        .bucket(bucket)
                        .key(name)
                        .body(ByteStream::from(binary_data));

                    if s3_tags.is_some() {
                        operation = operation.set_tagging(s3_tags);
                    }
                    operation
                        .send()
                        .await
                        .into_diagnostic()
                        .wrap_err("failed to upload function code to S3")?;

                    builder = builder.s3_bucket(bucket).s3_key(name);
                }
            }

            let output = builder
                .publish(true)
                .send()
                .await
                .into_diagnostic()
                .wrap_err("failed to update function code")?;

            (output.function_arn, output.version)
        }
    };

    Ok((
        arn.expect("missing function ARN"),
        version.expect("missing function version"),
    ))
}

fn load_deploy_environment(
    manifest_path: &PathBuf,
    binary_name: &str,
    function_config: &FunctionDeployConfig,
    tags: &Option<Vec<String>>,
) -> Result<(Environment, DeployConfig)> {
    let base = function_deploy_metadata(manifest_path, binary_name)?
        .unwrap_or_else(|| function_config.to_deploy_config());
    let (environment, deploy_metadata) = merge_configuration(&base, function_config, tags)?;

    debug!(env = ?environment.variables(), metadata = ?deploy_metadata, "loaded function metadata for deployment");
    Ok((environment, deploy_metadata))
}

fn merge_configuration(
    base: &DeployConfig,
    function_config: &FunctionDeployConfig,
    tags: &Option<Vec<String>>,
) -> Result<(Environment, DeployConfig)> {
    let mut deploy_metadata = base.clone();

    if let Some(tags) = tags {
        deploy_metadata.append_tags(extract_tags(tags));
    }

    if let Some(tracing) = &function_config.tracing {
        if &deploy_metadata.tracing != tracing {
            deploy_metadata.tracing = tracing.clone();
        }
    }

    if function_config.role.is_some() {
        deploy_metadata.iam_role = function_config.role.clone();
    }

    if function_config.layer.is_some() {
        deploy_metadata.layers = function_config.layer.clone();
    }

    if function_config.memory.is_some() {
        deploy_metadata.memory = function_config.memory.clone();
    }

    if function_config.subnet_ids.is_some() {
        deploy_metadata.subnet_ids = function_config.subnet_ids.clone();
    }

    if function_config.security_group_ids.is_some() {
        deploy_metadata.security_group_ids = function_config.security_group_ids.clone();
    }

    if let Some(timeout) = &function_config.timeout {
        if !timeout.is_zero() {
            deploy_metadata.timeout = Some(timeout.clone())
        }
    }

    let environment = match &function_config.env_options {
        None => deploy_metadata.lambda_environment()?,
        Some(config) => {
            deploy_metadata.use_for_update = true;

            if config.env_file.is_some() {
                deploy_metadata.env_file = config.env_file.clone();
            }

            let flag_env = config.lambda_environment()?;
            deploy_metadata.extend_environment(flag_env)?
        }
    };

    if let Some(env) = &environment.variables() {
        if !env.is_empty() {
            deploy_metadata.use_for_update = true;
        }
    }

    Ok((environment, deploy_metadata))
}

async fn wait_for_ready_state(
    client: &LambdaClient,
    name: &str,
    alias: &Option<String>,
    progress: &Progress,
) -> Result<()> {
    // wait until the function state has been completely propagated
    for attempt in 2..5 {
        let backoff = attempt * attempt;
        progress.set_message(&format!(
            "the function is not ready for updates, waiting {backoff} seconds before checking for state changes"
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

        match &conf.state {
            Some(state) => match state {
                State::Active | State::Inactive | State::Failed => break,
                State::Pending => {}
                other => return Err(miette::miette!("unexpected function state: {:?}", other)),
            },
            None => return Err(miette::miette!("unknown function state")),
        }

        match &conf.last_update_status {
            Some(state) => match state {
                LastUpdateStatus::Failed | LastUpdateStatus::Successful => break,
                LastUpdateStatus::InProgress => {}
                other => {
                    return Err(miette::miette!(
                        "unexpected last update status: {:?}",
                        other
                    ))
                }
            },
            None => return Ok(()),
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
        (None, None) => false,
        (Some(_), None) => true,
        (None, Some(_)) => true,
        (Some(cl), Some(nl)) => {
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
                .wrap_err("failed to fetch alias")
        }
    };

    Ok(())
}

pub(crate) async fn upsert_function_url_config(
    name: &str,
    alias: &Option<String>,
    client: &LambdaClient,
) -> Result<Option<String>> {
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
    err.to_string() == "InvalidParameterValueException: The role defined for the function cannot be assumed by Lambda."
}

#[cfg(test)]
mod tests {
    use super::{load_deploy_environment, *};
    use std::{collections::HashMap, path::PathBuf};

    fn fixture(name: &str) -> PathBuf {
        format!("../../tests/fixtures/{name}/Cargo.toml").into()
    }

    #[test]
    fn test_load_deploy_environment() {
        let (env, config) = load_deploy_environment(
            &fixture("single-binary-package"),
            "basic-lambda",
            &Default::default(),
            &None,
        )
        .unwrap();

        let vars = env.variables().unwrap();
        assert_eq!("VAL1".to_string(), vars["VAR1"]);
        assert_eq!(Some(Memory::Mb512), config.memory);

        let mut tags = HashMap::new();
        tags.insert("organization".to_string(), "aws".to_string());
        tags.insert("team".to_string(), "lambda".to_string());
        assert_eq!(Some(tags), config.tags);

        assert_eq!(
            Some("arn:aws:lambda:us-east-1:xxxxxxxx:iam:role1".to_string()),
            config.iam_role
        );
        assert!(config.use_for_update);
    }

    #[test]
    fn test_load_deploy_environment_appending_flags() {
        let flags = FunctionDeployConfig {
            memory: Some(Memory::Mb1024),

            ..Default::default()
        };

        let tags = vec!["team=s3".to_string(), "subteam=storage".to_string()];

        let (env, config) = load_deploy_environment(
            &fixture("single-binary-package"),
            "basic-lambda",
            &flags,
            &Some(tags),
        )
        .unwrap();

        let vars = env.variables().unwrap();
        assert_eq!("VAL1".to_string(), vars["VAR1"]);
        assert_eq!(Some(Memory::Mb1024), config.memory);

        let mut tags = HashMap::new();
        tags.insert("organization".to_string(), "aws".to_string());
        tags.insert("team".to_string(), "s3".to_string());
        tags.insert("subteam".to_string(), "storage".to_string());

        assert_eq!(Some(tags), config.tags);
        assert!(config.use_for_update);
    }

    #[test]
    fn test_load_deploy_environment_empty_config() {
        let flags = FunctionDeployConfig {
            role: Some("role-arn".into()),
            ..Default::default()
        };

        let tags = vec!["team=s3".to_string(), "subteam=storage".to_string()];

        let (_env, config) = load_deploy_environment(
            &fixture("multi-binary-package"),
            "basic-lambda",
            &flags,
            &Some(tags),
        )
        .unwrap();

        assert_eq!(Some("role-arn".into()), config.iam_role);

        let mut tags = HashMap::new();
        tags.insert("team".to_string(), "s3".to_string());
        tags.insert("subteam".to_string(), "storage".to_string());

        assert_eq!(Some(tags), config.tags);
        assert!(!config.use_for_update);
    }

    #[test]
    fn test_load_deploy_environment_empty_config_with_env_flags() {
        let flags = FunctionDeployConfig {
            role: Some("role-arn".into()),
            env_options: Some(EnvOptions {
                env_var: Some(vec!["FOO=BAR".into()]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let (env, config) = load_deploy_environment(
            &fixture("multi-binary-package"),
            "basic-lambda",
            &flags,
            &None,
        )
        .unwrap();

        assert_eq!(Some("role-arn".into()), config.iam_role);
        assert_eq!("BAR", env.variables().unwrap()["FOO"]);
        assert!(config.use_for_update);
    }
}
