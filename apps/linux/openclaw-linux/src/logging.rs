use log::{LevelFilter, Log, Metadata, Record};
use std::sync::Once;

static INIT: Once = Once::new();

pub fn init() {
    INIT.call_once(|| {
        let level = resolve_level();
        let logger = StdoutLogger { level };
        let _ = log::set_boxed_logger(Box::new(logger));
        log::set_max_level(level);
    });
}

struct StdoutLogger {
    level: LevelFilter,
}

impl Log for StdoutLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        println!(
            "[{level}] {target} {message}",
            level = record.level(),
            target = record.target(),
            message = record.args()
        );
    }

    fn flush(&self) {}
}

fn resolve_level() -> LevelFilter {
    let raw = std::env::var("OPENCLAW_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    match raw.trim().to_ascii_lowercase().as_str() {
        "trace" => LevelFilter::Trace,
        "debug" => LevelFilter::Debug,
        "info" => LevelFilter::Info,
        "warn" | "warning" => LevelFilter::Warn,
        "error" => LevelFilter::Error,
        "off" => LevelFilter::Off,
        _ => LevelFilter::Info,
    }
}
