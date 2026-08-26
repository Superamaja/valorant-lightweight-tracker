//! Dev-only console log of what the tracker is doing: state transitions, rebuild triggers,
//! remote requests, enrichment progress and the connection lifecycle. Lines go to stderr, so
//! they appear in the terminal running `pnpm tauri dev` (see `docs/testing.md`).
//!
//! The module is declared unconditionally and cfg'd internally, so call sites never need a
//! `#[cfg]` of their own. In release builds `vlt_log!` swallows its tokens: nothing is
//! formatted, no argument is evaluated and no IO happens.
//!
//! Call-site rule: never introduce a binding solely to feed `vlt_log!` — in release the
//! swallowed tokens would leave it unused. Inline the expression into the macro call instead,
//! or gate the binding with `#[cfg(debug_assertions)]`.

/// Log one line under `category` (`state`, `rebuild`, `net`, `enrich`, `ws`, `conn`).
#[cfg(debug_assertions)]
macro_rules! vlt_log {
    ($category:expr, $($arg:tt)*) => {
        $crate::debug_log::log_line($category, format_args!($($arg)*))
    };
}

/// Release builds: the whole call, arguments included, compiles to nothing.
#[cfg(not(debug_assertions))]
macro_rules! vlt_log {
    ($category:expr, $($arg:tt)*) => {{}};
}

#[cfg(debug_assertions)]
mod imp {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::OnceLock;
    use std::time::Instant;

    /// Process start, taken on the first line so every timestamp shares one origin.
    static START: OnceLock<Instant> = OnceLock::new();

    /// Serial number shared by every remote request, so a response can be matched to the
    /// request that earned it even when several are in flight.
    static REQUEST_SEQ: AtomicU32 = AtomicU32::new(0);

    /// Write one `[vlt <seconds>] <category>: <message>` line to stderr.
    pub fn log_line(category: &str, args: std::fmt::Arguments) {
        let elapsed = START.get_or_init(Instant::now).elapsed().as_secs_f64();
        eprintln!("[vlt {elapsed:9.3}] {category}: {args}");
    }

    /// The next request serial.
    pub fn next_request_seq() -> u32 {
        REQUEST_SEQ.fetch_add(1, Ordering::Relaxed)
    }

    /// The first 8 characters of an id, for logs where the full puuid/match id is noise.
    /// Never splits a multi-byte character; an id shorter than that is returned whole.
    pub fn short(id: &str) -> &str {
        id.get(..8).unwrap_or(id)
    }
}

#[cfg(debug_assertions)]
pub use imp::{log_line, next_request_seq, short};

#[cfg(all(test, debug_assertions))]
mod tests {
    use super::short;

    #[test]
    fn truncates_a_long_id_to_eight_characters() {
        assert_eq!(short("0123456789abcdef"), "01234567");
    }

    #[test]
    fn a_short_id_is_returned_whole() {
        assert_eq!(short(""), "");
        assert_eq!(short("abc"), "abc");
        assert_eq!(short("01234567"), "01234567");
    }

    #[test]
    fn a_multibyte_boundary_falls_back_to_the_whole_id() {
        // Byte 8 lands inside the third 3-byte character, so no 8-byte prefix is a valid
        // string and the id survives intact rather than panicking.
        assert_eq!(short("日日日日"), "日日日日");
        // ...while a boundary that does land cleanly still truncates.
        assert_eq!(short("ééééabcd"), "éééé");
    }
}
