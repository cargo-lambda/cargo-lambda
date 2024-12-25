use crate::roles;
use aws_sdk_s3::{primitives::ByteStream, Client as S3Client};
use cargo_lambda_build::{BinaryArchive, BinaryModifiedAt};
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_metadata::cargo::deploy::Deploy;
use cargo_lambda_remote::{
    aws_sdk_config::SdkConfig,
    aws_sdk_lambda::{
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
            Architecture, Environment, FunctionCode, FunctionConfiguration, FunctionUrlAuthType,
            LastUpdateStatus, Runtime, State, TracingConfig, VpcConfig as LambdaVpcConfig,
        },
        Client as LambdaClient,
    },
};
use miette::{IntoDiagnostic, Result, WrapErr};
use serde::Serialize;
use std::{collections::HashMap, str::FromStr};
use tokio::time::{sleep, Duration};
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
}

impl std::fmt::Display for DeployOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "✅ function deployed successfully 🎉")?;
        writeln!(
            f,
            "🛠️  binary last compiled {}",
            self.binary_modified_at.humanize()
        )?;
        write!(f, "🔍 function arn: {}", self.function_arn)?;
        if let Some(url) = &self.function_url {
            write!(f, "🔗 function url: {url}")?;
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn deploy(
    config: &Deploy,
    base_env: &HashMap<String, String>,
    name: &str,
    sdk_config: &SdkConfig,
    binary_archive: &BinaryArchive,
    architecture: Architecture,
    progress: &Progress,
) -> Result<DeployOutput> {
    let client = LambdaClient::new(sdk_config);

    let (function_arn, version) = upsert_function(
        config,
        base_env,
        name,
        &client,
        sdk_config,
        binary_archive,
        architecture,
        progress,
    )
    .await?;

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

    Ok(DeployOutput {
        function_arn,
        function_url,
        binary_modified_at: binary_archive.binary_modified_at.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn upsert_function(
    config: &Deploy,
    base_env: &HashMap<String, String>,
    name: &str,
    client: &LambdaClient,
    sdk_config: &SdkConfig,
    binary_archive: &BinaryArchive,
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

    let tracing = config.function_config.tracing.clone().unwrap_or_default();
    let tracing_config = TracingConfig::builder()
        .mode(tracing.to_string().as_str().into())
        .build();

    let environment = config.environment(base_env)?;
    let environment = Environment::builder()
        .set_variables(Some(environment))
        .build();
    let lambda_tags = config.lambda_tags();
    let s3_tags = config.s3_tags();

    let (arn, version) = match action {
        FunctionAction::Create => {
            let (iam_role, is_new_role) = match &config.function_config.role {
                None => (roles::create(sdk_config, progress).await?, true),
                Some(role) => (role.clone(), false),
            };

            debug!(role_arn = ?iam_role, ?config, "creating new function");
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
                    let client = S3Client::new(sdk_config);
                    client
                        .put_object()
                        .bucket(bucket)
                        .key(key)
                        .body(ByteStream::from(binary_archive.read()?))
                        .set_tagging(s3_tags)
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

                let mut function = client.create_function();
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
                    .role(iam_role.clone())
                    .architectures(architecture.clone())
                    .code(code.clone())
                    .publish(true)
                    .set_memory_size(memory)
                    .timeout(timeout)
                    .tracing_config(tracing_config.clone())
                    .environment(environment.clone())
                    .set_layers(config.function_config.layer.clone())
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

                if let Some(vpc) = &config.function_config.vpc {
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

            if update_config {
                debug!(config = ?builder, "updating function's configuration");
                builder
                    .send()
                    .await
                    .into_diagnostic()
                    .wrap_err("failed to update function configuration")?;

                wait_for_ready_state(client, name, &config.remote_config.alias, progress).await?;
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

            match &config.s3_bucket {
                None => {
                    debug!("uploading zip to Lambda");
                    let blob = Blob::new(binary_archive.read()?);
                    builder = builder.zip_file(blob)
                }
                Some(bucket) => {
                    let key = config.s3_key.as_deref().unwrap_or(name);

                    debug!(bucket, key, "uploading zip to S3");

                    let client = S3Client::new(sdk_config);
                    let mut operation = client
                        .put_object()
                        .bucket(bucket)
                        .key(key)
                        .body(ByteStream::from(binary_archive.read()?));

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
                .wrap_err("failed to fetch alias")
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
