//! Process-wide progress settings.

use super::*;

// ── Progress settings and per-phase counters ─────────────────────────────────

/// Process-wide progress settings.
pub(super) struct ProgressSettings {
    pub(super) enabled: bool,
    pub(super) tty: bool,
    pub(super) verbose: bool,
}

static SETTINGS: OnceLock<Mutex<ProgressSettings>> = OnceLock::new();

fn settings() -> &'static Mutex<ProgressSettings> {
    SETTINGS.get_or_init(|| {
        Mutex::new(ProgressSettings {
            enabled: false,
            tty: false,
            verbose: false,
        })
    })
}

pub(super) fn with_settings<T>(f: impl FnOnce(&mut ProgressSettings) -> T) -> T {
    let mut guard = match settings().lock() {
        Ok(guard) => guard,
        Err(poison) => poison.into_inner(),
    };
    f(&mut guard)
}

/// Enable progress rendering for this process.
pub fn init(enabled: bool, _no_color: bool, verbose: bool) {
    let enabled = enabled && std::env::var_os("XPM_NO_PROGRESS").is_none();
    with_settings(|state| {
        state.enabled = enabled;
        state.tty = io::stderr().is_terminal();
        state.verbose = verbose;
    });
}

/// Whether detailed per-package lines (downloads, extraction) should print.
pub fn verbose() -> bool {
    with_settings(|state| state.verbose)
}

/// Print a persistent message.
pub fn message(msg: impl AsRef<str>) {
    eprintln!("{}", msg.as_ref());
}

/// Print a warning.
pub fn warn(msg: impl AsRef<str>) {
    eprintln!("warning: {}", msg.as_ref());
}

/// Finish any active progress state. Safe to call multiple times.
pub fn finish_all() {
    with_settings(|state| {
        if state.enabled && state.tty {
            let _ = writeln!(io::stderr());
        }
        state.enabled = false;
    });
}
