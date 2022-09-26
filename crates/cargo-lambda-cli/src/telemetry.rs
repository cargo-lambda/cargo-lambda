use miette::{IntoDiagnostic, Result};
use sentry::Breadcrumb;
use serde::Serialize;
use std::{
    collections::{BTreeMap, HashMap},
    env,
};
use sysinfo::{System, SystemExt};
use tokio::task::JoinHandle;

const TELEMETRY_URL: &str = "";

#[derive(Debug, Serialize)]
pub(crate) struct Data<'a> {
    device_id: uuid::Uuid,
    os_name: Option<String>,
    os_version: Option<String>,
    app_version: String,
    event_properties: HashMap<&'a str, String>,
}

impl<'a> Default for Data<'a> {
    fn default() -> Self {
        let system = System::default();
        let mut args = env::args_os();
        let _ = args.next(); // program name
        let _ = args.next(); // `lambda` as subcommand
        let args = args
            .map(|a| a.to_string_lossy().to_string())
            .collect::<Vec<String>>()
            .join(" ");

        let mut event_properties = HashMap::new();
        event_properties.insert("arguments", args);
        Data {
            device_id: uuid::Uuid::new_v4(),
            os_name: system.name(),
            os_version: system.os_version(),
            app_version: version(),
            event_properties,
        }
    }
}

impl<'a> Data<'a> {
    pub(crate) fn breadcrumb() -> Breadcrumb {
        let tm = Data::default();
        let mut data = BTreeMap::new();
        if let Some(name) = tm.os_name {
            data.insert("os_name".to_string(), name.into());
        }
        if let Some(version) = tm.os_version {
            data.insert("os_version".to_string(), version.into());
        }
        data.insert(
            "arguments".to_string(),
            tm.event_properties["arguments"].clone().into(),
        );
        Breadcrumb {
            data,
            ..Default::default()
        }
    }
}

pub(crate) fn version() -> String {
    format!(
        "{} {}",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_LAMBDA_BUILD_INFO")
    )
    .trim_end()
    .into()
}

pub(crate) async fn send_telemetry_data() -> JoinHandle<Result<()>> {
    if is_do_not_track_enabled() {
        tokio::spawn(async { Ok(()) })
    } else {
        tokio::spawn(async { send_environment_data().await })
    }
}

pub(crate) fn is_do_not_track_enabled() -> bool {
    env::var("DO_NOT_TRACK").is_ok()
}

pub(crate) fn enable_do_not_track() {
    env::set_var("DO_NOT_TRACK", "true")
}

async fn send_environment_data() -> Result<()> {
    let data = Data::default();

    let client = reqwest::Client::new();
    let res = client
        .post(TELEMETRY_URL)
        .json(&data)
        .send()
        .await
        .into_diagnostic()?;

    tracing::debug!(status = %res.status(), data = ?data, "telemetry data sent");
    Ok(())
}
