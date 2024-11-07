use base64::{engine::general_purpose as b64, Engine as _};
use cargo_lambda_remote::{
    aws_sdk_lambda::{primitives::Blob, Client as LambdaClient},
    RemoteConfig,
};
use clap::{Args, ValueHint};
use miette::{IntoDiagnostic, Result, WrapErr};
use reqwest::{Client, StatusCode};
use serde::Serialize;
use serde_json::{from_str, to_string_pretty, value::Value};
use std::{
    convert::TryFrom,
    fs::{create_dir_all, read_to_string, File},
    io::copy,
    net::IpAddr,
    path::PathBuf,
    str::{from_utf8, FromStr},
};
use strum_macros::{Display, EnumString};
use tracing::debug;

mod error;
use error::*;

/// Name for the function when no name is provided.
/// This will make the watch command to compile
/// the binary without the `--bin` option, and will
/// assume that the package only has one function,
/// which is the main binary for that package.
pub const DEFAULT_PACKAGE_FUNCTION: &str = "_";
const EXAMPLES_URL: &str = "https://event-examples.cargo-lambda.info";

const LAMBDA_RUNTIME_CLIENT_CONTEXT: &str = "lambda-runtime-client-context";
const LAMBDA_RUNTIME_COGNITO_IDENTITY: &str = "lambda-runtime-cognito-identity";

#[derive(Args, Clone, Debug)]
#[command(
    name = "invoke",
    after_help = "Full command documentation: https://www.cargo-lambda.info/commands/invoke.html"
)]
pub struct Invoke {
    #[cfg_attr(
        target_os = "windows",
        arg(short = 'a', long, default_value = "127.0.0.1")
    )]
    #[cfg_attr(
        not(target_os = "windows"),
        arg(short = 'a', long, default_value = "::1")
    )]
    /// Local address host (IPv4 or IPv6) to send invoke requests
    invoke_address: String,

    /// Local port to send invoke requests
    #[arg(short = 'p', long, default_value = "9000")]
    invoke_port: u16,

    /// File to read the invoke payload from
    #[arg(short = 'F', long, value_hint = ValueHint::FilePath)]
    data_file: Option<PathBuf>,

    /// Invoke payload as a string
    #[arg(short = 'A', long)]
    data_ascii: Option<String>,

    /// Example payload from AWS Lambda Events
    #[arg(short = 'E', long)]
    data_example: Option<String>,

    /// Invoke the function already deployed on AWS Lambda
    #[arg(short = 'R', long)]
    remote: bool,

    #[command(flatten)]
    remote_config: RemoteConfig,

    /// JSON string representing the client context for the function invocation
    #[arg(long)]
    client_context_ascii: Option<String>,

    /// Path to a file with the JSON representation of the client context for the function invocation
    #[arg(long)]
    client_context_file: Option<PathBuf>,

    /// Format to render the output (text, or json)
    #[arg(short, long, default_value_t = OutputFormat::Text)]
    output_format: OutputFormat,

    #[command(flatten)]
    cognito: Option<CognitoIdentity>,

    /// Ignore data stored in the local cache
    #[arg(long, default_value_t = false)]
    skip_cache: bool,

    /// Name of the function to invoke
    #[arg(default_value = DEFAULT_PACKAGE_FUNCTION)]
    function_name: String,
}

#[derive(Clone, Debug, Display, EnumString)]
#[strum(ascii_case_insensitive)]
enum OutputFormat {
    Text,
    Json,
}

#[derive(Args, Clone, Debug, Serialize)]
pub struct CognitoIdentity {
    /// The unique identity id for the Cognito credentials invoking the function.
    #[arg(long, requires = "identity-pool-id")]
    #[serde(rename = "cognitoIdentityId")]
    pub identity_id: Option<String>,
    /// The identity pool id the caller is "registered" with.
    #[arg(long, requires = "identity-id")]
    #[serde(rename = "cognitoIdentityPoolId")]
    pub identity_pool_id: Option<String>,
}

impl CognitoIdentity {
    fn is_valid(&self) -> bool {
        self.identity_id.is_some() && self.identity_pool_id.is_some()
    }
}

impl Invoke {
    #[tracing::instrument(skip(self), target = "cargo_lambda")]
    pub async fn run(&self) -> Result<()> {
        tracing::trace!(options = ?self, "invoking function");

        let data = if let Some(file) = &self.data_file {
            read_to_string(file)
                .into_diagnostic()
                .wrap_err("error reading data file")?
        } else if let Some(data) = &self.data_ascii {
            data.clone()
        } else if let Some(example) = &self.data_example {
            let name = example_name(example);

            let cache = dirs::cache_dir()
                .map(|p| p.join("cargo-lambda").join("invoke-fixtures").join(&name));

            match cache {
                Some(cache) if !self.skip_cache && cache.exists() => {
                    tracing::debug!(?cache, "using example from cache");
                    read_to_string(cache)
                        .into_diagnostic()
                        .wrap_err("error reading data file")?
                }
                _ if self.skip_cache => download_example(&name, None).await?,
                _ => download_example(&name, cache).await?,
            }
        } else {
            return Err(InvokeError::MissingPayload.into());
        };

        let text = if self.remote {
            self.invoke_remote(&data).await?
        } else {
            self.invoke_local(&data).await?
        };

        let text = match &self.output_format {
            OutputFormat::Text => text,
            OutputFormat::Json => {
                let obj: Value = from_str(&text)
                    .into_diagnostic()
                    .wrap_err("failed to serialize response into json")?;

                to_string_pretty(&obj)
                    .into_diagnostic()
                    .wrap_err("failed to format json output")?
            }
        };

        println!("{text}");

        Ok(())
    }

    async fn invoke_remote(&self, data: &str) -> Result<String> {
        if self.function_name == DEFAULT_PACKAGE_FUNCTION {
            return Err(InvokeError::InvalidFunctionName.into());
        }

        let client_context = self.client_context(true)?;

        let sdk_config = self.remote_config.sdk_config(None).await;
        let client = LambdaClient::new(&sdk_config);

        let resp = client
            .invoke()
            .function_name(&self.function_name)
            .set_qualifier(self.remote_config.alias.clone())
            .payload(Blob::new(data.as_bytes()))
            .set_client_context(client_context)
            .send()
            .await
            .into_diagnostic()
            .wrap_err("failed to invoke remote function")?;

        if let Some(payload) = resp.payload {
            let blob = payload.into_inner();
            let data = from_utf8(&blob)
                .into_diagnostic()
                .wrap_err("failed to read response payload")?;

            if resp.function_error.is_some() {
                let err = RemoteInvokeError::try_from(data)?;
                Err(err.into())
            } else {
                Ok(data.into())
            }
        } else {
            Ok("OK".into())
        }
    }

    async fn invoke_local(&self, data: &str) -> Result<String> {
        let host = parse_invoke_ip_address(&self.invoke_address)?;

        let url = format!(
            "http://{}:{}/2015-03-31/functions/{}/invocations",
            &host, self.invoke_port, &self.function_name
        );

        let client = Client::new();
        let mut req = client.post(url).body(data.to_string());
        if let Some(identity) = &self.cognito {
            if identity.is_valid() {
                let ser = serde_json::to_string(&identity)
                    .into_diagnostic()
                    .wrap_err("failed to serialize Cognito's identity information")?;
                req = req.header(LAMBDA_RUNTIME_COGNITO_IDENTITY, ser);
            }
        }
        if let Some(client_context) = self.client_context(false)? {
            req = req.header(LAMBDA_RUNTIME_CLIENT_CONTEXT, client_context);
        }

        let resp = req
            .send()
            .await
            .into_diagnostic()
            .wrap_err("error sending request to the runtime emulator")?;
        let success = resp.status() == StatusCode::OK;

        let payload = resp
            .text()
            .await
            .into_diagnostic()
            .wrap_err("error reading response body")?;

        if success {
            Ok(payload)
        } else {
            debug!(error = ?payload, "error received from server");
            let err = RemoteInvokeError::try_from(payload.as_str())?;
            Err(err.into())
        }
    }

    fn client_context(&self, encode: bool) -> Result<Option<String>> {
        let mut data = if let Some(file) = &self.client_context_file {
            read_to_string(file)
                .into_diagnostic()
                .wrap_err("error reading client context file")?
        } else if let Some(data) = &self.client_context_ascii {
            data.clone()
        } else {
            return Ok(None);
        };

        if encode {
            data = b64::STANDARD.encode(data)
        }

        Ok(Some(data))
    }
}

fn example_name(example: &str) -> String {
    let mut name = if example.starts_with("example-") {
        example.to_string()
    } else {
        format!("example-{example}")
    };
    if !name.ends_with(".json") {
        name.push_str(".json");
    }
    name
}

async fn download_example(name: &str, cache: Option<PathBuf>) -> Result<String> {
    let target = format!("{EXAMPLES_URL}/{name}");

    tracing::debug!(?target, "downloading remote example");
    let response = reqwest::get(&target)
        .await
        .into_diagnostic()
        .wrap_err("error dowloading example data")?;

    if response.status() != StatusCode::OK {
        Err(InvokeError::ExampleDownloadFailed(target, response).into())
    } else {
        let content = response
            .text()
            .await
            .into_diagnostic()
            .wrap_err("error reading example data")?;

        if let Some(cache) = cache {
            tracing::debug!(?cache, "storing example in cache");
            create_dir_all(cache.parent().unwrap()).into_diagnostic()?;
            let mut dest = File::create(cache).into_diagnostic()?;
            copy(&mut content.as_bytes(), &mut dest).into_diagnostic()?;
        }
        Ok(content)
    }
}

fn parse_invoke_ip_address(address: &str) -> Result<String> {
    let invoke_address = IpAddr::from_str(address).map_err(|e| miette::miette!(e))?;

    let invoke_address = match invoke_address {
        IpAddr::V4(address) => address.to_string(),
        IpAddr::V6(address) => format!("[{address}]"),
    };

    Ok(invoke_address)
}

#[cfg(test)]
mod test {
    use super::*;

    #[tokio::test]
    async fn test_download_example() {
        let data = download_example("example-apigw-request.json", None)
            .await
            .expect("failed to download json");
        assert!(data.contains("\"path\": \"/hello/world\""));
    }

    #[test]
    fn test_example_name() {
        assert_eq!(example_name("apigw-request"), "example-apigw-request.json");
        assert_eq!(
            example_name("apigw-request.json"),
            "example-apigw-request.json"
        );
        assert_eq!(
            example_name("example-apigw-request"),
            "example-apigw-request.json"
        );
        assert_eq!(
            example_name("example-apigw-request.json"),
            "example-apigw-request.json"
        );
    }
}
