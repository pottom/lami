//! Rendering file contents.
//!
//! Contents are Jinja2 templates, the same shape you would use in an HTML
//! templating engine. Every host parameter is available as a variable, plus
//! `hostname` and `layer`.
//!
//! ```kdl
//! file "/etc/makepkg.conf.d/99-local.conf" {
//!     text """
//!     MAKEFLAGS="-j{{ cpu_threads }}"
//!     """
//! }
//! ```
//!
//! Note that parameter names reach templates verbatim, so use `cpu_threads`
//! rather than `cpu-threads` in the config: a hyphen would be read as
//! subtraction by the template engine.

use std::fs;

use minijinja::{Environment, Value as JValue};

use crate::config::{FileDecl, Layer, Resolved, Source, Value};
use crate::error::{Error, Result};

/// The template variables available to a file's content.
fn context(r: &Resolved<'_>, layer: &Layer) -> JValue {
    let mut map = std::collections::BTreeMap::new();
    for (k, v) in &r.host.params {
        let jv = match v {
            Value::Str(s) => JValue::from(s.clone()),
            Value::Int(i) => JValue::from(*i),
            Value::Bool(b) => JValue::from(*b),
            Value::List(l) => JValue::from(l.clone()),
        };
        map.insert(k.clone(), jv);
    }
    map.insert("hostname".into(), JValue::from(r.host.name.clone()));
    map.insert("layer".into(), JValue::from(layer.name.clone()));
    JValue::from(map)
}

/// Render one file's content for this host.
pub fn file(r: &Resolved<'_>, layer: &Layer, decl: &FileDecl) -> Result<String> {
    let raw = match &decl.source {
        Source::Text(t) => t.clone(),
        Source::From(p) => fs::read_to_string(p).map_err(|source| Error::Io {
            path: p.clone(),
            source,
        })?,
    };

    let env = Environment::new();
    let name = decl.path.clone();
    let rendered = env
        .render_named_str(&name, &raw, context(r, layer))
        .map_err(|e| {
            // minijinja reports the line within the template; point at the
            // declaration too, so the user knows which file to open.
            Error::Other(format!(
                "cannot render {}\n  declared at {}\n  {e}",
                decl.path, decl.origin
            ))
        })?;

    Ok(with_trailing_newline(rendered))
}

/// Config files end with a newline.
///
/// KDL's multi-line strings dedent and strip the final newline, and a
/// single-line `text "{{ hostname }}"` never had one. Writing /etc/hostname
/// without a trailing newline would differ from what every other tool
/// produces, and would show up as a spurious diff forever.
fn with_trailing_newline(mut s: String) -> String {
    if s.is_empty() {
        return s;
    }
    while s.ends_with('\n') {
        s.pop();
    }
    s.push('\n');
    s
}

/// Where a file should be written, with `~` expanded.
pub fn target_path(decl: &FileDecl, home: &std::path::Path) -> std::path::PathBuf {
    match decl.path.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => std::path::PathBuf::from(&decl.path),
    }
}
