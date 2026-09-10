//! Error types with source references.
//!
//! The goal: every error should point at the exact line of config that is
//! wrong. The `kdl` crate implements `miette::Diagnostic`, so parse errors get
//! caret-annotated source quoting for free; we attach spans to our own
//! semantic errors.

use std::path::PathBuf;

use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("cannot read {path}")]
    #[diagnostic(code(lami::io))]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    #[diagnostic(transparent)]
    Kdl(#[from] kdl::KdlError),

    #[error("no host named '{name}'")]
    #[diagnostic(
        code(lami::unknown_host),
        help("known hosts: {known}\nadd hosts/{name}.kdl, or use `lami --host <name>` to test another profile")
    )]
    UnknownHost { name: String, known: String },

    #[error("layer '{layer}' does not exist")]
    #[diagnostic(code(lami::unknown_layer))]
    UnknownLayer {
        layer: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("referenced here")]
        span: SourceSpan,
    },

    #[error("{msg}")]
    #[diagnostic(code(lami::config))]
    Config {
        msg: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("{label}")]
        span: SourceSpan,
        label: String,
    },

    #[error("{0}")]
    #[diagnostic(code(lami::other))]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
