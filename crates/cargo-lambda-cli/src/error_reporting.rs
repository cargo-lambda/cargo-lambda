use sentry_core::{types::Uuid, Hub};

use crate::telemetry::Data;

pub const SENTRY_DSN: &str = "";

pub fn capture_error(err: &miette::Error) -> Uuid {
    Hub::with_active(|hub| hub.capture_miette(err))
}

/// Hub extension methods for working with [`miette`].
pub trait MietteHubExt {
    /// Captures an [`miette::Error`] on a specific hub.
    fn capture_miette(&self, e: &miette::Error) -> Uuid;
}

impl MietteHubExt for Hub {
    fn capture_miette(&self, err: &miette::Error) -> Uuid {
        sentry_core::add_breadcrumb(Data::breadcrumb());
        let event = sentry_core::event_from_error(err.root_cause());
        self.capture_event(event)
    }
}
