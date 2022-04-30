use cargo_lambda_build::{find_function_archive, BinaryArchive};
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_metadata::cargo::root_package;
use cargo_lambda_remote::{
    aws_sdk_lambda::{
        error::{
            DeleteFunctionUrlConfigError, GetAliasError, GetFunctionError,
            GetFunctionUrlConfigError,
        },
        model::{
            Architecture, Environment, FunctionCode, FunctionUrlAuthType, Runtime, State,
            TracingConfig,
        },
        output::GetFunctionOutput,
        types::{Blob, SdkError},
        Client,
    },
    init_client, RemoteConfig,
};
use clap::{Args, ValueHint};
use miette::{IntoDiagnostic, Result, WrapErr};
use std::{
    fs::{read, File},
    io::{BufRead, BufReader},
    path::PathBuf,
};
use strum_macros::{Display, EnumString};
use tokio::time::{sleep, Duration};

enum FunctionAction {
    Create,
    Update(Box<GetFunctionOutput>),
}

#[derive(Args, Clone, Debug)]
#[clap(name = "deploy")]
pub struct Deploy {
    #[clap(flatten)]
    config: RemoteConfig,

    /// Memory allocated for the function
    #[clap(long)]
    memory_size: Option<i32>,

    /// Enable function URL for this function
    #[clap(long)]
    enable_function_url: bool,

    /// Disable function URL for this function
    #[clap(long)]
    disable_function_url: bool,

    /// How long the function can be running for, in seconds
    #[clap(long, default_value = "30")]
    timeout: i32,

    /// Directory where the lambda binaries are located
    #[clap(short, long, value_hint = ValueHint::DirPath)]
    lambda_dir: Option<PathBuf>,

    /// Path to Cargo.toml
    #[clap(
        long,
        value_name = "PATH",
        parse(from_os_str),
        default_value = "Cargo.toml"
    )]
    pub manifest_path: PathBuf,

    /// Name of the binary to deploy if it doesn't match the name that you want to deploy it with
    #[clap(long)]
    pub binary_name: Option<String>,

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

    /// Name of the binary to deploy
    #[clap(value_name = "FUNCTION_NAME")]
    function_name: Option<String>,
}

#[derive(Clone, Debug, Display, EnumString)]
#[strum(ascii_case_insensitive)]
pub enum Tracing {
    Active,
    Passthrough,
}

impl Deploy {
    pub async fn run(&self) -> Result<()> {
        if self.enable_function_url && self.disable_function_url {
            return Err(miette::miette!("invalid options: --enable-function-url and --disable-function-url cannot be set together"));
        }

        let name = match &self.function_name {
            Some(name) => name.clone(),
            None => root_package(&self.manifest_path)?.name,
        };
        let binary_name = self.binary_name.as_deref().unwrap_or(&name);

        let client = init_client(&self.config).await;

        let progress = Progress::start("deploying function");

        let archive = match find_function_archive(binary_name, &self.lambda_dir) {
            Ok(arc) => arc,
            Err(err) => {
                progress.finish_and_clear();
                return Err(err);
            }
        };

        let version = match self.upsert_function(&client, &name, &archive).await {
            Ok(version) => version,
            Err(err) => {
                progress.finish_and_clear();
                return Err(err);
            }
        };

        if let Some(alias) = &self.config.alias {
            progress.set_message("updating alias version");

            if let Err(err) = upsert_alias(&client, &name, alias, &version).await {
                progress.finish_and_clear();
                return Err(err);
            }
        }

        let url = if self.enable_function_url {
            progress.set_message("configuring function url");

            match self.upsert_function_url_config(&client, &name).await {
                Ok(url) => url,
                Err(err) => {
                    progress.finish_and_clear();
                    return Err(err);
                }
            }
        } else {
            None
        };

        if self.disable_function_url {
            progress.set_message("deleting function url configuration");

            if let Err(err) = self.delete_function_url_config(&client, &name).await {
                progress.finish_and_clear();
                return Err(err);
            }
        }

        progress.finish("done");
        if let Some(url) = url {
            println!("🔗 function url: {}", url);
        }

        Ok(())
    }

    async fn upsert_function(
        &self,
        client: &Client,
        name: &str,
        archive: &BinaryArchive,
    ) -> Result<String> {
        let binary_data = read(&archive.path)
            .into_diagnostic()
            .wrap_err("failed to read binary archive")?;
        let blob = Blob::new(binary_data);

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
            .mode(self.tracing.to_string().as_str().into())
            .build();

        let iam_role = self.iam_role.clone();

        let version = match action {
            FunctionAction::Create => {
                let code = FunctionCode::builder().zip_file(blob).build();

                let output = client
                    .create_function()
                    .runtime(Runtime::Providedal2)
                    .handler("bootstrap")
                    .function_name(name)
                    .set_role(iam_role)
                    .architectures(Architecture::from(archive.architecture.as_str()))
                    .code(code)
                    .publish(true)
                    .set_memory_size(self.memory_size)
                    .timeout(self.timeout)
                    .tracing_config(tracing_config)
                    .environment(self.function_environment()?)
                    .send()
                    .await
                    .into_diagnostic()
                    .wrap_err("failed to create new lambda function")?;

                output.version
            }
            FunctionAction::Update(fun) => {
                if let Some(conf) = fun.configuration {
                    let mut builder = client
                        .update_function_configuration()
                        .function_name(name)
                        .set_role(iam_role);

                    let mut update_config = false;
                    if self.memory_size.is_some() && conf.memory_size != self.memory_size {
                        update_config = true;
                        builder = builder.set_memory_size(self.memory_size);
                    }
                    if conf.timeout.unwrap_or_default() != self.timeout {
                        update_config = true;
                        builder = builder.timeout(self.timeout);
                    }

                    let env = self.function_environment()?;
                    if env.variables != conf.environment.map(|e| e.variables).unwrap_or_default() {
                        update_config = true;
                        builder = builder.environment(env);
                    }

                    if tracing_config.mode
                        != conf.tracing_config.map(|t| t.mode).unwrap_or_default()
                    {
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
                                .set_qualifier(self.config.alias.clone())
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

                let output = client
                    .update_function_code()
                    .function_name(name)
                    .zip_file(blob)
                    .publish(true)
                    .send()
                    .await
                    .into_diagnostic()
                    .wrap_err("failed to update function code")?;

                output.version
            }
        };

        Ok(version.unwrap_or_default())
    }

    async fn upsert_function_url_config(
        &self,
        client: &Client,
        name: &str,
    ) -> Result<Option<String>> {
        let result = client
            .get_function_url_config()
            .function_name(name)
            .set_qualifier(self.config.alias.clone())
            .send()
            .await;

        let url = match result {
            Ok(fun) => fun.function_url,
            Err(no_fun) if function_url_config_doesnt_exist_error(&no_fun) => {
                client
                    .add_permission()
                    .function_name(name)
                    .set_qualifier(self.config.alias.clone())
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
                    .set_qualifier(self.config.alias.clone())
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

    async fn delete_function_url_config(&self, client: &Client, name: &str) -> Result<()> {
        let result = client
            .delete_function_url_config()
            .function_name(name)
            .set_qualifier(self.config.alias.clone())
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

    fn function_environment(&self) -> Result<Environment> {
        let mut env = Environment::builder();
        if let Some(path) = &self.env_file {
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

        if let Some(vars) = &self.env_var {
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
}

async fn upsert_alias(client: &Client, name: &str, alias: &str, version: &str) -> Result<()> {
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

fn function_doesnt_exist_error(err: &SdkError<GetFunctionError>) -> bool {
    match err {
        SdkError::ServiceError { err, .. } => err.is_resource_not_found_exception(),
        _ => false,
    }
}

fn function_url_config_doesnt_exist_error(err: &SdkError<GetFunctionUrlConfigError>) -> bool {
    match err {
        SdkError::ServiceError { err, .. } => err.is_resource_not_found_exception(),
        _ => false,
    }
}

fn delete_function_url_config_doesnt_exist_error(
    err: &SdkError<DeleteFunctionUrlConfigError>,
) -> bool {
    match err {
        SdkError::ServiceError { err, .. } => err.is_resource_not_found_exception(),
        _ => false,
    }
}

fn alias_doesnt_exist_error(err: &SdkError<GetAliasError>) -> bool {
    match err {
        SdkError::ServiceError { err, .. } => err.is_resource_not_found_exception(),
        _ => false,
    }
}
