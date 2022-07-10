use opentelemetry::{
    global,
    sdk::{export::trace::stdout, trace},
};
use opentelemetry_aws::trace::XrayPropagator;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub(crate) fn init_tracing(print_traces: bool) {
    global::set_text_map_propagator(XrayPropagator::default());

    let builder = stdout::new_pipeline().with_trace_config(
        trace::config()
            .with_sampler(trace::Sampler::AlwaysOn)
            .with_id_generator(trace::XrayIdGenerator::default()),
    );
    let tracer = if print_traces {
        builder.install_simple()
    } else {
        builder.with_writer(std::io::sink()).install_simple()
    };
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);

    let fmt = tracing_subscriber::fmt::layer()
        .with_target(false)
        .without_time();

    tracing_subscriber::registry()
        .with(telemetry)
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "cargo_lambda=info".into()),
        ))
        .with(fmt)
        .init();
}
