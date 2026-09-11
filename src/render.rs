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
fn context(r: &Resolved<'_>, layer: &Layer, home: &std::path::Path, user: &str) -> JValue {
    let mut map = std::collections::BTreeMap::new();
    // The resolved parameters, not the host file's: a layer's `default=` has
    // to reach a template as well, or a host that leaves a line out would
    // render an empty value instead of the declared default.
    for (k, v) in &r.params {
        let jv = match v {
            Value::Str(s) => JValue::from(s.clone()),
            Value::Int(i) => JValue::from(*i),
            Value::Bool(b) => JValue::from(*b),
            Value::List(l) => JValue::from(l.clone()),
        };
        map.insert(k.clone(), jv);
    }
    map.insert("layer".into(), JValue::from(layer.name.clone()));
    // Needed by anything that has to write an absolute path into a file --
    // .desktop Exec lines and config formats that do not expand `~`.
    map.insert("home".into(), JValue::from(home.display().to_string()));
    map.insert("user".into(), JValue::from(user.to_string()));
    JValue::from(map)
}

/// Render one file's content for this host.
/// Whether a source file is a template.
///
/// Only a `*.tmpl` name is. Rendering everything was convenient and wrong in
/// two ways: a static file that happens to contain `{{` would break or be
/// mangled, and -- worse -- `capture` writes the live content back into the
/// source, so capturing a rendered template would replace `{{ cpu_threads }}`
/// with `8` and destroy the template.
///
/// Inline `text` is always a template: it is written in the layer, so there is
/// nothing to capture back into.
pub fn is_template(p: &std::path::Path) -> bool {
    p.to_string_lossy().ends_with(".tmpl")
}

pub fn file(
    r: &Resolved<'_>,
    layer: &Layer,
    decl: &FileDecl,
    settings: &crate::config::Settings,
    home: &std::path::Path,
    user: &str,
) -> Result<String> {
    let templated = match &decl.source {
        Source::Text(_) => true,
        Source::From(p) => is_template(p),
    };

    let raw = match &decl.source {
        Source::Text(t) => t.clone(),
        // A source named *.age is decrypted on the way out. No attribute to
        // remember, so there is no way to commit a secret in the clear by
        // forgetting one.
        Source::From(p) if crate::secret::is_encrypted(p) => {
            crate::secret::decrypt(p, settings.age_identity.as_deref())?
        }
        Source::From(p) => fs::read_to_string(p).map_err(|source| Error::Io {
            path: p.clone(),
            source,
        })?,
    };

    // A plain source is copied through untouched. Only *.tmpl is rendered.
    if !templated {
        return Ok(with_trailing_newline(raw));
    }

    let mut env = Environment::empty();

    // Jinja strips one trailing newline by default -- a convention that makes
    // sense for HTML, and quietly corrupts config files. Without this, a file
    // ending in a blank line loses it on every render.
    env.set_keep_trailing_newline(true);

    let name = decl.path.clone();
    let rendered = env
        .render_named_str(&name, &raw, context(r, layer, home, user))
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

/// Ensure the content ends with a newline -- and only that.
///
/// KDL's multi-line strings dedent and strip the final newline, and a
/// single-line `text "{{ hostname }}"` never had one. Writing /etc/hostname
/// without a trailing newline would differ from what every other tool
/// produces and show up as a spurious diff forever.
///
/// Crucially this only ADDS; it never removes. An earlier version collapsed
/// every trailing newline into one, which silently rewrote any file that
/// legitimately ends with a blank line -- /etc/conf.d/snapper as shipped by
/// the snapper package does exactly that. Mutating a user's file to satisfy
/// our own tidiness is not the tool's business.
fn with_trailing_newline(mut s: String) -> String {
    if s.is_empty() || s.ends_with('\n') {
        return s;
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
