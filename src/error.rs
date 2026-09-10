//! Hibatípusok forrás-hivatkozással.
//!
//! A cél, hogy minden hiba megmondja, a config MELYIK SORÁBAN van a baj.
//! A `kdl` crate `miette::Diagnostic`-ot implementál, ezért a parse-hibák
//! ingyen kapnak caret-es idézést; a szemantikai hibáinkhoz mi tesszük hozzá
//! a span-t.

use std::path::PathBuf;

use miette::{Diagnostic, NamedSource, SourceSpan};
use thiserror::Error;

#[derive(Debug, Error, Diagnostic)]
pub enum Error {
    #[error("nem olvasható: {path}")]
    #[diagnostic(code(lami::io))]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error(transparent)]
    #[diagnostic(transparent)]
    Kdl(#[from] kdl::KdlError),

    #[error("'{name}' nincs a hosts/ könyvtárban")]
    #[diagnostic(
        code(lami::unknown_host),
        help("ismert gépek: {known}\nvedd fel a hosts/{name}.kdl fájlt, vagy teszteléshez: lami --host <nev> ...")
    )]
    UnknownHost { name: String, known: String },

    #[error("a(z) '{layer}' réteg nem létezik")]
    #[diagnostic(code(lami::unknown_layer))]
    UnknownLayer {
        layer: String,
        #[source_code]
        src: NamedSource<String>,
        #[label("itt hivatkozol rá")]
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
