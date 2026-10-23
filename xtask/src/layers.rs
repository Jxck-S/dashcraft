//! `cargo xtask layers`: crate dependency direction.
//!
//! Every workspace crate belongs to one layer ([`LAYERS`]) and may depend only on crates in
//! lower layers. An edge between two crates of the same layer must be listed in
//! [`INTRA_LAYER`]; an edge to a higher layer must be listed in [`EXCEPTIONS`] (empty, and it
//! should stay that way: move orchestration up into `pbi-session` or `pbi-ops` instead).
//! Crates other than [`UI_CRATES`] must not reach a UI-toolkit crate ([`UI_TOOLKIT`]) through
//! any chain of dependencies, and [`ISOLATED`] crates have no workspace dependencies at all.
//! HTTP/TLS clients and the OS keychain ([`NETWORK_CLIENT`]) may only be reached by the
//! [`NETWORK_CRATES`] (the opt-in AI layer), crates that depend on them, and the
//! [`NETWORK_EXEMPT`] crates: everything else stays offline by construction.
//!
//! Normal, build and dev dependencies are checked, for every target platform and with all
//! features enabled (`cargo metadata --all-features`). A dependency's own dev-dependencies are
//! not followed (they are never built for the dependent).

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::util;

/// One architecture layer. Lower `level`s are closer to the model.
#[derive(Debug, Clone, Copy)]
pub struct Layer {
    pub level: u8,
    pub name: &'static str,
    pub crates: &'static [&'static str],
}

/// An allowed edge that the level rule alone would reject.
#[derive(Debug, Clone, Copy)]
pub struct Edge {
    pub from: &'static str,
    pub to: &'static str,
    pub why: &'static str,
}

/// The layers, bottom up. Taken from the dependency graph in `docs/ARCHITECTURE.md`; a crate
/// listed here but absent from the workspace is fine (reported as "not in the workspace").
pub const LAYERS: &[Layer] = &[
    Layer {
        level: 0,
        name: "model",
        crates: &["pbi-model"],
    },
    Layer {
        level: 1,
        name: "formats, M, storage",
        crates: &[
            "pbi-tmdl",
            "pbi-tmsl",
            "pbi-pbir",
            "pbi-m",
            "pbi-vertipaq",
            "pbi-engine",
        ],
    },
    Layer {
        level: 2,
        name: "DAX, readers",
        crates: &["pbi-dax", "pbi-core"],
    },
    Layer {
        level: 3,
        name: "session",
        crates: &["pbi-session"],
    },
    Layer {
        level: 4,
        name: "render",
        crates: &["pbi-render"],
    },
    Layer {
        level: 5,
        name: "operations",
        crates: &["pbi-ops", "pbi-ai"],
    },
    Layer {
        level: 6,
        name: "front ends",
        crates: &["pbiview-cli", "pbiview-app"],
    },
];

/// Allowed edges inside one layer.
pub const INTRA_LAYER: &[Edge] = &[
    Edge {
        from: "pbi-core",
        to: "pbi-dax",
        why: "the linter and mashup readers parse DAX",
    },
    Edge {
        from: "pbi-ai",
        to: "pbi-ops",
        why: "registers the opt-in ai.* commands into the command registry",
    },
];

/// Allowed upward edges. Keep empty.
pub const EXCEPTIONS: &[Edge] = &[];

/// Build tooling outside the layers: may use any crate, and no crate may depend on it.
pub const TOOLING: &[&str] = &["xtask"];

/// The only crates allowed to use a UI toolkit.
pub const UI_CRATES: &[&str] = &["pbiview-app"];

/// UI-toolkit and webview crates (`*` matches any suffix).
pub const UI_TOOLKIT: &[&str] = &[
    "tauri*",
    "wry",
    "tao",
    "specta*",
    "egui*",
    "eframe",
    "winit",
    "muda",
    "rfd",
    "tray-icon",
    "webkit2gtk*",
    "webview2-com*",
    "gtk",
];

/// Crates without any workspace dependency (`pbi-m` is fuzzed on its own).
pub const ISOLATED: &[&str] = &["pbi-model", "pbi-m"];

/// The only crates that may use an HTTP/TLS client or the OS keychain; crates that depend on
/// them (the front ends that opt in) may too.
pub const NETWORK_CRATES: &[&str] = &["pbi-ai"];

/// HTTP/TLS client and keychain crates (`*` matches any suffix, or any prefix when leading).
pub const NETWORK_CLIENT: &[&str] = &[
    "ureq*",
    "reqwest*",
    "hyper*",
    "h2",
    "rustls*",
    "tokio-rustls",
    "native-tls",
    "tokio-native-tls",
    "openssl*",
    "webpki-roots",
    "isahc",
    "curl*",
    "attohttpc",
    "surf",
    "keyring*",
    "*-keyring-store",
];

/// A crate allowed to reach [`NETWORK_CLIENT`] crates without depending on a
/// [`NETWORK_CRATES`] crate.
#[derive(Debug, Clone, Copy)]
pub struct Exempt {
    pub krate: &'static str,
    pub why: &'static str,
}

/// Exemptions from the network rule (checked with `cargo tree -e normal -i <crate>`).
pub const NETWORK_EXEMPT: &[Exempt] = &[Exempt {
    krate: "pbiview-app",
    why: "Tauri 2 depends on reqwest and hyper (without a TLS backend)",
}];

/// The rule set [`check`] applies.
#[derive(Debug, Clone, Copy)]
pub struct Rules<'a> {
    pub layers: &'a [Layer],
    pub intra_layer: &'a [Edge],
    pub exceptions: &'a [Edge],
    pub tooling: &'a [&'a str],
    pub ui_crates: &'a [&'a str],
    pub ui_toolkit: &'a [&'a str],
    pub isolated: &'a [&'a str],
    pub network_crates: &'a [&'a str],
    pub network_client: &'a [&'a str],
    pub network_exempt: &'a [Exempt],
}

impl Rules<'static> {
    pub const CURRENT: Rules<'static> = Rules {
        layers: LAYERS,
        intra_layer: INTRA_LAYER,
        exceptions: EXCEPTIONS,
        tooling: TOOLING,
        ui_crates: UI_CRATES,
        ui_toolkit: UI_TOOLKIT,
        isolated: ISOLATED,
        network_crates: NETWORK_CRATES,
        network_client: NETWORK_CLIENT,
        network_exempt: NETWORK_EXEMPT,
    };
}

/// `name` matches `pattern`: exact, `prefix*` or `*suffix`.
fn matches_pattern(pattern: &str, name: &str) -> bool {
    if let Some(prefix) = pattern.strip_suffix('*') {
        name.starts_with(prefix)
    } else if let Some(suffix) = pattern.strip_prefix('*') {
        name.ends_with(suffix)
    } else {
        name == pattern
    }
}

impl Rules<'_> {
    pub fn level_of(&self, krate: &str) -> Option<u8> {
        self.layers
            .iter()
            .find(|l| l.crates.contains(&krate))
            .map(|l| l.level)
    }

    fn listed(edges: &[Edge], from: &str, to: &str) -> bool {
        edges.iter().any(|e| e.from == from && e.to == to)
    }

    fn is_ui_toolkit(&self, name: &str) -> bool {
        self.ui_toolkit.iter().any(|p| matches_pattern(p, name))
    }

    pub fn is_network_client(&self, name: &str) -> bool {
        self.network_client.iter().any(|p| matches_pattern(p, name))
    }

    /// Inconsistencies in the rules themselves (duplicate crates, allow-list entries that
    /// are not intra-layer or not upward).
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = BTreeSet::new();
        for krate in self.layers.iter().flat_map(|l| l.crates) {
            if !seen.insert(*krate) {
                out.push(format!("`{krate}` is in more than one layer"));
            }
            if self.tooling.contains(krate) {
                out.push(format!("`{krate}` is both tooling and in a layer"));
            }
        }
        for e in self.intra_layer {
            match (self.level_of(e.from), self.level_of(e.to)) {
                (Some(a), Some(b)) if a == b => {}
                _ => out.push(format!(
                    "intra-layer entry {} → {} is not inside one layer",
                    e.from, e.to
                )),
            }
        }
        for e in self.exceptions {
            match (self.level_of(e.from), self.level_of(e.to)) {
                (Some(a), Some(b)) if a < b => {}
                _ => out.push(format!(
                    "exception {} → {} is not an upward edge",
                    e.from, e.to
                )),
            }
        }
        for krate in self
            .network_crates
            .iter()
            .chain(self.network_exempt.iter().map(|e| &e.krate))
        {
            if self.level_of(krate).is_none() {
                out.push(format!("network crate `{krate}` is in no layer"));
            }
        }
        out
    }
}

/// Kind of a dependency edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum DepKind {
    Normal,
    Build,
    Dev,
}

impl fmt::Display for DepKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            DepKind::Normal => "normal",
            DepKind::Build => "build",
            DepKind::Dev => "dev",
        })
    }
}

/// A resolved package and its direct dependencies (indexes into [`Graph::nodes`]).
#[derive(Debug, Clone, Default)]
pub struct Node {
    pub name: String,
    pub deps: Vec<(usize, BTreeSet<DepKind>)>,
}

/// The resolved dependency graph of the workspace.
#[derive(Debug, Clone, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    /// Indexes of the workspace members.
    pub members: Vec<usize>,
}

impl Graph {
    /// Builds the graph from `cargo metadata --format-version 1` output (with `resolve`).
    pub fn from_metadata(meta: &Value) -> Result<Self> {
        let packages = meta["packages"]
            .as_array()
            .context("metadata without `packages`")?;
        let mut index: HashMap<&str, usize> = HashMap::new();
        let mut nodes = Vec::with_capacity(packages.len());
        for p in packages {
            let id = p["id"].as_str().context("package without `id`")?;
            let name = p["name"].as_str().context("package without `name`")?;
            index.insert(id, nodes.len());
            nodes.push(Node {
                name: name.to_owned(),
                deps: Vec::new(),
            });
        }
        let lookup = |id: &str| {
            index
                .get(id)
                .copied()
                .with_context(|| format!("unknown package id `{id}`"))
        };
        let resolve = meta["resolve"]["nodes"]
            .as_array()
            .context("metadata without `resolve` (do not pass --no-deps)")?;
        for n in resolve {
            let from = lookup(n["id"].as_str().context("resolve node without `id`")?)?;
            let mut deps = Vec::new();
            for d in n["deps"].as_array().map(Vec::as_slice).unwrap_or_default() {
                let to = lookup(d["pkg"].as_str().context("dependency without `pkg`")?)?;
                let kinds = d["dep_kinds"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .map(|k| match k["kind"].as_str() {
                        Some("dev") => DepKind::Dev,
                        Some("build") => DepKind::Build,
                        _ => DepKind::Normal,
                    })
                    .collect();
                deps.push((to, kinds));
            }
            if let Some(node) = nodes.get_mut(from) {
                node.deps = deps;
            }
        }
        let members = meta["workspace_members"]
            .as_array()
            .context("metadata without `workspace_members`")?
            .iter()
            .map(|m| lookup(m.as_str().unwrap_or_default()))
            .collect::<Result<_>>()?;
        Ok(Self { nodes, members })
    }

    fn name(&self, i: usize) -> &str {
        self.nodes.get(i).map_or("?", |n| n.name.as_str())
    }

    fn deps(&self, i: usize) -> &[(usize, BTreeSet<DepKind>)] {
        self.nodes.get(i).map_or(&[], |n| n.deps.as_slice())
    }

    /// Shortest dependency path from member `start` to a package matching `target`: all
    /// edge kinds from `start` itself, normal and build edges below it.
    fn path_to(&self, start: usize, target: impl Fn(&str) -> bool) -> Option<Vec<String>> {
        let mut parent: HashMap<usize, usize> = HashMap::new();
        let mut queue = VecDeque::from([start]);
        parent.insert(start, start);
        while let Some(i) = queue.pop_front() {
            for (to, kinds) in self.deps(i) {
                let followed = i == start
                    || kinds.contains(&DepKind::Normal)
                    || kinds.contains(&DepKind::Build);
                if !followed || parent.contains_key(to) {
                    continue;
                }
                parent.insert(*to, i);
                if target(self.name(*to)) {
                    let mut path = vec![self.name(*to).to_owned()];
                    let mut at = *to;
                    while at != start {
                        at = parent.get(&at).copied().unwrap_or(start);
                        path.push(self.name(at).to_owned());
                    }
                    path.reverse();
                    return Some(path);
                }
                queue.push_back(*to);
            }
        }
        None
    }
}

/// A broken rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// A workspace crate in no layer and not tooling.
    Unassigned { krate: String },
    /// A dependency on a higher layer.
    Upward {
        from: String,
        from_level: u8,
        to: String,
        to_level: u8,
        kinds: Vec<DepKind>,
    },
    /// A dependency inside one layer that is not in the intra-layer allow-list.
    Sideways {
        from: String,
        to: String,
        level: u8,
        kinds: Vec<DepKind>,
    },
    /// A dependency on build tooling.
    OnTooling {
        from: String,
        to: String,
        kinds: Vec<DepKind>,
    },
    /// An isolated crate with a workspace dependency.
    NotIsolated {
        krate: String,
        dep: String,
        kinds: Vec<DepKind>,
    },
    /// A non-UI crate that reaches a UI-toolkit crate (path from the crate to the toolkit).
    UiLeak { krate: String, path: Vec<String> },
    /// A crate outside the network crates and their dependents that reaches an HTTP/TLS
    /// client or keychain crate (path from the crate to it).
    NetworkLeak { krate: String, path: Vec<String> },
}

fn kinds_text(kinds: &[DepKind]) -> String {
    kinds
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("+")
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Violation::Unassigned { krate } => write!(
                f,
                "`{krate}` is in no layer: add it to LAYERS (or TOOLING) in xtask/src/layers.rs"
            ),
            Violation::Upward {
                from,
                from_level,
                to,
                to_level,
                kinds,
            } => write!(
                f,
                "upward dependency {from} (L{from_level}) → {to} (L{to_level}) [{}]",
                kinds_text(kinds)
            ),
            Violation::Sideways {
                from,
                to,
                level,
                kinds,
            } => write!(
                f,
                "same-layer dependency {from} → {to} (both L{level}) [{}] is not in INTRA_LAYER",
                kinds_text(kinds)
            ),
            Violation::OnTooling { from, to, kinds } => write!(
                f,
                "{from} depends on build tooling `{to}` [{}]",
                kinds_text(kinds)
            ),
            Violation::NotIsolated { krate, dep, kinds } => write!(
                f,
                "`{krate}` must have no workspace dependencies but uses `{dep}` [{}]",
                kinds_text(kinds)
            ),
            Violation::UiLeak { krate, path } => {
                write!(f, "`{krate}` reaches a UI toolkit: {}", path.join(" → "))
            }
            Violation::NetworkLeak { krate, path } => write!(
                f,
                "`{krate}` reaches an HTTP/TLS client or the OS keychain: {} (only the network \
                 crates in NETWORK_CRATES and crates that depend on them may)",
                path.join(" → ")
            ),
        }
    }
}

/// Result of [`check`].
#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub violations: Vec<Violation>,
    /// Workspace crates with their level (`None` for tooling).
    pub crates: Vec<(String, Option<u8>)>,
    /// Workspace-internal `(from, to)` edges checked.
    pub edges: usize,
    /// Layer crates that are not in the workspace (yet).
    pub absent: Vec<String>,
}

/// Applies `rules` to `graph`.
pub fn check(graph: &Graph, rules: &Rules<'_>) -> Outcome {
    let mut out = Outcome::default();
    let member_names: BTreeSet<&str> = graph.members.iter().map(|&m| graph.name(m)).collect();
    for &m in &graph.members {
        let name = graph.name(m);
        let level = rules.level_of(name);
        let tooling = rules.tooling.contains(&name);
        out.crates.push((name.to_owned(), level));
        if level.is_none() && !tooling {
            out.violations.push(Violation::Unassigned {
                krate: name.to_owned(),
            });
        }
        for (to, kinds) in graph.deps(m) {
            let dep = graph.name(*to);
            if !member_names.contains(dep) {
                continue;
            }
            out.edges += 1;
            let kinds: Vec<DepKind> = kinds.iter().copied().collect();
            if rules.tooling.contains(&dep) {
                out.violations.push(Violation::OnTooling {
                    from: name.to_owned(),
                    to: dep.to_owned(),
                    kinds: kinds.clone(),
                });
                continue;
            }
            if rules.isolated.contains(&name) {
                out.violations.push(Violation::NotIsolated {
                    krate: name.to_owned(),
                    dep: dep.to_owned(),
                    kinds: kinds.clone(),
                });
            }
            let (Some(from_level), Some(to_level)) = (level, rules.level_of(dep)) else {
                continue;
            };
            if from_level < to_level && !Rules::listed(rules.exceptions, name, dep) {
                out.violations.push(Violation::Upward {
                    from: name.to_owned(),
                    from_level,
                    to: dep.to_owned(),
                    to_level,
                    kinds,
                });
            } else if from_level == to_level
                && !Rules::listed(rules.intra_layer, name, dep)
                && !Rules::listed(rules.exceptions, name, dep)
            {
                out.violations.push(Violation::Sideways {
                    from: name.to_owned(),
                    to: dep.to_owned(),
                    level: from_level,
                    kinds,
                });
            }
        }
        if !rules.ui_crates.contains(&name)
            && let Some(path) = graph.path_to(m, |n| rules.is_ui_toolkit(n))
        {
            out.violations.push(Violation::UiLeak {
                krate: name.to_owned(),
                path,
            });
        }
        let network_allowed = rules.network_crates.contains(&name)
            || rules.network_exempt.iter().any(|e| e.krate == name)
            || graph
                .path_to(m, |n| rules.network_crates.contains(&n))
                .is_some();
        if !network_allowed && let Some(path) = graph.path_to(m, |n| rules.is_network_client(n)) {
            out.violations.push(Violation::NetworkLeak {
                krate: name.to_owned(),
                path,
            });
        }
    }
    out.absent = rules
        .layers
        .iter()
        .flat_map(|l| l.crates)
        .filter(|c| !member_names.contains(*c))
        .map(|c| (*c).to_owned())
        .collect();
    out
}

fn print_map(rules: &Rules<'_>, outcome: &Outcome) {
    println!("Layers (a crate may depend only on lower layers):");
    for layer in rules.layers.iter().rev() {
        let crates: Vec<String> = layer
            .crates
            .iter()
            .map(|c| {
                if outcome.absent.iter().any(|a| a == c) {
                    format!("{c} (not in the workspace)")
                } else {
                    (*c).to_owned()
                }
            })
            .collect();
        println!(
            "  L{} {:<20} {}",
            layer.level,
            layer.name,
            crates.join(", ")
        );
        for e in rules
            .intra_layer
            .iter()
            .filter(|e| rules.level_of(e.from) == Some(layer.level))
        {
            println!("     {:<20} allowed: {} → {} ({})", "", e.from, e.to, e.why);
        }
    }
    println!("     {:<20} {}", "tooling", rules.tooling.join(", "));
    println!(
        "  UI toolkit ({}) only in: {}",
        rules.ui_toolkit.join(", "),
        rules.ui_crates.join(", ")
    );
    println!(
        "  HTTP/TLS clients and keychain ({}) only in: {} and crates that depend on them",
        rules.network_client.join(", "),
        rules.network_crates.join(", ")
    );
    for e in rules.network_exempt {
        println!("     {:<20} exempt: {} ({})", "", e.krate, e.why);
    }
}

/// `cargo xtask layers [--quiet]`.
pub fn main(args: &[String]) -> Result<bool> {
    let mut quiet = false;
    for a in args {
        match a.as_str() {
            "-q" | "--quiet" => quiet = true,
            "-h" | "--help" => {
                println!(
                    "usage: cargo xtask layers [--quiet]\n\n\
                     Checks the workspace dependency graph (cargo metadata, all features, all \
                     platforms) against the layers in xtask/src/layers.rs."
                );
                return Ok(true);
            }
            other => bail!("unknown option `{other}` (see `cargo xtask layers --help`)"),
        }
    }
    let rules = Rules::CURRENT;
    let problems = rules.problems();
    if !problems.is_empty() {
        bail!("invalid layer rules: {}", problems.join("; "));
    }
    let meta = util::cargo_metadata(&["--all-features"])?;
    let graph = Graph::from_metadata(&meta)?;
    let outcome = check(&graph, &rules);
    if !quiet {
        print_map(&rules, &outcome);
    }
    let layered = outcome.crates.iter().filter(|(_, l)| l.is_some()).count();
    if outcome.violations.is_empty() {
        println!(
            "layers: ok ({layered} layered crates, {} internal edges, no UI toolkit outside {}, \
             no HTTP/TLS client or keychain outside {} and its dependents)",
            outcome.edges,
            rules.ui_crates.join(", "),
            rules.network_crates.join(", ")
        );
        return Ok(true);
    }
    for v in &outcome.violations {
        println!("error: {v}");
    }
    println!(
        "layers: {} violation(s); fix the dependency or, if it is intended, update \
         xtask/src/layers.rs and docs/ARCHITECTURE.md together",
        outcome.violations.len()
    );
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_LAYERS: &[Layer] = &[
        Layer {
            level: 0,
            name: "model",
            crates: &["model"],
        },
        Layer {
            level: 1,
            name: "m",
            crates: &["m"],
        },
        Layer {
            level: 2,
            name: "engines",
            crates: &["dax", "core", "ai"],
        },
        Layer {
            level: 3,
            name: "front",
            crates: &["cli", "app", "future"],
        },
    ];

    const RULES: Rules<'static> = Rules {
        layers: TEST_LAYERS,
        intra_layer: &[Edge {
            from: "core",
            to: "dax",
            why: "test",
        }],
        exceptions: &[],
        tooling: &["xtask"],
        ui_crates: &["app"],
        ui_toolkit: &["tauri*", "wry"],
        isolated: &["m"],
        network_crates: &["ai"],
        network_client: &["ureq*", "rustls*", "hyper*", "keyring*", "*-keyring-store"],
        network_exempt: &[Exempt {
            krate: "app",
            why: "test",
        }],
    };

    /// Graph builder: members first, then external packages.
    struct G {
        graph: Graph,
    }

    impl G {
        fn new(members: &[&str], external: &[&str]) -> Self {
            let mut graph = Graph::default();
            for name in members.iter().chain(external) {
                graph.nodes.push(Node {
                    name: (*name).to_owned(),
                    deps: Vec::new(),
                });
            }
            graph.members = (0..members.len()).collect();
            Self { graph }
        }

        fn edge(mut self, from: &str, to: &str, kinds: &[DepKind]) -> Self {
            let find = |n: &str| self.graph.nodes.iter().position(|x| x.name == n).unwrap();
            let (f, t) = (find(from), find(to));
            self.graph.nodes[f]
                .deps
                .push((t, kinds.iter().copied().collect()));
            self
        }

        fn with_external(mut self, name: &str) -> Self {
            self.graph.nodes.push(Node {
                name: name.to_owned(),
                deps: Vec::new(),
            });
            self
        }

        fn check(&self) -> Outcome {
            check(&self.graph, &RULES)
        }
    }

    use DepKind::{Build, Dev, Normal};

    fn base() -> G {
        G::new(
            &["model", "m", "dax", "core", "ai", "cli", "app", "xtask"],
            &["serde", "tauri", "tauri-utils", "wry"],
        )
        .edge("dax", "model", &[Normal])
        .edge("core", "dax", &[Normal])
        .edge("core", "m", &[Normal])
        .edge("cli", "core", &[Normal])
        .edge("app", "core", &[Normal])
        .edge("app", "tauri", &[Normal])
        .edge("tauri", "wry", &[Normal])
        .edge("model", "serde", &[Normal])
    }

    #[test]
    fn current_rules_are_consistent() {
        assert_eq!(Rules::CURRENT.problems(), Vec::<String>::new());
        assert_eq!(RULES.problems(), Vec::<String>::new());
    }

    #[test]
    fn downward_and_allowed_intra_layer_edges_pass() {
        let out = base().check();
        assert_eq!(out.violations, vec![]);
        assert_eq!(out.edges, 5);
        assert_eq!(out.absent, vec!["future".to_owned()]);
        assert!(out.crates.contains(&("xtask".to_owned(), None)));
    }

    #[test]
    fn upward_edges_fail_for_every_kind() {
        let out = base()
            .edge("model", "core", &[Normal])
            .edge("dax", "cli", &[Dev])
            .check();
        assert_eq!(
            out.violations,
            vec![
                Violation::Upward {
                    from: "model".into(),
                    from_level: 0,
                    to: "core".into(),
                    to_level: 2,
                    kinds: vec![Normal],
                },
                Violation::Upward {
                    from: "dax".into(),
                    from_level: 2,
                    to: "cli".into(),
                    to_level: 3,
                    kinds: vec![Dev],
                },
            ]
        );
    }

    #[test]
    fn same_layer_edges_need_an_allow_list_entry() {
        let out = base()
            .edge("dax", "core", &[Normal])
            .edge("cli", "app", &[Build])
            .check();
        assert_eq!(
            out.violations,
            vec![
                Violation::Sideways {
                    from: "dax".into(),
                    to: "core".into(),
                    level: 2,
                    kinds: vec![Normal],
                },
                Violation::Sideways {
                    from: "cli".into(),
                    to: "app".into(),
                    level: 3,
                    kinds: vec![Build],
                },
                // Through the UI crate, the CLI now reaches the toolkit too.
                Violation::UiLeak {
                    krate: "cli".into(),
                    path: vec!["cli".into(), "app".into(), "tauri".into()],
                },
            ]
        );
    }

    #[test]
    fn exceptions_allow_listed_upward_edges() {
        let g = base().edge("dax", "cli", &[Normal]);
        let mut rules = RULES;
        let exceptions = [Edge {
            from: "dax",
            to: "cli",
            why: "test",
        }];
        rules.exceptions = &exceptions;
        assert_eq!(check(&g.graph, &rules).violations, vec![]);
    }

    #[test]
    fn unassigned_crates_tooling_and_isolation_are_enforced() {
        let g = G::new(&["model", "m", "newcrate", "xtask", "cli"], &[])
            .edge("cli", "xtask", &[Dev])
            .edge("m", "model", &[Normal])
            .edge("xtask", "cli", &[Normal]);
        let out = check(&g.graph, &RULES);
        assert_eq!(
            out.violations,
            vec![
                Violation::NotIsolated {
                    krate: "m".into(),
                    dep: "model".into(),
                    kinds: vec![Normal],
                },
                Violation::Unassigned {
                    krate: "newcrate".into(),
                },
                Violation::OnTooling {
                    from: "cli".into(),
                    to: "xtask".into(),
                    kinds: vec![Dev],
                },
            ]
        );
    }

    #[test]
    fn ui_toolkit_leaks_are_found_transitively_with_their_path() {
        let out = base()
            .with_external("helper")
            .edge("core", "helper", &[Normal])
            .edge("helper", "tauri-utils", &[Build])
            .check();
        assert_eq!(
            out.violations,
            vec![
                Violation::UiLeak {
                    krate: "core".into(),
                    path: vec!["core".into(), "helper".into(), "tauri-utils".into()],
                },
                Violation::UiLeak {
                    krate: "cli".into(),
                    path: vec![
                        "cli".into(),
                        "core".into(),
                        "helper".into(),
                        "tauri-utils".into()
                    ],
                },
            ]
        );
    }

    #[test]
    fn own_dev_dependencies_count_but_not_those_of_dependencies() {
        // `dax` dev-depends on wry directly: a leak. `core` → `dax` must not inherit it.
        let out = base().edge("dax", "wry", &[Dev]).check();
        assert_eq!(
            out.violations,
            vec![Violation::UiLeak {
                krate: "dax".into(),
                path: vec!["dax".into(), "wry".into()],
            }]
        );
    }

    #[test]
    fn network_clients_stay_in_network_crates_and_their_dependents() {
        let g = G::new(
            &["model", "dax", "core", "ai", "cli", "app"],
            &[
                "ureq",
                "rustls-webpki",
                "keyring-core",
                "apple-native-keyring-store",
                "helper",
                "hyper",
            ],
        )
        .edge("ai", "model", &[Normal])
        .edge("ai", "ureq", &[Normal])
        .edge("ureq", "rustls-webpki", &[Normal])
        .edge("ai", "keyring-core", &[Normal])
        .edge("keyring-core", "apple-native-keyring-store", &[Normal])
        // A dependent of the network crate may reach clients, even through other paths.
        .edge("cli", "ai", &[Normal])
        .edge("cli", "core", &[Normal])
        // `core` reaches a keychain store through a helper: a leak.
        .edge("core", "helper", &[Normal])
        .edge("helper", "apple-native-keyring-store", &[Build])
        // The exempt crate may; a dev-dependency of `dax` may not.
        .edge("app", "hyper", &[Normal])
        .edge("dax", "hyper", &[Dev]);
        let out = check(&g.graph, &RULES);
        assert_eq!(
            out.violations,
            vec![
                Violation::NetworkLeak {
                    krate: "dax".into(),
                    path: vec!["dax".into(), "hyper".into()],
                },
                Violation::NetworkLeak {
                    krate: "core".into(),
                    path: vec![
                        "core".into(),
                        "helper".into(),
                        "apple-native-keyring-store".into()
                    ],
                },
            ]
        );
    }

    #[test]
    fn current_network_rule_covers_clients_and_keychains_but_not_plain_http_types() {
        let rules = Rules::CURRENT;
        for name in [
            "ureq",
            "ureq-proto",
            "reqwest",
            "hyper-util",
            "h2",
            "rustls",
            "rustls-webpki",
            "tokio-rustls",
            "native-tls",
            "openssl-sys",
            "webpki-roots",
            "curl-sys",
            "keyring",
            "keyring-core",
            "apple-native-keyring-store",
            "zbus-secret-service-keyring-store",
        ] {
            assert!(rules.is_network_client(name), "{name}");
        }
        for name in ["http", "http-body", "ring", "tokio", "serde_json", "url"] {
            assert!(!rules.is_network_client(name), "{name}");
        }
        assert_eq!(rules.level_of("pbi-ai"), rules.level_of("pbi-ops"));
        assert!(Rules::listed(rules.intra_layer, "pbi-ai", "pbi-ops"));
        assert!(!Rules::listed(rules.intra_layer, "pbi-ops", "pbi-ai"));
    }

    #[test]
    fn graph_reads_cargo_metadata_json() {
        let meta = serde_json::json!({
            "packages": [
                {"id": "a 0.1", "name": "pbi-model"},
                {"id": "b 0.1", "name": "pbi-dax"},
                {"id": "c 0.1", "name": "tauri"},
            ],
            "workspace_members": ["a 0.1", "b 0.1"],
            "resolve": {"nodes": [
                {"id": "a 0.1", "deps": []},
                {"id": "b 0.1", "deps": [
                    {"pkg": "a 0.1", "dep_kinds": [{"kind": null}, {"kind": "dev"}]},
                    {"pkg": "c 0.1", "dep_kinds": [{"kind": "build", "target": "cfg(unix)"}]},
                ]},
                {"id": "c 0.1", "deps": []},
            ]},
        });
        let graph = Graph::from_metadata(&meta).unwrap();
        assert_eq!(graph.members, vec![0, 1]);
        assert_eq!(
            graph.nodes[1].deps,
            vec![
                (0, BTreeSet::from([Normal, Dev])),
                (2, BTreeSet::from([Build])),
            ]
        );
        let out = check(&graph, &Rules::CURRENT);
        assert_eq!(
            out.violations,
            vec![Violation::UiLeak {
                krate: "pbi-dax".into(),
                path: vec!["pbi-dax".into(), "tauri".into()],
            }]
        );
    }
}
