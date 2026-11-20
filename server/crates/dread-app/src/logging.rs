//! Tracing setup: stderr + the in-app log buffer.

use dread_core::diag::{LogBuffer, LogLine};
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

struct BufferLayer(LogBuffer);

struct MsgVisitor(String);

impl Visit for MsgVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0.insert_str(0, &format!("{value:?}"));
        } else {
            self.0.push_str(&format!(" {}={:?}", field.name(), value));
        }
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.insert_str(0, value);
        } else {
            self.0.push_str(&format!(" {}={}", field.name(), value));
        }
    }
}

impl<S: Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, ev: &Event<'_>, _: Context<'_, S>) {
        let mut v = MsgVisitor(String::new());
        ev.record(&mut v);
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let secs = now.as_secs() % 86400;
        self.0.push(LogLine {
            time: format!("{:02}:{:02}:{:02}.{:03}", secs / 3600, (secs / 60) % 60, secs % 60, now.subsec_millis()),
            level: ev.metadata().level().to_string(),
            target: ev.metadata().target().to_string(),
            message: v.0,
        });
    }
}

pub fn init(level: &str) -> LogBuffer {
    let buf = LogBuffer::default();
    let filter = EnvFilter::try_from_env("DREAD_LOG").unwrap_or_else(|_| EnvFilter::new(format!("{level},wgpu=warn,eframe=warn,egui=warn,mdns_sd=warn,zbus=warn")));
    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr).with_target(false))
        .with(BufferLayer(buf.clone()))
        .init();
    buf
}
