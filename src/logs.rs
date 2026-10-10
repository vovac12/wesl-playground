//! Captures warnings and errors from the tracing pipeline so they can be reported over MCP.

use std::{
    collections::VecDeque,
    fmt::Write as _,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use bevy::{
    log::{
        BoxedLayer,
        tracing::{self, Level, Subscriber, field::Field},
        tracing_subscriber::{Layer, field::Visit, layer::Context},
    },
    prelude::*,
};
use serde::Serialize;

const CAPACITY: usize = 500;

static LOGS: Mutex<VecDeque<LogEntry>> = Mutex::new(VecDeque::new());
static SEQ: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Serialize)]
pub struct LogEntry {
    pub seq: u64,
    pub level: String,
    pub target: String,
    pub message: String,
}

/// Sequence number the next captured entry will get.
pub fn next_seq() -> u64 {
    SEQ.load(Ordering::SeqCst)
}

/// Entries with `seq >= since` matching `filter`.
pub fn since(since: u64, filter: impl Fn(&LogEntry) -> bool) -> Vec<LogEntry> {
    LOGS.lock()
        .unwrap()
        .iter()
        .filter(|e| e.seq >= since && filter(e))
        .cloned()
        .collect()
}

pub fn recent(count: usize) -> Vec<LogEntry> {
    let logs = LOGS.lock().unwrap();
    logs.iter()
        .skip(logs.len().saturating_sub(count))
        .cloned()
        .collect()
}

pub fn custom_layer(_app: &mut App) -> Option<BoxedLayer> {
    Some(Box::new(CaptureLayer))
}

struct CaptureLayer;

impl<S: Subscriber> Layer<S> for CaptureLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        if *meta.level() > Level::WARN {
            return;
        }
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        let entry = LogEntry {
            seq: SEQ.fetch_add(1, Ordering::SeqCst),
            level: meta.level().to_string(),
            target: meta.target().to_string(),
            message: visitor.0,
        };
        let mut logs = LOGS.lock().unwrap();
        if logs.len() >= CAPACITY {
            logs.pop_front();
        }
        logs.push_back(entry);
    }
}

struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if !self.0.is_empty() {
            self.0.push(' ');
        }
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, "{}={value:?}", field.name());
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            if !self.0.is_empty() {
                self.0.push(' ');
            }
            self.0.push_str(value);
        } else {
            self.record_debug(field, &value);
        }
    }
}
