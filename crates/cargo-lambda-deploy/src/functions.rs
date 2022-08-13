use super::DeployResult;
use aws_sdk_iam::Client as IamClient;
use aws_sdk_s3::{types::ByteStream, Client as S3Client};
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_remote::{
    aws_sdk_config::SdkConfig,
    aws_sdk_lambda::{
        error::{
            CreateFunctionError, DeleteFunctionUrlConfigError, GetAliasError, GetFunctionError,
            GetFunctionUrlConfigError,
        },
        model::{
            Architecture, Environment, FunctionCode, FunctionConfiguration, FunctionUrlAuthType,
            Runtime, State, TracingConfig,
        },
        output::GetFunctionOutput,
        types::{Blob, SdkError},
        Client as LambdaClient,
    },
    RemoteConfig,
};
use clap::{Args, ValueHint};
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Serialize;
use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::PathBuf,
};
use strum_macros::{Display, EnumString};
use tokio::time::{sleep, Duration};

const BASIC_LAMBDA_EXECUTION_POLICY: &str =
    "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole";
const ASSUME_ROLE_TRUST_POLICY: &str = r#"{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Principal": {
        "Service": "lambda.amazonaws.com"
      },
      "Action": "sts:AssumeRole"
    }
  ]
}"#;

enum FunctionAction {
    Create,
    Update(Box<GetFunctionOutput>),
}

#[derive(Clone, Debug, Display, EnumString)]
#[strum(ascii_case_insensitive)]
pub enum Tracing {
    Active,
    Passthrough,
}

#[derive(Args, Clone, Debug)]
pub struct FunctionDeployConfig {
    /// Memory allocated for the function
    #[clap(long)]
    pub memory_size: Option<i32>,

    /// Enable function URL for this function
    #[clap(long)]
    pub enable_function_url: bool,

    /// Disable function URL for this function
    #[clap(long)]
    pub disable_function_url: bool,

    /// How long the function can be running for, in seconds
    #[clap(long, default_value = "30")]
    pub timeout: i32,

    /// Option to add one or many environment variables, allows multiple repetitions
    /// Use VAR_KEY=VAR_VALUE as format
    #[clap(long)]
    pub env_var: Option<Vec<String>>,

    /// Read environment variables from a file
    #[clap(long, value_hint = ValueHint::FilePath)]
    pub env_file: Option<PathBuf>,

    /// Tracing mode with X-Ray
    #[clap(long, default_value_t = Tracing::Active)]
    pub tracing: Tracing,

    /// IAM Role associated with the function
    #[clap(long)]
    pub iam_role: Option<String>,

    /// Lambda Layer ARN to associate the deployed function with
    #[clap(long)]
    pub layer_arn: Option<Vec<String>>,

    #[clap(long, hide = true, default_value = "10")]
    wait_role: u64,
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
            write!(f, "🔗 function url: {}", url)?;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn deploy(
    name: &str,
    function_config: &FunctionDeployConfig,
    remote_config: &RemoteConfig,
    sdk_config: &SdkConfig,
    s3_bucket: &Option<String>,
    binary_data: Vec<u8>,
    architecture: Architecture,
    progress: &Progress,
) -> Result<DeployResult> {
    let client = LambdaClient::new(sdk_config);

    let (function_arn, version) = upsert_function(
        name,
        &client,
        function_config,
        remote_config,
        sdk_config,
        s3_bucket,
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
    client: &LambdaClient,
    function_config: &FunctionDeployConfig,
    remote_config: &RemoteConfig,
    sdk_config: &SdkConfig,
    s3_bucket: &Option<String>,
    binary_data: Vec<u8>,
    architecture: Architecture,
    progress: &Progress,
) -> Result<(String, String)> {
    let current_function = client.get_function().function_name(name).send().await;

    let action = match current_function {
        Ok(fun) => FunctionAction::Update(Box::new(fun)),
        Err(no_fun) if function_doesnt_exist_error(&no_fun) => FunctionAction::Create,
        Err(no_fun) => {
            return Err(no_fun)
                .into_diagnostic()
                .wrap_err("failed to fetch lambda function")
        }
    };

    let tracing_config = TracingConfig::builder()
        .mode(function_config.tracing.to_string().as_str().into())
        .build();

    let (arn, version) = match action {
        FunctionAction::Create => {
            let wait_role = Duration::from_secs(function_config.wait_role);

            let (iam_role, is_new_role) = match &function_config.iam_role {
                None => (
                    create_lambda_role(sdk_config, wait_role, progress).await?,
                    true,
                ),
                Some(role) => (role.clone(), false),
            };

            tracing::debug!(role_arn = ?iam_role, "using iam role");
            progress.set_message("deploying function");

            let code = match &s3_bucket {
                None => {
                    let blob = Blob::new(binary_data);
                    FunctionCode::builder().zip_file(blob).build()
                }
                Some(bucket) => {
                    let client = S3Client::new(sdk_config);
                    client
                        .put_object()
                        .bucket(bucket)
                        .key(name)
                        .body(ByteStream::from(binary_data))
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
                let result = client
                    .create_function()
                    .runtime(Runtime::Providedal2)
                    .handler("bootstrap")
                    .function_name(name)
                    .role(iam_role.clone())
                    .architectures(architecture.clone())
                    .code(code.clone())
                    .publish(true)
                    .set_memory_size(function_config.memory_size)
                    .timeout(function_config.timeout)
                    .tracing_config(tracing_config.clone())
                    .environment(function_environment(
                        &function_config.env_file,
                        &function_config.env_var,
                    )?)
                    .set_layers(function_config.layer_arn.clone())
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
                            "new role not full propagated, waiting {} seconds before retrying",
                            backoff
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

            if let Some(conf) = fun.configuration {
                let mut update_config = false;
                let mut builder = client.update_function_configuration().function_name(name);

                if let Some(iam_role) = &function_config.iam_role {
                    builder = builder.role(iam_role);
                }

                if function_config.memory_size.is_some()
                    && conf.memory_size != function_config.memory_size
                {
                    update_config = true;
                    builder = builder.set_memory_size(function_config.memory_size);
                }

                if conf.timeout.unwrap_or_default() != function_config.timeout {
                    update_config = true;
                    builder = builder.timeout(function_config.timeout);
                }

                if should_update_layers(&function_config.layer_arn, &conf) {
                    update_config = true;
                    builder = builder.set_layers(function_config.layer_arn.clone());
                }

                let env =
                    function_environment(&function_config.env_file, &function_config.env_var)?;
                if env.variables != conf.environment.map(|e| e.variables).unwrap_or_default() {
                    update_config = true;
                    builder = builder.environment(env);
                }

                if tracing_config.mode != conf.tracing_config.map(|t| t.mode).unwrap_or_default() {
                    update_config = true;
                    builder = builder.tracing_config(tracing_config);
                }

                if update_config {
                    builder
                        .send()
                        .await
                        .into_diagnostic()
                        .wrap_err("failed to update function configuration")?;

                    // wait until the update has been conpletely propagated
                    for attempt in 1..4 {
                        let conf = client
                            .get_function_configuration()
                            .function_name(name)
                            .set_qualifier(remote_config.alias.clone())
                            .send()
                            .await
                            .into_diagnostic()
                            .wrap_err("failed to fetch the function configuration")?;

                        match &conf.state {
                            Some(state) => match state {
                                State::Active => break,
                                State::Pending => {
                                    sleep(Duration::from_secs(attempt * attempt)).await;
                                }
                                other => {
                                    return Err(miette::miette!(
                                        "unexpected function state: {:?}",
                                        other
                                    ))
                                }
                            },
                            None => return Err(miette::miette!("unknown function state")),
                        }

                        if attempt == 3 {
                            return Err(miette::miette!("configuration update didn't finish in time, wait a few minutes and try again"));
                        }
                    }
                }
            }

            let mut builder = client.update_function_code().function_name(name);

            match &s3_bucket {
                None => {
                    let blob = Blob::new(binary_data);
                    builder = builder.zip_file(blob)
                }
                Some(bucket) => {
                    let client = S3Client::new(sdk_config);
                    client
                        .put_object()
                        .bucket(bucket)
                        .key(name)
                        .body(ByteStream::from(binary_data))
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
            client
                .add_permission()
                .function_name(name)
                .set_qualifier(alias.clone())
                .action("lambda:InvokeFunctionUrl")
                .principal("*")
                .statement_id("FunctionUrlAllowPublicAccess")
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

pub(crate) fn function_environment(
    env_file: &Option<PathBuf>,
    env_vars: &Option<Vec<String>>,
) -> Result<Environment> {
    let mut env = Environment::builder();
    if let Some(path) = env_file {
        let file = File::open(path)
            .into_diagnostic()
            .wrap_err("failed to open env file: {path}")?;
        let reader = BufReader::new(file);

        for line in reader.lines() {
            let line = line.into_diagnostic().wrap_err("failed to read env line")?;

            let mut iter = line.trim().splitn(2, '=');
            let key = iter
                .next()
                .ok_or_else(|| miette::miette!("invalid env variable {var}"))?;
            let value = iter
                .next()
                .ok_or_else(|| miette::miette!("invalid env variable {var}"))?;
            env = env.variables(key, value);
        }
    }

    if let Some(vars) = env_vars {
        for var in vars {
            let mut iter = var.trim().splitn(2, '=');
            let key = iter
                .next()
                .ok_or_else(|| miette::miette!("invalid env variable {var}"))?;
            let value = iter
                .next()
                .ok_or_else(|| miette::miette!("invalid env variable {var}"))?;
            env = env.variables(key, value);
        }
    }

    Ok(env.build())
}

pub(crate) fn function_doesnt_exist_error(err: &SdkError<GetFunctionError>) -> bool {
    match err {
        SdkError::ServiceError { err, .. } => err.is_resource_not_found_exception(),
        _ => false,
    }
}

pub(crate) fn function_url_config_doesnt_exist_error(
    err: &SdkError<GetFunctionUrlConfigError>,
) -> bool {
    match err {
        SdkError::ServiceError { err, .. } => err.is_resource_not_found_exception(),
        _ => false,
    }
}

pub(crate) fn delete_function_url_config_doesnt_exist_error(
    err: &SdkError<DeleteFunctionUrlConfigError>,
) -> bool {
    match err {
        SdkError::ServiceError { err, .. } => err.is_resource_not_found_exception(),
        _ => false,
    }
}

pub(crate) fn alias_doesnt_exist_error(err: &SdkError<GetAliasError>) -> bool {
    match err {
        SdkError::ServiceError { err, .. } => err.is_resource_not_found_exception(),
        _ => false,
    }
}

// There is specific error type for this failure case, so
// we need to compare error messages and hope for the best :(
fn is_role_cannot_be_assumed_error(err: &SdkError<CreateFunctionError>) -> bool {
    err.to_string() == "InvalidParameterValueException: The role defined for the function cannot be assumed by Lambda."
}

async fn create_lambda_role(
    config: &SdkConfig,
    wait_role: Duration,
    progress: &Progress,
) -> Result<String> {
    progress.set_message("creating execution role");

    let role_name = format!("cargo-lambda-role-{}", uuid::Uuid::new_v4());
    let client = IamClient::new(config);
    let role = client
        .create_role()
        .role_name(&role_name)
        .assume_role_policy_document(ASSUME_ROLE_TRUST_POLICY)
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to create function role")?
        .role
        .expect("missing role information");

    client
        .attach_role_policy()
        .role_name(&role_name)
        .policy_arn(BASIC_LAMBDA_EXECUTION_POLICY)
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to attach policy AWSLambdaBasicExecutionRole to function role")?;

    progress.set_message(&format!(
        "waiting {} seconds until the role propagates",
        wait_role.as_secs()
    ));
    sleep(wait_role).await;

    tracing::debug!(role = ?role, "function role created");

    role.arn()
        .map(String::from)
        .ok_or_else(|| miette::miette!("missing role arn"))
}
