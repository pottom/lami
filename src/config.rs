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

use crate::error::{ConfigError, Error, Result, UnknownLayerError};

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

/// Which systemd instance a unit belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    System,
    /// The invoking user's own systemd instance.
    User,
}

/// What a unit's state should be.
///
/// A bare name means `Enabled`, because that is what a declaration almost
/// always means. `Disabled` is worth writing even though leaving the line out
/// would also leave it off: it says the choice was made rather than forgotten,
/// and it makes lami actively turn it off rather than merely ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitState {
    Enabled,
    Disabled,
    /// Symlinked to /dev/null: cannot be started even as a dependency.
    Masked,
}

impl std::fmt::Display for UnitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnitState::Enabled => write!(f, "enabled"),
            UnitState::Disabled => write!(f, "disabled"),
            UnitState::Masked => write!(f, "masked"),
        }
    }
}

/// A systemd unit this host should have in a particular state.
#[derive(Debug, Clone)]
pub struct UnitDecl {
    pub name: String,
    pub state: UnitState,
    pub scope: Scope,
    /// Restart this unit when lami rewrites its own unit file or a drop-in for
    /// it. Deliberately narrow: anything else belongs in an `on-change` hook,
    /// where the consequence is written out rather than implied.
    pub restart_on_change: bool,
    pub origin: Origin,
    pub condition: Option<Condition>,
}

/// Where a managed file's content comes from.
#[derive(Debug, Clone)]
pub enum Source {
    /// A file inside the layer directory: `file "/etc/foo" from="foo.conf"`
    From(PathBuf),
    /// Inline content: `file "/etc/foo" { text "..." }`
    ///
    /// KDL v2 multi-line strings dedent automatically, so the indentation used
    /// to keep the config readable does not leak into the rendered file.
    Text(String),
}

/// A file this host should have.
#[derive(Debug, Clone)]
pub struct FileDecl {
    /// Absolute target path, or `~/...` for the user's home.
    pub path: String,
    pub source: Source,
    /// Explicit overrides. Left unset, ownership and mode are inferred from
    /// the path -- see the `perms` module.
    pub owner: Option<String>,
    pub group: Option<String>,
    pub mode: Option<u32>,
    pub origin: Origin,
    pub condition: Option<Condition>,
}

/// A command to run when one of the watched paths changes.
#[derive(Debug, Clone)]
pub struct Hook {
    pub watch: Vec<String>,
    pub run: String,
    pub origin: Origin,
    pub condition: Option<Condition>,
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub name: String,
    pub description: Option<String>,
    pub needs: Vec<String>,
    pub packages: Vec<Decl>,
    pub services: Vec<UnitDecl>,
    pub files: Vec<FileDecl>,
    pub hooks: Vec<Hook>,
    /// The layer's own directory, which `from=` paths are relative to.
    pub dir: PathBuf,
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

/// A node's value.
///
/// A child block is always a list, even with one entry:
///
/// ```kdl
/// monitors {
///     "DP-1, 2560x1440@144, 0x0, 1"
/// }
/// ```
///
/// This matters because KDL cannot otherwise tell one string from a list of
/// one, and a template looping over the scalar form silently produces nothing.
/// Writing a list as a block is also how `packages` and `services` already
/// read, so there is one rule rather than two.
fn value_of(node: &KdlNode) -> Option<Value> {
    if let Some(children) = node.children() {
        return Some(Value::List(
            children
                .nodes()
                .iter()
                .map(|n| n.name().value().to_string())
                .collect(),
        ));
    }

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
            return Err(Error::Config(Box::new(ConfigError {
                msg: format!("host '{name}' declares no layers"),
                src: named(path, &src),
                span: (0, src.len().min(1)).into(),
                label: "the `layers` line is missing".into(),
            })));
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
            files: Vec::new(),
            hooks: Vec::new(),
            dir: path.parent().unwrap_or(Path::new(".")).to_path_buf(),
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
            "services" => {
                layer
                    .services
                    .extend(units_from(node, src, path, cond.as_ref(), Scope::System)?)
            }
            "user-services" => {
                layer
                    .services
                    .extend(units_from(node, src, path, cond.as_ref(), Scope::User)?)
            }
            "when" => {
                // `when gpu="nvidia" { ... }` -- exactly one property
                let props: Vec<(&str, &KdlValue)> = node
                    .entries()
                    .iter()
                    .filter_map(|e| e.name().map(|n| (n.value(), e.value())))
                    .collect();

                if props.len() != 1 {
                    return Err(Error::Config(Box::new(ConfigError {
                        msg: "`when` takes exactly one condition".into(),
                        src: named(path, src),
                        span: (node.span().offset(), node.span().len()).into(),
                        label: "e.g. `when gpu=\"nvidia\" { ... }`".into(),
                    })));
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
            "file" => {
                let decl = parse_file(node, src, path, cond.as_ref(), &layer.dir)?;
                layer.files.push(decl);
            }
            "dir" => {
                for decl in parse_dir(node, src, path, cond.as_ref(), &layer.dir)? {
                    layer.files.push(decl);
                }
            }
            "on-change" => {
                let watch = args(node);
                let run = node
                    .children()
                    .and_then(|c| {
                        c.nodes()
                            .iter()
                            .find(|n| n.name().value() == "run")
                            .and_then(|n| match n.entries().first().map(|e| e.value()) {
                                Some(KdlValue::String(s)) => Some(s.clone()),
                                _ => None,
                            })
                    })
                    .ok_or_else(|| {
                        Error::Config(Box::new(ConfigError {
                        msg: "`on-change` needs a `run` command".into(),
                        src: named(path, src),
                        span: (node.span().offset(), node.span().len()).into(),
                        label: "e.g. `on-change \"/etc/mkinitcpio.conf\" { run \"mkinitcpio -P\" }`"
                            .into(),
                    }))
                    })?;
                if watch.is_empty() {
                    return Err(Error::Config(Box::new(ConfigError {
                        msg: "`on-change` needs at least one path to watch".into(),
                        src: named(path, src),
                        span: (node.span().offset(), node.span().len()).into(),
                        label: "which file should trigger this?".into(),
                    })));
                }
                layer.hooks.push(Hook {
                    watch,
                    run,
                    origin: Origin {
                        file: path.to_path_buf(),
                        line: line_of(src, node.span().offset()),
                    },
                    condition: cond.clone(),
                });
            }
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------

/// Repository-wide settings, from an optional `config.kdl` at the root.
#[derive(Debug, Default)]
pub struct Settings {
    /// Where the age identity lives on this machine.
    pub age_identity: Option<PathBuf>,
    /// Who encrypted files are encrypted to.
    pub age_recipients: Vec<String>,
}

/// The whole parsed config: every host and every layer.
#[derive(Debug)]
pub struct Config {
    pub dir: PathBuf,
    pub hosts: BTreeMap<String, Host>,
    pub layers: BTreeMap<String, Layer>,
    pub settings: Settings,
}

/// Expand a leading `~/` against the given home directory.
pub fn expand_home(p: &str, home: &Path) -> PathBuf {
    match p.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(p),
    }
}

fn parse_settings(dir: &Path, home: &Path) -> Result<Settings> {
    let file = dir.join("config.kdl");
    if !file.is_file() {
        return Ok(Settings::default());
    }
    let src = read(&file)?;
    let doc: KdlDocument = src.parse()?;
    let mut st = Settings::default();

    for node in doc.nodes() {
        if node.name().value() != "age" {
            continue;
        }
        let Some(children) = node.children() else {
            continue;
        };
        for c in children.nodes() {
            let val = args(c).into_iter().next();
            match (c.name().value(), val) {
                ("identity", Some(v)) => st.age_identity = Some(expand_home(&v, home)),
                ("recipient", Some(v)) => st.age_recipients.push(v),
                _ => {}
            }
        }
    }
    Ok(st)
}

impl Config {
    pub fn load(dir: &Path, home: &Path) -> Result<Config> {
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
            settings: parse_settings(dir, home)?,
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
            Error::UnknownLayer(Box::new(UnknownLayerError {
                layer: name.to_string(),
                src: named(&host.origin, &src),
                span: (off, name.len()).into(),
            }))
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

    /// The files that actually apply to this host, in layer order.
    ///
    /// A later layer declaring the same path wins, which is what lets a more
    /// specific layer override a general one.
    pub fn files(&self) -> Vec<(&Layer, &FileDecl)> {
        let mut out: Vec<(&Layer, &FileDecl)> = Vec::new();
        for layer in &self.layers {
            for f in &layer.files {
                if !self.matches(&f.condition) {
                    continue;
                }
                if let Some(slot) = out.iter_mut().find(|(_, e)| e.path == f.path) {
                    *slot = (*layer, f);
                } else {
                    out.push((*layer, f));
                }
            }
        }
        out
    }

    /// The hooks that apply to this host.
    pub fn hooks(&self) -> Vec<(&Layer, &Hook)> {
        let mut out = Vec::new();
        for layer in &self.layers {
            for h in &layer.hooks {
                if self.matches(&h.condition) {
                    out.push((*layer, h));
                }
            }
        }
        out
    }

    /// The packages that actually apply to this host, in layer order.
    pub fn packages(&self) -> Vec<(&Layer, &Decl)> {
        self.select(|l| &l.packages)
    }
    pub fn services(&self) -> Vec<(&Layer, &UnitDecl)> {
        let mut out = Vec::new();
        for layer in &self.layers {
            for d in &layer.services {
                if self.matches(&d.condition) {
                    out.push((*layer, d));
                }
            }
        }
        out
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

/// `file "/etc/foo" from="foo.conf"` or `file "/etc/foo" { text "..." }`
fn parse_file(
    node: &KdlNode,
    src: &str,
    path: &Path,
    cond: Option<&Condition>,
    layer_dir: &Path,
) -> Result<FileDecl> {
    let span = (node.span().offset(), node.span().len());

    let target = args(node).into_iter().next().ok_or_else(|| {
        Error::Config(Box::new(ConfigError {
            msg: "`file` needs a target path".into(),
            src: named(path, src),
            span: span.into(),
            label: "e.g. `file \"/etc/foo.conf\" from=\"foo.conf\"`".into(),
        }))
    })?;

    let prop = |key: &str| -> Option<String> {
        node.entries()
            .iter()
            .find(|e| e.name().is_some_and(|n| n.value() == key))
            .map(|e| match e.value() {
                KdlValue::String(s) => s.clone(),
                other => other.to_string(),
            })
    };

    let from = prop("from");
    let owner = prop("owner");
    let group = prop("group");

    // Modes are written as strings so the leading zero survives: mode="0640".
    // A bare 0640 would be read as decimal by KDL and silently mean something
    // else entirely.
    let mode = match prop("mode") {
        None => None,
        Some(m) => Some(
            u32::from_str_radix(m.trim_start_matches("0o"), 8).map_err(|_| {
                Error::Config(Box::new(ConfigError {
                    msg: format!("`{m}` is not a valid octal mode"),
                    src: named(path, src),
                    span: span.into(),
                    label: "write it as a string, e.g. mode=\"0640\"".into(),
                }))
            })?,
        ),
    };

    let text = node.children().and_then(|c| {
        c.nodes()
            .iter()
            .find(|n| n.name().value() == "text")
            .and_then(|n| match n.entries().first().map(|e| e.value()) {
                Some(KdlValue::String(s)) => Some(s.clone()),
                _ => None,
            })
    });

    let source = match (from, text) {
        (Some(f), None) => Source::From(layer_dir.join(f)),
        (None, Some(t)) => Source::Text(t),
        (Some(_), Some(_)) => {
            return Err(Error::Config(Box::new(ConfigError {
                msg: format!("`{target}` declares both `from=` and `text`"),
                src: named(path, src),
                span: span.into(),
                label: "pick one".into(),
            })))
        }
        (None, None) => {
            return Err(Error::Config(Box::new(ConfigError {
                msg: format!("`{target}` has no content"),
                src: named(path, src),
                span: span.into(),
                label: "add `from=\"file\"` or a `text` child".into(),
            })))
        }
    };

    Ok(FileDecl {
        path: target,
        source,
        owner,
        group,
        mode,
        origin: Origin {
            file: path.to_path_buf(),
            line: line_of(src, node.span().offset()),
        },
        condition: cond.cloned(),
    })
}

/// Parse a `services` or `user-services` block.
///
/// ```kdl
/// services {
///     greetd                      // enabled -- the common case
///     bluetooth         disabled
///     systemd-networkd  masked
///     my-daemon         restart-on-change
/// }
/// ```
fn units_from(
    node: &KdlNode,
    src: &str,
    path: &Path,
    cond: Option<&Condition>,
    scope: Scope,
) -> Result<Vec<UnitDecl>> {
    let mut out = Vec::new();

    // The argument form, `services "greetd" "sshd"`, is still accepted for
    // short lists; every unit in it is simply enabled.
    for name in args(node) {
        out.push(UnitDecl {
            name,
            state: UnitState::Enabled,
            scope,
            restart_on_change: false,
            origin: Origin {
                file: path.to_path_buf(),
                line: line_of(src, node.span().offset()),
            },
            condition: cond.cloned(),
        });
    }

    if let Some(children) = node.children() {
        for child in children.nodes() {
            let mut state = UnitState::Enabled;
            let mut restart = false;

            for word in args(child) {
                match word.as_str() {
                    "enabled" => state = UnitState::Enabled,
                    "disabled" => state = UnitState::Disabled,
                    "masked" => state = UnitState::Masked,
                    "restart-on-change" => restart = true,
                    other => {
                        return Err(Error::Config(Box::new(ConfigError {
                            msg: format!("`{other}` is not a unit state"),
                            src: named(path, src),
                            span: (child.span().offset(), child.span().len()).into(),
                            label: "expected enabled, disabled, masked or restart-on-change".into(),
                        })))
                    }
                }
            }

            out.push(UnitDecl {
                name: child.name().value().to_string(),
                state,
                scope,
                restart_on_change: restart,
                origin: Origin {
                    file: path.to_path_buf(),
                    line: line_of(src, child.span().offset()),
                },
                condition: cond.cloned(),
            });
        }
    }
    Ok(out)
}

/// `dir "~/.config/fish" from="files/fish"`
///
/// Expands to one file declaration per file found in the source tree, which
/// keeps everything downstream -- diff, apply, capture, why -- working on
/// individual files with no special cases. A directory is a way of writing
/// many files, not a different kind of thing.
///
/// The `.tmpl` and `.age` suffixes are stripped from the target name, so
/// `files/fish/config.fish.tmpl` becomes `~/.config/fish/config.fish`.
fn parse_dir(
    node: &KdlNode,
    src: &str,
    path: &Path,
    cond: Option<&Condition>,
    layer_dir: &Path,
) -> Result<Vec<FileDecl>> {
    let span = (node.span().offset(), node.span().len());

    let target = args(node).into_iter().next().ok_or_else(|| {
        Error::Config(Box::new(ConfigError {
            msg: "`dir` needs a target path".into(),
            src: named(path, src),
            span: span.into(),
            label: "e.g. `dir \"~/.config/fish\" from=\"files/fish\"`".into(),
        }))
    })?;

    let from = node
        .entries()
        .iter()
        .find(|e| e.name().is_some_and(|n| n.value() == "from"))
        .and_then(|e| match e.value() {
            KdlValue::String(s) => Some(s.clone()),
            _ => None,
        })
        .ok_or_else(|| {
            Error::Config(Box::new(ConfigError {
                msg: format!("`{target}` has no source directory"),
                src: named(path, src),
                span: span.into(),
                label: "add `from=\"files/...\"`".into(),
            }))
        })?;

    let root = layer_dir.join(&from);
    if !root.is_dir() {
        return Err(Error::Config(Box::new(ConfigError {
            msg: format!("{} is not a directory", root.display()),
            src: named(path, src),
            span: span.into(),
            label: "`dir` copies a tree; use `file` for a single file".into(),
        })));
    }

    let line = line_of(src, node.span().offset());
    let mut out = Vec::new();
    walk(&root, &root, &target, path, line, cond, &mut out)?;
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

fn walk(
    root: &Path,
    dir: &Path,
    target_root: &str,
    origin_file: &Path,
    line: usize,
    cond: Option<&Condition>,
    out: &mut Vec<FileDecl>,
) -> Result<()> {
    let entries = fs::read_dir(dir).map_err(|source| Error::Io {
        path: dir.to_path_buf(),
        source,
    })?;

    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(root, &p, target_root, origin_file, line, cond, out)?;
            continue;
        }
        let rel = p.strip_prefix(root).unwrap_or(&p).to_string_lossy();
        // The suffix says how to process the source; it is not part of the
        // name the file should have on disk.
        let rel = rel
            .strip_suffix(".tmpl")
            .or_else(|| rel.strip_suffix(".age"))
            .unwrap_or(&rel)
            .to_string();

        out.push(FileDecl {
            path: format!("{}/{}", target_root.trim_end_matches('/'), rel),
            source: Source::From(p),
            owner: None,
            group: None,
            mode: None,
            origin: Origin {
                file: origin_file.to_path_buf(),
                line,
            },
            condition: cond.cloned(),
        });
    }
    Ok(())
}
