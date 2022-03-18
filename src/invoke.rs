use clap::{Args, ValueHint};
use miette::{IntoDiagnostic, Result, WrapErr};
use std::path::PathBuf;

#[derive(Args, Clone, Debug)]
#[clap(name = "invoke")]
pub struct Invoke {
    /// Address port where users send invoke requests
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
    /// Name of the function to invoke
    function_name: String,
}

impl Invoke {
    pub async fn run(&self) -> Result<()> {
        let data = if let Some(file) = &self.data_file {
            std::fs::read_to_string(file)
                .into_diagnostic()
                .wrap_err("error reading data file")?
        } else if let Some(data) = &self.data_ascii {
            data.clone()
        } else if let Some(example) = &self.data_example {
            let target = format!("https://raw.githubusercontent.com/LegNeato/aws-lambda-events/master/aws_lambda_events/src/generated/fixtures/example-{example}.json");

            let response = reqwest::get(target)
                .await
                .into_diagnostic()
                .wrap_err("error dowloading example data")?;

            if response.status() != axum::http::StatusCode::OK {
                return Err(miette::miette!(
                    "error downloading example data -- {:?}",
                    response
                ));
            }

            response
                .text()
                .await
                .into_diagnostic()
                .wrap_err("error reading example data")?
        } else {
            return Err(miette::miette!("no data payload provided, use one of the data flags: `--data-file`, `--data-ascii`, `--data-example`"));
        };

        let url = format!(
            "http://127.0.0.1:{}/2015-03-31/functions/{}/invocations",
            self.invoke_port, &self.function_name
        );

        let client = reqwest::Client::new();
        let resp = client
            .post(url)
            .body(data)
            .send()
            .await
            .into_diagnostic()
            .wrap_err("error sending request to the runtime emulator")?;

        let text = resp
            .text()
            .await
            .into_diagnostic()
            .wrap_err("error reading response body")?;

        println!("{text}");

        Ok(())
    }
}
