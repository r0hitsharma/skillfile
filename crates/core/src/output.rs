use std::sync::atomic::{AtomicBool, Ordering};

static QUIET: AtomicBool = AtomicBool::new(false);

/// Enable or disable quiet mode (suppresses progress output).
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// Returns `true` if quiet mode is active.
pub fn is_quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

/// Print a progress message to stderr (suppressed with `--quiet`).
///
/// Usage: `progress!("Syncing {count} entries...");`
#[macro_export]
macro_rules! progress {
    ($($arg:tt)*) => {
        if !$crate::output::is_quiet() {
            eprintln!($($arg)*);
        }
    };
}

/// Print an inline progress message to stderr without a newline (suppressed with `--quiet`).
///
/// Usage: `progress_inline!("  resolving ...");`
#[macro_export]
macro_rules! progress_inline {
    ($($arg:tt)*) => {
        if !$crate::output::is_quiet() {
            eprint!($($arg)*);
        }
    };
}
