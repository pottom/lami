//! What lami managed the last time it ran.
//!
//! Without this, "no longer declared" is indistinguishable from "never
//! declared". A package installed by hand and a package dropped from a layer
//! look identical from the outside, and only one of them should be a candidate
//! for removal.
//!
//! The file is deliberately plain JSON in /var/lib: readable, greppable, and
//! easy to repair by hand if it ever goes wrong. State that only the tool can
//! read is state you cannot recover from.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

const PATH: &str = "/var/lib/lami/state.json";

#[derive(Debug, Default)]
pub struct State {
    pub packages: BTreeSet<String>,
    pub files: BTreeSet<String>,
    pub services: BTreeSet<String>,
    pub host: String,
}

fn path() -> PathBuf {
    std::env::var("LAMI_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(PATH))
}

/// Minimal JSON, hand-rolled to avoid pulling in serde for four string sets.
fn encode(s: &State) -> String {
    let arr = |v: &BTreeSet<String>| {
        v.iter()
            .map(|x| format!("    \"{}\"", x.replace('\\', "\\\\").replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(",\n")
    };
    format!(
        "{{\n  \"host\": \"{}\",\n  \"packages\": [\n{}\n  ],\n  \"files\": [\n{}\n  ],\n  \"services\": [\n{}\n  ]\n}}\n",
        s.host,
        arr(&s.packages),
        arr(&s.files),
        arr(&s.services)
    )
}

fn decode(text: &str) -> State {
    let mut st = State::default();
    let mut section: Option<&mut BTreeSet<String>> = None;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("\"host\"") {
            st.host = t.split('"').nth(3).unwrap_or("").to_string();
            continue;
        }
        for (key, which) in [("packages", 0), ("files", 1), ("services", 2)] {
            if t.starts_with(&format!("\"{key}\"")) {
                section = Some(match which {
                    0 => &mut st.packages,
                    1 => &mut st.files,
                    _ => &mut st.services,
                });
            }
        }
        if t.starts_with('"') && !t.contains(": ") {
            if let Some(set) = section.as_deref_mut() {
                let v = t.trim_end_matches(',').trim_matches('"');
                if !v.is_empty() {
                    set.insert(v.replace("\\\"", "\"").replace("\\\\", "\\"));
                }
            }
        }
    }
    st
}

pub fn state_load() -> State {
    load()
}

pub fn load() -> State {
    std::fs::read_to_string(path())
        .map(|t| decode(&t))
        .unwrap_or_default()
}

pub fn save(s: &State) -> Result<()> {
    let p = path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|source| Error::Io {
            path: dir.to_path_buf(),
            source,
        })?;
    }
    crate::write::write(Path::new(&p), &encode(s), "root", "root", 0o644)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_survives_a_round_trip() {
        let mut s = State {
            host: "frodo".into(),
            ..Default::default()
        };
        s.packages.insert("firefox".into());
        s.packages.insert("ghostty".into());
        s.files.insert("/etc/pacman.conf".into());
        s.services.insert("greetd.service".into());

        let back = decode(&encode(&s));
        assert_eq!(back.host, "frodo");
        assert_eq!(back.packages, s.packages);
        assert_eq!(back.files, s.files);
        assert_eq!(back.services, s.services);
    }

    #[test]
    fn a_missing_state_file_is_simply_empty() {
        // A fresh machine has no state, and that must not be an error --
        // it just means nothing has been managed yet.
        let s = decode("");
        assert!(s.packages.is_empty());
    }
}
