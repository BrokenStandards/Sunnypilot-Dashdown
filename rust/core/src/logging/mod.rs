//! `tracing` → `LogSink` bridge (M8). A global `tracing_subscriber` layer renders
//! each event into a [`LogEvent`] and forwards it to the native-implemented
//! [`LogSink`], filtered by a configurable [`LogLevel`]. Fields named like a
//! secret (`password`/`pw`/`token`/`secret`) are redacted so credentials never
//! reach the log surface. `AppCore::new` calls [`install`] once;
//! `AppCore::set_log_sink` calls [`set_sink`].

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

use crate::model::time::now_ms;

/// Severity of a forwarded log event (and the verbosity threshold for the sink).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// 0 = most severe (Error) … 4 = least severe (Trace). An event is forwarded
    /// when its rank is `<=` the configured threshold rank.
    fn rank(self) -> u8 {
        match self {
            LogLevel::Error => 0,
            LogLevel::Warn => 1,
            LogLevel::Info => 2,
            LogLevel::Debug => 3,
            LogLevel::Trace => 4,
        }
    }

    fn from_tracing(l: &Level) -> LogLevel {
        match *l {
            Level::ERROR => LogLevel::Error,
            Level::WARN => LogLevel::Warn,
            Level::INFO => LogLevel::Info,
            Level::DEBUG => LogLevel::Debug,
            Level::TRACE => LogLevel::Trace,
        }
    }
}

/// One rendered log line crossing to the native layer.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct LogEvent {
    pub level: LogLevel,
    pub target: String,
    pub message: String,
    pub timestamp_ms: i64,
}

/// Implemented by the native layer to receive forwarded log events.
#[uniffi::export(with_foreign)]
pub trait LogSink: Send + Sync {
    fn on_log(&self, event: LogEvent);
}

static SINK: RwLock<Option<Arc<dyn LogSink>>> = RwLock::new(None);
static LEVEL: AtomicU8 = AtomicU8::new(2); // default Info
static INSTALLED: OnceLock<()> = OnceLock::new();

/// Install the forwarding layer into the global `tracing` subscriber, once per
/// process. Safe to call repeatedly (subsequent calls and an already-set global
/// subscriber are no-ops — never panics).
pub fn install() {
    INSTALLED.get_or_init(|| {
        let _ = tracing_subscriber::registry().with(ForwardLayer).try_init();
    });
}

/// Set (or clear) the active sink and the verbosity threshold. A level without a
/// sink simply means nothing is forwarded.
pub fn set_sink(sink: Option<Arc<dyn LogSink>>, level: LogLevel) {
    LEVEL.store(level.rank(), Ordering::Relaxed);
    // Poison-tolerant: a panic elsewhere must not wedge the log path.
    *SINK.write().unwrap_or_else(|e| e.into_inner()) = sink;
}

struct ForwardLayer;

impl<S> Layer<S> for ForwardLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let level = LogLevel::from_tracing(event.metadata().level());
        if level.rank() > LEVEL.load(Ordering::Relaxed) {
            return; // below the configured verbosity threshold
        }
        // Clone the Arc out and DROP the guard before calling the foreign sink:
        // holding the read lock across `on_log` would deadlock on a re-entrant
        // tracing call from foreign code, and a foreign panic would poison SINK.
        let sink = {
            let guard = SINK.read().unwrap_or_else(|e| e.into_inner());
            match guard.as_ref() {
                Some(s) => s.clone(),
                None => return, // no sink attached
            }
        };
        let mut v = MessageVisitor::default();
        event.record(&mut v);
        sink.on_log(LogEvent {
            level,
            target: event.metadata().target().to_string(),
            message: v.render(),
            timestamp_ms: now_ms(),
        });
    }
}

/// Collects the event's `message` plus structured fields, redacting secrets.
#[derive(Default)]
struct MessageVisitor {
    msg: String,
    fields: String,
}

impl MessageVisitor {
    fn push_field(&mut self, name: &str, rendered: &str) {
        if name == "message" {
            self.msg = rendered.to_string();
            return;
        }
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        self.fields.push_str(name);
        self.fields.push('=');
        self.fields.push_str(rendered);
    }

    fn render(self) -> String {
        match (self.msg.is_empty(), self.fields.is_empty()) {
            (false, true) => self.msg,
            (true, false) => self.fields,
            (false, false) => format!("{} {}", self.msg, self.fields),
            (true, true) => String::new(),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let name = field.name();
        if is_secret(name) {
            self.push_field(name, "<redacted>");
        } else {
            self.push_field(name, &format!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        let name = field.name();
        if is_secret(name) {
            self.push_field(name, "<redacted>");
        } else {
            self.push_field(name, value);
        }
    }
}

/// Whether a field name looks like a credential that must never be logged.
fn is_secret(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("password") || n == "pw" || n.contains("secret") || n.contains("token")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_field_names_are_detected() {
        for s in [
            "password",
            "pw",
            "device_password",
            "auth_token",
            "client_secret",
        ] {
            assert!(is_secret(s), "{s} should be secret");
        }
        for ok in ["message", "drive_key", "device_id", "url"] {
            assert!(!is_secret(ok), "{ok} should not be secret");
        }
    }

    #[test]
    fn render_combines_message_and_fields_and_redacts() {
        let mut v = MessageVisitor::default();
        v.push_field("message", "syncing device");
        // Non-secret field rendered verbatim; secret rendered as the redaction
        // marker by the record_* path (here we feed the already-decided value).
        v.push_field("device_id", "7");
        v.push_field("password", "<redacted>");
        let out = v.render();
        assert!(out.starts_with("syncing device"));
        assert!(out.contains("device_id=7"));
        assert!(out.contains("password=<redacted>"));
    }

    #[test]
    fn level_rank_ordering() {
        assert!(LogLevel::Error.rank() < LogLevel::Warn.rank());
        assert!(LogLevel::Info.rank() < LogLevel::Trace.rank());
        assert_eq!(LogLevel::from_tracing(&Level::WARN), LogLevel::Warn);
    }
}
