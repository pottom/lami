//! Reading the config and resolving a host profile.
//!
//! Two kinds of file:
//!   hosts/<name>.kdl        a machine: which layers it gets, with what params
//!   layers/<name>/layer.kdl a layer: packages, services, files, hooks
//!
//! Every declaration keeps its source location (file + line) so that
//! `lami why` can say where a resource comes from.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use kdl::{KdlDocument, KdlNode, KdlValue};
use miette::NamedSource;

use crate::error::{Error, Result};

/// Where a declaration came from. This is what `lami why` reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub file: PathBuf,
    pub line: usize,
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.file.display(), self.line)
    }
}

/// A host profile parameter. Deliberately few kinds: the config should be
/// readable, not expressive.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Bool(bool),
    List(Vec<String>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_list(&self) -> Vec<String> {
        match self {
            Value::List(v) => v.clone(),
            Value::Str(s) => vec![s.clone()],
            _ => vec![],
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Str(s) => write!(f, "{s}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Bool(true) => write!(f, "on"),
            Value::Bool(false) => write!(f, "off"),
            Value::List(v) => write!(f, "{}", v.join(", ")),
        }
    }
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Host {
    pub name: String,
    pub description: Option<String>,
    pub layers: Vec<String>,
    pub params: BTreeMap<String, Value>,
    pub origin: PathBuf,
}

impl Host {
    pub fn param(&self, key: &str) -> Option<&Value> {
        self.params.get(key)
    }
    pub fn param_str(&self, key: &str) -> Option<&str> {
        self.param(key).and_then(Value::as_str)
    }
}

/// A single declaration together with where it was written.
#[derive(Debug, Clone)]
pub struct Decl {
    pub name: String,
    pub origin: Origin,
    /// Set when declared inside a conditional block, e.g. `gpu=intel`.
    pub condition: Option<Condition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub key: String,
    pub value: String,
}

impl std::fmt::Display for Condition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}={}", self.key, self.value)
    }
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub name: String,
    pub description: Option<String>,
    pub needs: Vec<String>,
    pub packages: Vec<Decl>,
    pub services: Vec<Decl>,
    pub path: PathBuf,
}

// ---------------------------------------------------------------------------

fn line_of(src: &str, offset: usize) -> usize {
    src.get(..offset).map_or(1, |s| s.matches('\n').count() + 1)
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn named(path: &Path, src: &str) -> NamedSource<String> {
    NamedSource::new(path.display().to_string(), src.to_string()).with_language("kdl")
}

/// A node's arguments as strings. `layers "a" "b"` -> ["a", "b"]
fn args(node: &KdlNode) -> Vec<String> {
    node.entries()
        .iter()
        .filter(|e| e.name().is_none())
        .filter_map(|e| match e.value() {
            KdlValue::String(s) => Some(s.clone()),
            KdlValue::Integer(i) => Some(i.to_string()),
            KdlValue::Bool(b) => Some(b.to_string()),
            _ => None,
        })
        .collect()
}

/// A node's single argument as a value.
fn value_of(node: &KdlNode) -> Option<Value> {
    let a: Vec<&KdlValue> = node
        .entries()
        .iter()
        .filter(|e| e.name().is_none())
        .map(|e| e.value())
        .collect();
    match a.len() {
        // A node with no arguments means enabled. Shorthand for `on`.
        0 => Some(Value::Bool(true)),
        1 => Some(match a[0] {
            // In KDL v2 a bare `true` is no longer a keyword; you must write
            // `#true`. But `ddc #true` reads worse than `ddc on`, so on/off is
            // the documented spelling. `#true`/`#false` still work because
            // that is KDL's native form.
            //
            // Why have `off` at all, when omitting the line would also disable
            // it: because `off` DOCUMENTS ITSELF. A missing line does not tell
            // you whether the choice was considered or simply forgotten.
            KdlValue::String(s) if s == "on" => Value::Bool(true),
            KdlValue::String(s) if s == "off" => Value::Bool(false),
            KdlValue::String(s) => Value::Str(s.clone()),
            KdlValue::Integer(i) => Value::Int(*i as i64),
            KdlValue::Bool(b) => Value::Bool(*b),
            other => Value::Str(other.to_string()),
        }),
        _ => Some(Value::List(args(node))),
    }
}

/// Both `packages { foo; bar }` and `packages "foo" "bar"` are accepted.
/// The block form is preferred because it allows a comment per line:
///
/// ```kdl
/// packages {
///     firefox   // needed for work SSO
/// }
/// ```
fn names_from(node: &KdlNode, src: &str, file: &Path, cond: Option<&Condition>) -> Vec<Decl> {
    let mut out: Vec<Decl> = args(node)
        .into_iter()
        .map(|name| Decl {
            name,
            origin: Origin {
                file: file.to_path_buf(),
                line: line_of(src, node.span().offset()),
            },
            condition: cond.cloned(),
        })
        .collect();

    if let Some(children) = node.children() {
        for child in children.nodes() {
            out.push(Decl {
                name: child.name().value().to_string(),
                origin: Origin {
                    file: file.to_path_buf(),
                    line: line_of(src, child.span().offset()),
                },
                condition: cond.cloned(),
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------

impl Host {
    fn parse(path: &Path) -> Result<Host> {
        let src = read(path)?;
        let doc: KdlDocument = src.parse()?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        let mut description = None;
        let mut layers = Vec::new();
        let mut params = BTreeMap::new();

        for node in doc.nodes() {
            match node.name().value() {
                "description" => description = value_of(node).map(|v| v.to_string()),
                "layers" => layers = args(node),
                key => {
                    if let Some(v) = value_of(node) {
                        params.insert(key.to_string(), v);
                    }
                }
            }
        }

        if layers.is_empty() {
            return Err(Error::Config {
                msg: format!("host '{name}' declares no layers"),
                src: named(path, &src),
                span: (0, src.len().min(1)).into(),
                label: "the `layers` line is missing".into(),
            });
        }

        Ok(Host {
            name,
            description,
            layers,
            params,
            origin: path.to_path_buf(),
        })
    }
}

impl Layer {
    fn parse(name: &str, path: &Path) -> Result<Layer> {
        let src = read(path)?;
        let doc: KdlDocument = src.parse()?;

        let mut layer = Layer {
            name: name.to_string(),
            description: None,
            needs: Vec::new(),
            packages: Vec::new(),
            services: Vec::new(),
            path: path.to_path_buf(),
        };

        collect(&doc, &src, path, None, &mut layer)?;
        Ok(layer)
    }
}

/// Collects declarations recursively. Descending into a `when` block carries
/// the condition along, so that `why` can explain WHY a package applies.
fn collect(
    doc: &KdlDocument,
    src: &str,
    path: &Path,
    cond: Option<Condition>,
    layer: &mut Layer,
) -> Result<()> {
    for node in doc.nodes() {
        match node.name().value() {
            "description" if cond.is_none() => {
                layer.description = value_of(node).map(|v| v.to_string())
            }
            "needs" if cond.is_none() => layer.needs.extend(args(node)),
            // There is no separate `aur` block. AUR packages live in the same
            // `packages` list: paru decides for itself what comes from a repo
            // and what from the AUR, so you need not track it while writing
            // config. The distinction only matters for display, and `check`
            // derives it from pacman's sync database.
            "packages" => layer
                .packages
                .extend(names_from(node, src, path, cond.as_ref())),
            "services" => layer
                .services
                .extend(names_from(node, src, path, cond.as_ref())),
            "when" => {
                // `when gpu="nvidia" { ... }` -- exactly one property
                let props: Vec<(&str, &KdlValue)> = node
                    .entries()
                    .iter()
                    .filter_map(|e| e.name().map(|n| (n.value(), e.value())))
                    .collect();

                if props.len() != 1 {
                    return Err(Error::Config {
                        msg: "`when` takes exactly one condition".into(),
                        src: named(path, src),
                        span: (node.span().offset(), node.span().len()).into(),
                        label: "e.g. `when gpu=\"nvidia\" { ... }`".into(),
                    });
                }
                let (key, val) = props[0];
                let inner = Condition {
                    key: key.to_string(),
                    value: match val {
                        KdlValue::String(s) => s.clone(),
                        other => other.to_string(),
                    },
                };
                if let Some(children) = node.children() {
                    collect(children, src, path, Some(inner), layer)?;
                }
            }
            _ => { /* file / on-change: next milestone */ }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------

/// The whole parsed config: every host and every layer.
#[derive(Debug)]
pub struct Config {
    pub dir: PathBuf,
    pub hosts: BTreeMap<String, Host>,
    pub layers: BTreeMap<String, Layer>,
}

impl Config {
    pub fn load(dir: &Path) -> Result<Config> {
        let mut hosts = BTreeMap::new();
        let hosts_dir = dir.join("hosts");
        if hosts_dir.is_dir() {
            let mut entries: Vec<PathBuf> = fs::read_dir(&hosts_dir)
                .map_err(|source| Error::Io {
                    path: hosts_dir.clone(),
                    source,
                })?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|e| e == "kdl"))
                .collect();
            entries.sort();
            for p in entries {
                let h = Host::parse(&p)?;
                hosts.insert(h.name.clone(), h);
            }
        }

        let mut layers = BTreeMap::new();
        let layers_dir = dir.join("layers");
        if layers_dir.is_dir() {
            let mut entries: Vec<PathBuf> = fs::read_dir(&layers_dir)
                .map_err(|source| Error::Io {
                    path: layers_dir.clone(),
                    source,
                })?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.is_dir())
                .collect();
            entries.sort();
            for p in entries {
                let file = p.join("layer.kdl");
                if !file.is_file() {
                    continue;
                }
                let name = p.file_name().and_then(|s| s.to_str()).unwrap_or_default();
                let l = Layer::parse(name, &file)?;
                layers.insert(l.name.clone(), l);
            }
        }

        Ok(Config {
            dir: dir.to_path_buf(),
            hosts,
            layers,
        })
    }

    /// Resolve a host: its `layers` list plus the transitive closure of `needs`.
    ///
    /// An unknown host name is an ERROR, never a default profile. A typo'd
    /// hostname or a fresh VM must not silently receive the wrong config.
    pub fn resolve(&self, name: &str) -> Result<Resolved<'_>> {
        let host = self.hosts.get(name).ok_or_else(|| Error::UnknownHost {
            name: name.to_string(),
            known: if self.hosts.is_empty() {
                "(none)".into()
            } else {
                self.hosts.keys().cloned().collect::<Vec<_>>().join(", ")
            },
        })?;

        let mut ordered: Vec<String> = Vec::new();
        for l in &host.layers {
            self.expand(l, host, &mut ordered)?;
        }

        Ok(Resolved {
            host,
            layers: ordered
                .iter()
                .map(|n| self.layers.get(n).expect("checked during expand"))
                .collect(),
        })
    }

    fn expand(&self, name: &str, host: &Host, out: &mut Vec<String>) -> Result<()> {
        if out.iter().any(|n| n == name) {
            return Ok(());
        }
        let layer = self.layers.get(name).ok_or_else(|| {
            let src = fs::read_to_string(&host.origin).unwrap_or_default();
            let off = src.find(name).unwrap_or(0);
            Error::UnknownLayer {
                layer: name.to_string(),
                src: named(&host.origin, &src),
                span: (off, name.len()).into(),
            }
        })?;
        // Dependencies first, so the order is deterministic.
        for dep in &layer.needs {
            self.expand(dep, host, out)?;
        }
        if !out.iter().any(|n| n == name) {
            out.push(name.to_string());
        }
        Ok(())
    }
}

/// The config resolved for one specific host.
#[derive(Debug)]
pub struct Resolved<'a> {
    pub host: &'a Host,
    pub layers: Vec<&'a Layer>,
}

impl Resolved<'_> {
    /// Whether the condition holds for this host.
    fn matches(&self, cond: &Option<Condition>) -> bool {
        match cond {
            None => true,
            Some(c) => self.host.param(&c.key).is_some_and(|v| match v {
                // Normalize booleans so that `when ddc=on` and
                // `when ddc=#true` mean the same thing.
                Value::Bool(b) => matches!(
                    (b, c.value.as_str()),
                    (true, "on" | "true" | "yes") | (false, "off" | "false" | "no")
                ),
                other => other.to_string() == c.value,
            }),
        }
    }

    /// The packages that actually apply to this host, in layer order.
    pub fn packages(&self) -> Vec<(&Layer, &Decl)> {
        self.select(|l| &l.packages)
    }
    pub fn services(&self) -> Vec<(&Layer, &Decl)> {
        self.select(|l| &l.services)
    }

    fn select<'s, F>(&'s self, f: F) -> Vec<(&'s Layer, &'s Decl)>
    where
        F: Fn(&'s Layer) -> &'s Vec<Decl>,
    {
        let mut out = Vec::new();
        for layer in &self.layers {
            for d in f(layer) {
                if self.matches(&d.condition) {
                    out.push((*layer, d));
                }
            }
        }
        out
    }
}
