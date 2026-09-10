//! Pulling changes made on the machine back into the repo.
//!
//! This is the half that most configuration tools simply do not have, and the
//! reason the config format had to preserve comments: capture writes into the
//! files you hand-wrote. A tool that reformats your config, or eats the comment
//! explaining why a package is there, is not one you would keep using.
//!
//! Two rules, both from known kdl-rs behaviour rather than caution:
//!
//! 1. **Never call `autoformat()`.** It silently deletes an end-of-line comment
//!    (kdl-rs issue #179) and rewrites number literals. It is exactly the wrong
//!    function for this job.
//!
//! 2. **Inherit a sibling's INDENTATION, not its formatting.** A freshly built
//!    node has empty leading whitespace and would be glued onto the previous
//!    line. But a node's `leading` also holds any comment written above it, so
//!    copying it wholesale duplicates that comment onto the new node -- worse
//!    than losing it, because it now says something untrue about a different
//!    package. Only the whitespace after the last newline is taken.

use std::path::Path;

use kdl::{KdlDocument, KdlNode};

use crate::error::{Error, Result};

/// The whitespace a sibling node is indented by.
///
/// A node's `leading` holds everything between the previous node and this one,
/// which includes any comment written above it. Only the run of spaces or tabs
/// after the final newline is actual indentation.
fn indent_of(leading: &str) -> String {
    leading
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// The result of a capture, whether or not it was written.
#[derive(Debug)]
pub struct Edit {
    pub file: std::path::PathBuf,
    pub before: String,
    pub after: String,
}

impl Edit {
    /// The lines that would change, as a plain unified-ish summary.
    pub fn summary(&self) -> String {
        let b: Vec<&str> = self.before.lines().collect();
        let a: Vec<&str> = self.after.lines().collect();
        let mut out: Vec<String> = b
            .iter()
            .filter(|l| !a.contains(l))
            .map(|l| format!("-{l}"))
            .collect();
        out.extend(a.iter().filter(|l| !b.contains(l)).map(|l| format!("+{l}")));
        if out.is_empty() {
            "(no change)".into()
        } else {
            out.join("\n")
        }
    }
}

/// Add a package to a layer's top-level `packages` block.
///
/// Only the unconditional block is touched: a package captured from a running
/// machine has no `when` condition attached to it, and guessing one would be
/// worse than leaving the decision to the person.
pub fn add_package(layer_file: &Path, package: &str, dry: bool) -> Result<Edit> {
    let src = std::fs::read_to_string(layer_file).map_err(|source| Error::Io {
        path: layer_file.to_path_buf(),
        source,
    })?;
    let mut doc: KdlDocument = src.parse()?;

    let node = doc
        .nodes_mut()
        .iter_mut()
        .find(|n| n.name().value() == "packages")
        .ok_or_else(|| {
            Error::Other(format!(
                "{} has no top-level `packages` block to add to.\n\
                 Add one first, even if empty:\n\n  packages {{\n  }}",
                layer_file.display()
            ))
        })?;

    let children = node.ensure_children();

    if children
        .nodes()
        .iter()
        .any(|n| n.name().value() == package)
    {
        return Err(Error::Other(format!(
            "{package} is already declared in {}",
            layer_file.display()
        )));
    }

    // How KDL lays a block out: the newline between two nodes belongs to the
    // FIRST one's `terminator`, and `leading` is only the indentation. So an
    // appended node needs leading = indent, terminator = newline -- except in
    // an empty block, where no terminator precedes it and it must supply the
    // newline itself.
    let (indent, leading) = match children.nodes().last().and_then(|n| n.format()) {
        Some(f) => {
            let i = indent_of(&f.leading);
            (i.clone(), i)
        }
        None => ("    ".to_string(), "\n    ".to_string()),
    };
    let _ = indent;

    let mut new = KdlNode::new(package);
    new.set_format(kdl::KdlNodeFormat {
        leading,
        terminator: "\n".to_string(),
        ..Default::default()
    });
    children.nodes_mut().push(new);

    let after = doc.to_string();
    if !dry {
        std::fs::write(layer_file, &after).map_err(|source| Error::Io {
            path: layer_file.to_path_buf(),
            source,
        })?;
    }
    Ok(Edit {
        file: layer_file.to_path_buf(),
        before: src,
        after,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str, body: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("lami-cap-{}-{name}.kdl", std::process::id()));
        std::fs::write(&p, body).unwrap();
        p
    }

    const LAYER: &str = r#"description "a layer"
needs "core"

packages {
    firefox     // needed for work SSO, do not swap for chromium
    // a standalone note about the next one
    ghostty
}

services {
    greetd
}
"#;

    #[test]
    fn comments_survive_an_insert() {
        // The whole reason the format has to round-trip: a comment explaining
        // why a package is there must not be collateral damage.
        let p = tmp("comments", LAYER);
        add_package(&p, "ripgrep", false).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();

        assert!(
            out.contains("// needed for work SSO, do not swap for chromium"),
            "the end-of-line comment was lost:\n{out}"
        );
        assert!(
            out.contains("// a standalone note about the next one"),
            "the standalone comment was lost:\n{out}"
        );
        assert!(out.contains("ripgrep"), "{out}");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn everything_else_is_byte_identical() {
        let p = tmp("identical", LAYER);
        add_package(&p, "ripgrep", false).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();

        // Removing the one added line must give back exactly the original.
        let without: String = out
            .lines()
            .filter(|l| l.trim() != "ripgrep")
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(
            format!("{without}\n"),
            LAYER,
            "something other than the insertion changed"
        );
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn indentation_is_inherited_from_a_sibling() {
        let p = tmp("indent", LAYER);
        add_package(&p, "ripgrep", false).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert!(
            out.contains("\n    ripgrep"),
            "should match the block's existing indentation:\n{out}"
        );
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn a_comment_above_a_sibling_is_not_duplicated() {
        // A node's leading whitespace also holds the comment written above it.
        // Cloning it wholesale copied that comment onto the new node, which is
        // worse than losing it: it then says something untrue about a
        // different package.
        let p = tmp("nodupe-comment", LAYER);
        add_package(&p, "ripgrep", false).unwrap();
        let out = std::fs::read_to_string(&p).unwrap();
        assert_eq!(
            out.matches("// a standalone note about the next one").count(),
            1,
            "the comment was duplicated:\n{out}"
        );
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn a_duplicate_is_refused_rather_than_silently_added() {
        let p = tmp("dupe", LAYER);
        let err = add_package(&p, "firefox", false).unwrap_err();
        assert!(err.to_string().contains("already declared"), "{err}");
        std::fs::remove_file(&p).ok();
    }

    #[test]
    fn a_missing_packages_block_says_what_to_do() {
        let p = tmp("noblock", "description \"empty\"\n");
        let err = add_package(&p, "firefox", false).unwrap_err();
        assert!(err.to_string().contains("packages {"), "{err}");
        std::fs::remove_file(&p).ok();
    }
}

/// Copy a file's live content back into the layer that declares it.
///
/// Only `from=` sources can round-trip. An inline `text` block may contain
/// template expressions, and rendering is not reversible -- there is no way to
/// tell which part of the result came from `{{ cpu_threads }}`. Rather than
/// guess, those are reported as needing a hand edit, which is also why the
/// layer files in this repo prefer `from=` for anything likely to be tweaked
/// in place.
pub fn capture_file(
    decl: &crate::config::FileDecl,
    live: &Path,
    dry: bool,
) -> Result<Edit> {
    let source = match &decl.source {
        crate::config::Source::From(p) if crate::render::is_template(p) => {
            return Err(Error::Other(format!(
                "{} comes from a template ({}), which cannot be captured.\n\
                 \nRendering is not reversible: writing the live file back would\n\
                 replace the template expressions with whatever they evaluated to,\n\
                 and the template would be gone. Edit it in the layer instead.",
                decl.path,
                p.display()
            )))
        }
        crate::config::Source::From(p) => p.clone(),
        crate::config::Source::Text(_) => {
            return Err(Error::Other(format!(
                "{} is declared as an inline `text` block, which cannot be captured.\n\
                 \nRendering is not reversible: there is no way to tell which part of the\n\
                 file came from a template expression. Edit it in the layer instead:\n\
                 \n  {}",
                decl.path, decl.origin
            )))
        }
    };

    let after = std::fs::read_to_string(live).map_err(|source| Error::Io {
        path: live.to_path_buf(),
        source,
    })?;
    let before = std::fs::read_to_string(&source).unwrap_or_default();

    if !dry {
        std::fs::write(&source, &after).map_err(|e| Error::Io {
            path: source.clone(),
            source: e,
        })?;
    }
    Ok(Edit {
        file: source,
        before,
        after,
    })
}
