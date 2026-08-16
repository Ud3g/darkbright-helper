//! Process-wide panic recording.
//!
//! Release builds hide the console, so a panic's default stderr report goes
//! nowhere. The hook installed here routes every panic through the `log`
//! sinks (console + optional rolling file) and flushes them before the
//! previously installed handler continues, so crashes leave a trace in the
//! artifact users can actually retrieve.

use std::any::Any;

/// Extracts the human-readable message from a panic payload.
///
/// Payloads are `&str` (from `panic!("literal")`) or `String` (from
/// `panic!("formatted {x}")`); anything else (`std::panic::panic_any`) has
/// no portable text form and yields a fixed placeholder.
#[must_use]
pub(crate) fn payload_message(payload: &dyn Any) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("<non-string panic payload>")
}

/// Installs a process-wide panic hook that logs the panic (payload, source
/// location, thread) at error level and flushes the active logger, then
/// chains to the previously installed hook. Call once at startup, after the
/// logger is installed.
///
/// Runs on whatever thread panics. Logging first and flushing second means
/// the record reaches the file sink even when the panic goes on to abort
/// the process (e.g. by unwinding into an `extern "system"` callback).
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map_or_else(|| "<unknown>".to_owned(), ToString::to_string);
        let thread = std::thread::current();
        log::error!(
            payload = payload_message(info.payload()),
            location:% = location,
            thread = thread.name().unwrap_or("<unnamed>");
            "Thread panicked"
        );
        log::logger().flush();
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn payload_message_extracts_str_literal() {
        let payload: Box<dyn Any + Send> = Box::new("boom");
        assert_eq!(payload_message(payload.as_ref()), "boom");
    }

    #[test]
    fn payload_message_extracts_formatted_string() {
        let code = 7;
        let payload: Box<dyn Any + Send> = Box::new(format!("code {code}"));
        assert_eq!(payload_message(payload.as_ref()), "code 7");
    }

    #[test]
    fn payload_message_falls_back_for_non_string_payload() {
        let payload: Box<dyn Any + Send> = Box::new(42u32);
        assert_eq!(
            payload_message(payload.as_ref()),
            "<non-string panic payload>"
        );
    }

    /// Records every log line (message + rendered key-values) and whether
    /// `flush` was called.
    struct CaptureLogger {
        lines: Mutex<Vec<String>>,
        flushed: AtomicBool,
    }

    /// Renders key-value pairs as ` key=value` for content assertions.
    struct KvCollector<'a>(&'a mut String);

    impl<'kvs> log::kv::VisitSource<'kvs> for KvCollector<'_> {
        fn visit_pair(
            &mut self,
            key: log::kv::Key<'kvs>,
            value: log::kv::Value<'kvs>,
        ) -> Result<(), log::kv::Error> {
            let _ = write!(self.0, " {key}={value}");
            Ok(())
        }
    }

    impl log::Log for CaptureLogger {
        fn enabled(&self, _metadata: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            let mut line = record.args().to_string();
            let _ = record.key_values().visit(&mut KvCollector(&mut line));
            self.lines.lock().unwrap().push(line);
        }

        fn flush(&self) {
            self.flushed.store(true, Ordering::SeqCst);
        }
    }

    #[test]
    fn hook_logs_payload_and_location_and_flushes() {
        static LOGGER: CaptureLogger = CaptureLogger {
            lines: Mutex::new(Vec::new()),
            flushed: AtomicBool::new(false),
        };
        let _ = log::set_logger(&LOGGER);
        log::set_max_level(log::LevelFilter::Error);

        install();
        let result = std::panic::catch_unwind(|| panic!("unique-panic-payload-3f9c"));
        assert!(result.is_err());

        let joined = LOGGER.lines.lock().unwrap().join("\n");
        assert!(
            joined.contains("unique-panic-payload-3f9c"),
            "payload missing in: {joined}"
        );
        assert!(
            joined.contains("panic_hook.rs"),
            "source location missing in: {joined}"
        );
        assert!(
            LOGGER.flushed.load(Ordering::SeqCst),
            "logger was not flushed"
        );
    }
}
