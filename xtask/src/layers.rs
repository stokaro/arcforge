// Copyright (c) 2026 ArtCraft Team and the PhotoCraft contributors
// Copyright (c) 2026 ArcForge contributors
// SPDX-License-Identifier: MIT
// Adapted from PhotoCraft 71f0d7483eb9ec56f483a2a4dbd13c2e6a0f65af,
// xtask/src/layers.rs: metadata/model separation and explicit layer rules.
// This smaller implementation also checks transitive dependencies and keeps the
// CLI subject to the headless rule. See licenses/artcraft-photocraft-MIT.txt.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use serde_json::Value;

use crate::Result;

#[derive(Debug)]
struct Package {
    name: String,
    declared: Vec<String>,
    edges: Vec<String>,
}

#[derive(Debug)]
struct Graph {
    packages: BTreeMap<String, Package>,
    members: BTreeSet<String>,
}

fn layer(name: &str) -> Option<u8> {
    match name {
        "arcforge-core" => Some(0),
        "arcforge-ui-egui" | "arcforge-platform-macos" | "arcforge-cli" => Some(1),
        "arcforge" => Some(2),
        "xtask" => Some(3),
        _ => None,
    }
}

fn headless(name: &str) -> bool {
    matches!(name, "arcforge-core" | "arcforge-cli")
}

fn is_ui(name: &str) -> bool {
    matches!(name, "egui" | "eframe" | "winit" | "rfd")
        || name.starts_with("egui_")
        || matches!(
            name,
            "arcforge-ui-egui" | "arcforge-platform-macos" | "objc2-app-kit"
        )
}

fn parse(metadata: &Value) -> Result<Graph> {
    let mut packages = BTreeMap::new();
    for package in metadata["packages"]
        .as_array()
        .ok_or("metadata has no packages")?
    {
        let id = package["id"].as_str().ok_or("package has no id")?;
        let name = package["name"].as_str().ok_or("package has no name")?;
        let declared = package["dependencies"]
            .as_array()
            .ok_or("package has no dependency list")?
            .iter()
            .map(|dependency| {
                dependency["name"]
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "dependency has no package name".into())
            })
            .collect::<Result<Vec<_>>>()?;
        packages.insert(
            id.to_owned(),
            Package {
                name: name.to_owned(),
                declared,
                edges: Vec::new(),
            },
        );
    }
    let members = metadata["workspace_members"]
        .as_array()
        .ok_or("metadata has no workspace members")?
        .iter()
        .map(|id| {
            id.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "invalid workspace member id".into())
        })
        .collect::<Result<BTreeSet<_>>>()?;
    for member in &members {
        if !packages.contains_key(member) {
            return Err(format!("workspace member {member} is missing from metadata").into());
        }
    }

    let nodes = metadata["resolve"]["nodes"]
        .as_array()
        .ok_or("full dependency resolution is required; do not use --no-deps")?;
    let mut resolved = BTreeSet::new();
    for node in nodes {
        let id = node["id"].as_str().ok_or("resolve node has no id")?;
        let mut edges = Vec::new();
        for dependency in node["deps"].as_array().ok_or("resolve node has no deps")? {
            // Intentionally retain all normal/build/dev and target-conditioned
            // edges. `pkg`, not the rename in `name`, identifies the package.
            let dependency_id = dependency["pkg"]
                .as_str()
                .ok_or("resolved dependency has no pkg")?;
            if !packages.contains_key(dependency_id) {
                return Err(
                    format!("resolved dependency {dependency_id} has no package metadata").into(),
                );
            }
            edges.push(dependency_id.to_owned());
        }
        packages
            .get_mut(id)
            .ok_or("resolve node has no package metadata")?
            .edges = edges;
        resolved.insert(id);
    }
    if members.iter().any(|id| !resolved.contains(id.as_str())) {
        return Err("a workspace member is absent from the resolved dependency graph".into());
    }
    Ok(Graph { packages, members })
}

fn check(graph: &Graph) -> BTreeSet<String> {
    let mut violations = BTreeSet::new();
    let member_names: BTreeSet<&str> = graph
        .members
        .iter()
        .map(|id| graph.packages[id].name.as_str())
        .collect();
    for id in &graph.members {
        let package = &graph.packages[id];
        let Some(from) = layer(&package.name) else {
            violations.insert(format!(
                "{}: register the new workspace crate in xtask/src/layers.rs",
                package.name
            ));
            continue;
        };
        if package.name == "xtask" {
            continue;
        }
        // Declared dependencies also cover optional edges that resolution may
        // omit. Cargo metadata supplies actual package names despite aliases.
        for dependency in &package.declared {
            if dependency == &package.name {
                continue;
            }
            if member_names.contains(dependency.as_str())
                && let Some(to) = layer(dependency)
                && to >= from
            {
                violations.insert(format!("{} (L{from}) -> {dependency} (L{to}): dependencies must point to a lower layer", package.name));
            }
            if headless(&package.name) && is_ui(dependency) {
                violations.insert(format!(
                    "{}: headless package declares UI dependency {dependency}",
                    package.name
                ));
            }
        }
        if !headless(&package.name) {
            continue;
        }
        let mut queue = VecDeque::from([(id.clone(), vec![package.name.clone()])]);
        let mut visited = BTreeSet::new();
        while let Some((current, path)) = queue.pop_front() {
            if !visited.insert(current.clone()) {
                continue;
            }
            let node = &graph.packages[&current];
            if is_ui(&node.name) {
                violations.insert(format!("headless dependency path: {}", path.join(" -> ")));
                continue;
            }
            for next in &node.edges {
                let mut next_path = path.clone();
                next_path.push(graph.packages[next].name.clone());
                queue.push_back((next.clone(), next_path));
            }
        }
    }
    violations
}

pub fn run(root: &Path) -> Result<()> {
    let graph = parse(&crate::metadata(root, true)?)?;
    let violations = check(&graph);
    if violations.is_empty() {
        println!("Dependency layers pass; archive core and CLI have no GUI dependency paths.");
        Ok(())
    } else {
        Err(violations.into_iter().collect::<Vec<_>>().join("\n").into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture(members: &[&str], edges: &[(&str, &str, &str)]) -> Value {
        let names: BTreeSet<&str> = members
            .iter()
            .copied()
            .chain(edges.iter().flat_map(|(from, to, _)| [*from, *to]))
            .collect();
        json!({
            "workspace_members": members,
            "packages": names.iter().map(|name| json!({
                "id": name, "name": name,
                "dependencies": edges.iter().filter(|(from, _, _)| from == name).map(|(_, to, kind)| json!({
                    "name": to, "rename": "innocent_alias", "kind": kind,
                    "target": "cfg(target_os = \"windows\")"
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>(),
            "resolve": { "nodes": names.iter().map(|name| json!({
                "id": name,
                "deps": edges.iter().filter(|(from, _, _)| from == name).map(|(_, to, kind)| json!({
                    "name": "innocent_alias", "pkg": to,
                    "dep_kinds": [{"kind":kind, "target":"cfg(target_os = \"windows\")"}]
                })).collect::<Vec<_>>()
            })).collect::<Vec<_>>() }
        })
    }

    fn violations(metadata: &Value) -> Vec<String> {
        check(&parse(metadata).expect("valid test graph"))
            .into_iter()
            .collect()
    }

    #[test]
    fn gui_dependencies_do_not_contaminate_independent_headless_packages() {
        let metadata = fixture(
            &[
                "arcforge-core",
                "arcforge-cli",
                "arcforge-ui-egui",
                "arcforge",
                "xtask",
            ],
            &[
                ("arcforge-cli", "arcforge-core", "normal"),
                ("arcforge-ui-egui", "arcforge-core", "normal"),
                ("arcforge-ui-egui", "egui", "normal"),
                ("arcforge", "arcforge-ui-egui", "normal"),
                ("arcforge", "eframe", "normal"),
            ],
        );
        assert!(violations(&metadata).is_empty());
    }

    #[test]
    fn cli_is_not_exempt_from_layering_even_in_dev_or_build_dependencies() {
        for kind in ["normal", "dev", "build"] {
            let metadata = fixture(
                &["arcforge-cli", "arcforge-ui-egui"],
                &[("arcforge-cli", "arcforge-ui-egui", kind)],
            );
            assert!(
                violations(&metadata)
                    .iter()
                    .any(|message| message.contains("lower layer"))
            );
        }
    }

    #[test]
    fn finds_renamed_transitive_ui_dependencies_in_all_edge_kinds_and_targets() {
        for head in ["arcforge-core", "arcforge-cli"] {
            for kind in ["normal", "dev", "build"] {
                for ui in ["egui", "eframe", "winit", "rfd"] {
                    let metadata =
                        fixture(&[head], &[(head, "bridge", kind), ("bridge", ui, "normal")]);
                    assert!(
                        violations(&metadata)
                            .iter()
                            .any(|message| message.contains(&format!("{head} -> bridge -> {ui}")))
                    );
                }
            }
        }
    }

    #[test]
    fn declared_optional_ui_dependencies_cannot_hide_from_resolution() {
        let mut metadata = fixture(&["arcforge-core"], &[]);
        metadata["packages"][0]["dependencies"] =
            json!([{"name":"egui", "rename":"paint", "kind":"build", "optional":true}]);
        assert!(
            violations(&metadata)
                .iter()
                .any(|message| message.contains("declares UI dependency egui"))
        );
    }

    #[test]
    fn cycles_do_not_hide_ui_or_loop_forever() {
        let metadata = fixture(
            &["arcforge-cli"],
            &[
                ("arcforge-cli", "a", "dev"),
                ("a", "b", "normal"),
                ("b", "a", "normal"),
                ("b", "rfd", "build"),
            ],
        );
        assert!(
            violations(&metadata)
                .iter()
                .any(|message| message.contains("arcforge-cli -> a -> b -> rfd"))
        );
    }

    #[test]
    fn requires_registration_and_complete_resolved_metadata() {
        assert!(!violations(&fixture(&["arcforge-new"], &[])).is_empty());
        let mut metadata = fixture(&["arcforge-core"], &[]);
        metadata["resolve"] = Value::Null;
        assert!(parse(&metadata).is_err());
    }
}
