//! Terminal colour.
//!
//! On by default, and off whenever it would be noise rather than help:
//! when the output is not a terminal (so a pipe or a file gets clean text),
//! when `NO_COLOR` is set, or when `--no-color` is passed.
//!
//! Colour carries meaning here rather than decoration -- green adds, yellow
//! changes, red removes -- so the same information has to survive without it.
//! Every line is readable in plain text; the sigils (+ ~ - >) say the same
//! thing the colour does.

use std::io::IsTerminal;
use std::sync::OnceLock;

static ENABLED: OnceLock<bool> = OnceLock::new();

/// Decide once, at startup, whether to colour anything.
pub fn init(force_off: bool) {
    let on = !force_off
        // The de-facto standard: https://no-color.org
        && std::env::var_os("NO_COLOR").is_none()
        && std::io::stdout().is_terminal();
    let _ = ENABLED.set(on);
}

fn on() -> bool {
    *ENABLED.get().unwrap_or(&false)
}

fn wrap(code: &str, s: &str) -> String {
    if on() {
        format!("\x1b[{code}m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn bold(s: &str) -> String {
    wrap("1", s)
}
/// Something being added.
pub fn added(s: &str) -> String {
    wrap("32", s)
}
/// Something being changed in place.
pub fn changed(s: &str) -> String {
    wrap("33", s)
}
/// Something being removed. Also used for errors.
pub fn removed(s: &str) -> String {
    wrap("31", s)
}
/// A command that will be run.
pub fn action(s: &str) -> String {
    wrap("36", s)
}
/// Secondary detail: paths, origins, counts.
pub fn dim(s: &str) -> String {
    wrap("2", s)
}
/// A heading.
pub fn heading(s: &str) -> String {
    wrap("1;34", s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_when_disabled() {
        // The suite does not run on a terminal, so colour is off and every
        // helper must be the identity function. This is what keeps piped
        // output clean.
        init(true);
        assert_eq!(bold("x"), "x");
        assert_eq!(added("x"), "x");
        assert_eq!(dim("x"), "x");
    }
}
