use lambda_extension::*;
{%- assign use_events = false -%}
{%- if logs or telemetry -%}
    {%- if events -%}
        {%- assign use_events = true -%}
    {%- endif -%}
{%- else -%}
{%- assign use_events = true -%}
{%- endif -%}

{%- if logs %}

async fn logs_extension(logs: Vec<LambdaLog>) -> Result<(), Error> {
    for log in logs {
        match log.record {
            LambdaLogRecord::Function(record) => {
                tracing::info!(log_type = "function", record = ?record, "received function logs");
            }
            LambdaLogRecord::Extension(record) => {
                tracing::info!(log_type = "extension", record = ?record, "received extension logs");
            },
            _ignore_other => {},
        }
    }

    Ok(())
}
{%- endif -%}
{%- if telemetry %}

async fn telemetry_extension(events: Vec<LambdaTelemetry>) -> Result<(), Error> {
    for event in events {
        match event.record {
            LambdaTelemetryRecord::Function(record) => {
                tracing::info!(telemetry_type = "function", record = ?record, "received function telemetry");
            }
            _ignore_other => {},
        }
    }

    Ok(())
}
{%- endif -%}
{%- if use_events %}

async fn events_extension(event: LambdaEvent) -> Result<(), Error> {
    match event.next {
        NextEvent::Shutdown(e) => {
            tracing::info!(event_type = "shutdown", event = ?e, "shutting down");
        }
        NextEvent::Invoke(e) => {
            tracing::info!(event_type = "invoke", event = ?e, "invoking function");
        }
    }
    Ok(())
}
{%- endif %}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing::init_default_subscriber();

    {% if logs -%}
    let logs_processor = SharedService::new(service_fn(logs_extension));
    {% endif -%}
    {% if telemetry -%}
    let telemetry_processor = SharedService::new(service_fn(telemetry_extension));
    {% endif %}
    Extension::new()
        {%- if use_events %}
        .with_events_processor(service_fn(events_extension))
        {%- endif -%}
        {%- if logs %}
        .with_logs_processor(logs_processor)
        {%- endif -%}
        {%- if telemetry %}
        .with_telemetry_processor(telemetry_processor)
        {%- endif %}
        .run()
        .await
}