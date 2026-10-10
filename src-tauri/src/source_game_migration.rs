//! A deterministic, read-only migration plan for games authored in Godot.
//!
//! It is deliberately not a gameplay translator. The data model records the
//! information Kairo understands, its provenance, and each unsupported
//! dependency. Importers may only claim native play after their own execution
//! and parity gates have passed; this layer never claims that by inference.

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::source_game::inspect_source_game;
use crate::source_game_graph::inspect_godot_graph;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationIssue {
    pub code: String,
    pub severity: String,
    pub source: String,
    pub explanation: String,
    pub suggested_action: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationObject {
    /// Deterministic scene-local key, not an engine pointer or Godot instance ID.
    pub source_path: String,
    pub source_type: String,
    pub authored_script: Option<String>,
    pub packed_scene: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationAsset {
    pub resource_id: String,
    pub resource_type: String,
    pub source_path: String,
    pub exists: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GodotMigrationPlan {
    pub schema: &'static str,
    pub engine: &'static str,
    pub title: String,
    pub main_scene: String,
    pub source_objects: Vec<MigrationObject>,
    pub source_assets: Vec<MigrationAsset>,
    pub signal_count: usize,
    pub native_gameplay_ready: bool,
    pub original_runtime_available: bool,
    pub issues: Vec<MigrationIssue>,
}

fn issue(code: &str, severity: &str, source: impl Into<String>,
         explanation: impl Into<String>, suggested_action: &str) -> MigrationIssue {
    MigrationIssue {
        code: code.into(),
        severity: severity.into(),
        source: source.into(),
        explanation: explanation.into(),
        suggested_action: suggested_action.into(),
    }
}

/// Only read project-owned files; reject every symlink in the path (including
/// intermediate directory components) before reporting a resource as present.
fn safe_owned_resource(root: &Path, source: &str) -> bool {
    let Some(relative) = source.strip_prefix("res://") else { return false };
    let relative = Path::new(relative);
    if relative.as_os_str().is_empty()
        || !relative.components().all(|part| matches!(part, Component::Normal(_))) {
        return false;
    }
    let mut current = PathBuf::from(root);
    for part in relative.components() {
        current.push(part.as_os_str());
        let Ok(meta) = fs::symlink_metadata(&current) else { return false };
        if meta.file_type().is_symlink() { return false; }
    }
    current.is_file()
}

/// Returns an actionable plan without running GDScript or executing source
/// engine plugins. The source game is never written to.
pub fn plan_godot_migration(root: &Path) -> Result<GodotMigrationPlan, String> {
    let info = inspect_source_game(root)?;
    if info.engine != "Godot" {
        return Err("Migration plans currently support only Godot text scenes; Unity/Unreal inspection remains read-only".into());
    }
    let graph = inspect_godot_graph(&info.root)?;
    let mut plan = GodotMigrationPlan {
        schema: "kairo-migration-plan 1",
        engine: "Godot",
        title: info.display_name,
        main_scene: graph.source_scene,
        source_objects: Vec::with_capacity(graph.nodes.len()),
        source_assets: Vec::with_capacity(graph.resources.len()),
        signal_count: graph.signals.len(),
        native_gameplay_ready: false,
        original_runtime_available: info.original_runtime_available,
        issues: Vec::new(),
    };

    let mut resource_ids = BTreeMap::new();
    for res in &graph.resources {
        if resource_ids.insert(res.id.as_str(), res).is_some() {
            plan.issues.push(issue("DUPLICATE_RESOURCE_ID", "error",
                &res.id, "Multiple resources have the same scene-local ID.",
                "Give each external resource a unique ID in the Godot scene."));
        }
        let exists = safe_owned_resource(&info.root, &res.path);
        plan.source_assets.push(MigrationAsset {
            resource_id: res.id.clone(),
            resource_type: res.resource_type.clone(),
            source_path: res.path.clone(),
            exists,
        });
        if !exists {
            plan.issues.push(issue("RESOURCE_UNAVAILABLE", "warning", &res.path,
                "The resource path is missing, UID-only, outside the project, or crosses a symbolic link.",
                "Reimport the game using Godot, or supply the original project-owned resource."));
        }
        if res.resource_type == "Script" {
            plan.issues.push(issue("GDSCRIPT_RUNTIME", "blocker", &res.path,
                "GDScript behavior and callbacks have no verified Kairo translation.",
                "Port gameplay methods to Kairo's supported runtime and compare original behavior."));
        } else if res.resource_type == "PackedScene" {
            plan.issues.push(issue("PACKED_SCENE", "blocker", &res.path,
                "Instanced PackedScene hierarchy is not reconstructed as native Kairo entities.",
                "Expand and translate referenced scenes with stable identity and transforms."));
        }
    }

    // Godot parent attributes are root-relative paths. A root's source path
    // is "."; its direct children are "Child", nested paths "Child/Grandchild".
    let mut known_paths = BTreeSet::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        let path = if index == 0 && node.parent.is_empty() {
            ".".to_string()
        } else if node.parent == "." || node.parent.is_empty() {
            node.name.clone()
        } else {
            format!("{}/{}", node.parent, node.name)
        };
        if index != 0 && node.parent != "." && !node.parent.is_empty()
            && !known_paths.contains(&node.parent) {
            plan.issues.push(issue("MISSING_PARENT", "error", &path,
                format!("Scene parent '{}' was not declared before this node.", node.parent),
                "Repair or re-export the Godot scene hierarchy."));
        }
        if !known_paths.insert(path.clone()) {
            plan.issues.push(issue("DUPLICATE_NODE_PATH", "error", &path,
                "Two nodes resolve to the same scene-local path.",
                "Rename duplicated nodes before migration."));
        }

        let script = node.script_resource.as_ref().map(|id| {
            resource_ids.get(id.as_str()).map(|res| res.path.clone())
                .unwrap_or_else(|| format!("missing-resource:{id}"))
        });
        let packed = node.instance_resource.as_ref().map(|id| {
            resource_ids.get(id.as_str()).map(|res| res.path.clone())
                .unwrap_or_else(|| format!("missing-resource:{id}"))
        });
        for id in [node.script_resource.as_ref(), node.instance_resource.as_ref()].into_iter().flatten() {
            if !resource_ids.contains_key(id.as_str()) {
                plan.issues.push(issue("UNRESOLVED_RESOURCE_ID", "error", &path,
                    format!("Node references undeclared external resource '{id}'."),
                    "Restore the missing ext_resource declaration."));
            }
        }
        if node.node_type.contains("2D") || matches!(node.node_type.as_str(),
            "Control" | "ColorRect" | "CanvasLayer" | "Sprite2D" | "AnimatedSprite2D") {
            plan.issues.push(issue("CANVAS_2D_RUNTIME", "blocker", &path,
                format!("Godot node type '{}' requires a tested Kairo 2D renderer and scene serializer.", node.node_type),
                "Use the original Godot runtime until Kairo's native 2D adapter is implemented."));
        }
        if node.node_type == "Timer" {
            plan.issues.push(issue("TIMER_SEMANTICS", "blocker", &path,
                "Godot Timer pause/time-scale/timeout behavior is not translated.",
                "Port timer scheduling and verify callbacks against original input traces."));
        }
        plan.source_objects.push(MigrationObject {
            source_path: path,
            source_type: node.node_type.clone(),
            authored_script: script,
            packed_scene: packed,
        });
    }

    for signal in &graph.signals {
        plan.issues.push(issue("SIGNAL_BINDING", "blocker",
            format!("{}::{}", signal.from, signal.signal),
            format!("Signal connection to '{}' method '{}' is not translated.", signal.to, signal.method),
            "Rebind the event to an equivalent Kairo runtime callback and test it."));
    }
    if !plan.issues.iter().any(|entry| entry.severity == "blocker") {
        // A zero-issue structural parse still cannot prove scene/component
        // equivalence, let alone native gameplay. Require differential replay.
        plan.issues.push(issue("PARITY_NOT_ESTABLISHED", "blocker", &plan.main_scene,
            "No reference replay confirms that this game behaves the same in Kairo.",
            "Run original and reconstructed games under matching input traces, then compare."));
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scene(root: &Path, body: &str) {
        fs::write(root.join("project.godot"),
            "config_version=5\n[application]\nconfig/name=\"Port Candidate\"\nrun/main_scene=\"res://main.tscn\"\n").unwrap();
        fs::write(root.join("main.tscn"), body).unwrap();
    }

    #[test]
    fn stable_node_paths_missing_resources_and_signals_are_actionable() {
        let tmp = tempfile::tempdir().unwrap();
        scene(tmp.path(), "[gd_scene format=3]\n\
            [ext_resource type=\"Script\" path=\"res://scripts/player.gd\" id=\"1\"]\n\
            [node name=\"World\" type=\"Node2D\"]\n\
            [node name=\"Player\" type=\"CharacterBody2D\" parent=\".\"]\n\
            script = ExtResource(\"1\")\n\
            [node name=\"Camera\" type=\"Camera2D\" parent=\"Player\"]\n\
            [connection signal=\"hit\" from=\"Player\" to=\".\" method=\"game_over\"]\n");
        let plan = plan_godot_migration(tmp.path()).unwrap();
        assert_eq!(plan.schema, "kairo-migration-plan 1");
        assert_eq!(plan.source_objects.len(), 3);
        assert_eq!(plan.source_objects[0].source_path, ".");
        assert_eq!(plan.source_objects[1].source_path, "Player");
        assert_eq!(plan.source_objects[2].source_path, "Player/Camera");
        assert_eq!(plan.signal_count, 1);
        assert!(!plan.native_gameplay_ready);
        for expected in ["RESOURCE_UNAVAILABLE", "GDSCRIPT_RUNTIME",
                         "CANVAS_2D_RUNTIME", "SIGNAL_BINDING"] {
            assert!(plan.issues.iter().any(|problem| problem.code == expected), "{expected}");
        }
    }

    #[test]
    fn source_symlink_and_parent_traversal_are_not_considered_local_assets() {
        let tmp = tempfile::tempdir().unwrap();
        scene(tmp.path(), "[gd_scene format=3]\n\
            [ext_resource type=\"Texture2D\" path=\"res://../secret.png\" id=\"1\"]\n\
            [node name=\"World\" type=\"Node3D\"]\n");
        let plan = plan_godot_migration(tmp.path()).unwrap();
        assert!(!plan.source_assets[0].exists);
        assert!(!plan.native_gameplay_ready);
    }

    #[test]
    fn never_claims_native_gameplay_without_reference_replay() {
        let tmp = tempfile::tempdir().unwrap();
        scene(tmp.path(), "[gd_scene format=3]\n[node name=\"Root\" type=\"Node3D\"]\n");
        let plan = plan_godot_migration(tmp.path()).unwrap();
        assert_eq!(plan.issues.len(), 1);
        assert_eq!(plan.issues[0].code, "PARITY_NOT_ESTABLISHED");
        assert!(!plan.native_gameplay_ready);
    }
}
