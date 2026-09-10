//! Error types with source references.
//!
//! The goal: every error should point at the exact line of config that is
//! wrong. The `kdl` crate implements `miette::Diagnostic`, so parse errors get
//! caret-annotated source quoting for free; we attach spans to our own
//! semantic errors.

use std::path::PathBuf;

use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;

/// The two variants that quote config source are boxed: they carry a whole
/// file's text, and `Result<T, Error>` is the return type of nearly every
/// function here. Without the box the happy path pays for the error path.
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

    #[error(transparent)]
    #[diagnostic(transparent)]
    UnknownLayer(Box<UnknownLayerError>),

    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(Box<ConfigError>),

    #[error("{0}")]
    #[diagnostic(code(lami::other))]
    Other(String),
}

/// A layer name that no layer directory answers to.
#[derive(Debug, Error, Diagnostic)]
#[error("layer '{layer}' does not exist")]
#[diagnostic(code(lami::unknown_layer))]
pub struct UnknownLayerError {
    pub layer: String,
    #[source_code]
    pub src: NamedSource<String>,
    #[label("referenced here")]
    pub span: SourceSpan,
}

/// Anything wrong with the config that we can point at a line of.
#[derive(Debug, Error, Diagnostic)]
#[error("{msg}")]
#[diagnostic(code(lami::config))]
pub struct ConfigError {
    pub msg: String,
    #[source_code]
    pub src: NamedSource<String>,
    #[label("{label}")]
    pub span: SourceSpan,
    pub label: String,
}

pub type Result<T> = std::result::Result<T, Error>;
