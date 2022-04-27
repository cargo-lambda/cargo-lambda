use cargo_lambda_remote::{aws_sdk_lambda::types::Blob, init_client, RemoteConfig};
use clap::{Args, ValueHint};
use miette::{IntoDiagnostic, Result, WrapErr};
use reqwest::{Client, StatusCode};
use serde_json::{from_str, to_string_pretty, value::Value};
use std::{
    fs::{create_dir_all, read_to_string, File},
    io::copy,
    net::IpAddr,
    path::{Path, PathBuf},
    str::{from_utf8, FromStr},
};
use strum_macros::{Display, EnumString};

/// Name for the function when no name is provided.
/// This will make the watch command to compile
/// the binary without the `--bin` option, and will
/// assume that the package only has one function,
/// which is the main binary for that package.
pub const DEFAULT_PACKAGE_FUNCTION: &str = "@package-bootstrap@";

#[derive(Args, Clone, Debug)]
#[clap(name = "invoke")]
pub struct Invoke {
    /// Local address host (IPv4 or IPv6) to send invoke requests
    #[clap(short = 'a', long, default_value = "127.0.0.1")]
    invoke_address: String,

    /// Local port to send invoke requests
    #[clap(short = 'p', long, default_value = "9000")]
    invoke_port: u16,

    /// File to read the invoke payload from
    #[clap(long, parse(from_os_str), value_hint = ValueHint::FilePath)]
    data_file: Option<PathBuf>,

    /// Invoke payload as a string
    #[clap(long)]
    data_ascii: Option<String>,

    /// Example payload from LegNeato/aws-lambda-events
    #[clap(long)]
    data_example: Option<String>,

    /// Invoke the function already deployed on AWS Lambda
    #[clap(long)]
    remote: bool,

    #[clap(flatten)]
    remote_config: RemoteConfig,

    #[clap(long, default_value_t = OutputFormat::Text)]
    output_format: OutputFormat,

    /// Name of the function to invoke
    #[clap(default_value = DEFAULT_PACKAGE_FUNCTION)]
    function_name: String,
}

#[derive(Clone, Debug, Display, EnumString)]
#[strum(ascii_case_insensitive)]
enum OutputFormat {
    Text,
    Json,
}

impl Invoke {
    pub async fn run(&self) -> Result<()> {
        let data = if let Some(file) = &self.data_file {
            read_to_string(file)
                .into_diagnostic()
                .wrap_err("error reading data file")?
        } else if let Some(data) = &self.data_ascii {
            data.clone()
        } else if let Some(example) = &self.data_example {
            let name = format!("example-{example}.json");

            let cache = home::cargo_home()
                .into_diagnostic()?
                .join("lambda")
                .join("invoke-fixtures")
                .join(&name);

            if cache.exists() {
                read_to_string(cache)
                    .into_diagnostic()
                    .wrap_err("error reading data file")?
            } else {
                download_example(&name, &cache).await?
            }
        } else {
            return Err(miette::miette!("no data payload provided, use one of the data flags: `--data-file`, `--data-ascii`, `--data-example`"));
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
            return Err(miette::miette!("invalid function name, it must match the name you used to create the function remotely"));
        }
        let client = init_client(&self.remote_config).await;

        let resp = client
            .invoke()
            .function_name(&self.function_name)
            .set_qualifier(self.remote_config.alias.clone())
            .payload(Blob::new(data.as_bytes()))
            .send()
            .await
            .into_diagnostic()
            .wrap_err("failed to invoke remote function")?;

        if let Some(fail) = resp.function_error {
            let blob = resp
                .payload
                .expect("function error is not in the payload")
                .into_inner();
            let text = from_utf8(&blob)
                .into_diagnostic()
                .wrap_err("failed to read response payload")?;
            return Err(miette::miette!("{}: {}", fail, text));
        }

        if let Some(payload) = resp.payload {
            let blob = payload.into_inner();
            from_utf8(&blob)
                .map(String::from)
                .into_diagnostic()
                .wrap_err("failed to read response payload")
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
        let resp = client
            .post(url)
            .body(data.to_string())
            .send()
            .await
            .into_diagnostic()
            .wrap_err("error sending request to the runtime emulator")?;

        resp.text()
            .await
            .into_diagnostic()
            .wrap_err("error reading response body")
    }
}

async fn download_example(name: &str, cache: &Path) -> Result<String> {
    let target = format!("https://raw.githubusercontent.com/LegNeato/aws-lambda-events/master/aws_lambda_events/src/generated/fixtures/{name}");

    let response = reqwest::get(target)
        .await
        .into_diagnostic()
        .wrap_err("error dowloading example data")?;

    if response.status() != StatusCode::OK {
        return Err(miette::miette!(
            "error downloading example data -- {:?}",
            response
        ));
    }

    let content = response
        .text()
        .await
        .into_diagnostic()
        .wrap_err("error reading example data")?;

    create_dir_all(cache.parent().unwrap()).into_diagnostic()?;
    let mut dest = File::create(cache).into_diagnostic()?;
    copy(&mut content.as_bytes(), &mut dest).into_diagnostic()?;
    Ok(content)
}

fn parse_invoke_ip_address(address: &str) -> Result<String> {
    let invoke_address = IpAddr::from_str(address).map_err(|e| miette::miette!(e))?;

    let invoke_address = match invoke_address {
        IpAddr::V4(address) => address.to_string(),
        IpAddr::V6(address) => format!("[{}]", address),
    };

    Ok(invoke_address)
}
