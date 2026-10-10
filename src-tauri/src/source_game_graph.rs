//! Lossless-enough STRUCTURAL inspection of Godot text scenes.
//!
//! This is a typed intermediate representation, not a Godot interpreter.
//! It never evaluates GDScript, constructs arbitrary resource classes or
//! executes imported content. Every unsupported behaviour remains explicit.

use serde::Serialize;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalResource {
    pub id: String,
    pub resource_type: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceNode {
    pub name: String,
    pub parent: String,
    pub node_type: String,
    pub instance_resource: Option<String>,
    pub script_resource: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceSignal {
    pub signal: String,
    pub from: String,
    pub to: String,
    pub method: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GodotSceneGraph {
    pub source_scene: String,
    pub nodes: Vec<SourceNode>,
    pub resources: Vec<ExternalResource>,
    pub signals: Vec<SourceSignal>,
    pub unsupported_behaviors: Vec<String>,
}

/// Parse a quoted header attribute, handling backslash-escaped quotes without
/// accepting eval expressions. Missing attributes are represented as None.
fn attribute(line: &str, key: &str) -> Option<String> {
    let search = format!("{key}=\"");
    let start = line.find(&search)? + search.len();
    let mut value = String::new();
    let mut escaped = false;
    for ch in line[start..].chars() {
        if escaped {
            value.push(ch);
            escaped = false;
        } else if ch == '\\' {
            escaped = true;
        } else if ch == '"' {
            return Some(value);
        } else {
            value.push(ch);
        }
    }
    None
}

fn resource_call(line: &str, name: &str) -> Option<String> {
    let prefix = format!("{name}(\"");
    let first = line.find(&prefix)? + prefix.len();
    let tail = &line[first..];
    let last = tail.find("\")")?;
    Some(tail[..last].to_owned())
}

fn text_scene(root: &Path) -> Result<(String, String), String> {
    let info = crate::source_game::inspect_source_game(root)?;
    if info.engine != "Godot" {
        return Err("Scene graph decoding currently supports Godot .tscn sources only".into());
    }
    let scene = info.main_scene.ok_or("Source project has no validated main .tscn scene")?;
    if !scene.ends_with(".tscn") {
        return Err("Binary Godot scenes (.scn) are not interpreted; export a .tscn scene".into());
    }
    let relative = scene.strip_prefix("res://").ok_or("Expected a Godot res:// path")?;
    let scene_file = info.root.join(relative);
    let meta = fs::symlink_metadata(&scene_file).map_err(|e| e.to_string())?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("Scene must be a project-owned regular file".into());
    }
    if meta.len() > 8 * 1024 * 1024 {
        return Err("Godot scene exceeds the 8 MiB structural parser limit".into());
    }
    let body = fs::read_to_string(&scene_file)
        .map_err(|e| format!("Cannot read Godot text scene: {e}"))?;
    Ok((scene, body))
}

pub fn inspect_godot_graph(root: &Path) -> Result<GodotSceneGraph, String> {
    let (scene, content) = text_scene(root)?;
    let mut graph = GodotSceneGraph {
        source_scene: scene,
        nodes: Vec::new(),
        resources: Vec::new(),
        signals: Vec::new(),
        unsupported_behaviors: Vec::new(),
    };
    let mut active_node = None;
    for raw in content.lines() {
        let line = raw.trim();
        if line.starts_with("[node ") && line.ends_with(']') {
            let name = attribute(line, "name")
                .ok_or("Godot scene node is missing a name")?;
            if name.trim().is_empty() {
                return Err("Godot scene node has an empty name".into());
            }
            let parent = attribute(line, "parent").unwrap_or_default();
            let node_type = attribute(line, "type")
                .unwrap_or_else(|| "InstancedScene".into());
            let instance_resource = resource_call(line, "ExtResource");
            graph.nodes.push(SourceNode {
                name, parent, node_type, instance_resource, script_resource: None,
            });
            active_node = Some(graph.nodes.len() - 1);
        } else if line.starts_with("[ext_resource ") && line.ends_with(']') {
            let path = attribute(line, "path").unwrap_or_default();
            let id = attribute(line, "id").unwrap_or_default();
            let resource_type = attribute(line, "type").unwrap_or_default();
            if !id.is_empty() {
                graph.resources.push(ExternalResource { id, resource_type, path });
            }
            active_node = None;
        } else if line.starts_with("[connection ") && line.ends_with(']') {
            graph.signals.push(SourceSignal {
                signal: attribute(line, "signal").unwrap_or_default(),
                from: attribute(line, "from").unwrap_or_default(),
                to: attribute(line, "to").unwrap_or_default(),
                method: attribute(line, "method").unwrap_or_default(),
            });
            active_node = None;
        } else if line.starts_with('[') && line.ends_with(']') {
            active_node = None;
        } else if line.starts_with("script = ") {
            if let Some(index) = active_node {
                graph.nodes[index].script_resource = resource_call(line, "ExtResource");
            }
        }
    }
    if graph.nodes.is_empty() {
        return Err("Godot .tscn contains no authorable nodes".into());
    }

    if graph.nodes.iter().any(|node| node.node_type.contains("2D") ||
        node.node_type == "Control" || node.node_type == "ColorRect" ||
        node.node_type == "InstancedScene") {
        graph.unsupported_behaviors.push(
            "2D canvas and imported PackedScene instances need a native Kairo 2D authoring/runtime adapter".into());
    }
    if graph.nodes.iter().any(|node| node.node_type == "Timer") {
        graph.unsupported_behaviors.push(
            "Godot Timer and scheduling semantics are not converted".into());
    }
    if graph.resources.iter().any(|resource| resource.resource_type == "Script")
        || graph.nodes.iter().any(|node| node.script_resource.is_some()) {
        graph.unsupported_behaviors.push(
            "GDScript methods and callbacks are not executed or translated".into());
    }
    if !graph.signals.is_empty() {
        graph.unsupported_behaviors.push(
            "Godot signal connections require explicit Kairo event bindings".into());
    }
    Ok(graph)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_nodes_instances_scripts_and_signals_without_executing_them() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("project.godot"),
            "config_version=5\n[application]\nrun/main_scene=\"res://main.tscn\"\n").unwrap();
        fs::write(dir.path().join("main.tscn"),
            "[gd_scene format=3]\n\
             [ext_resource type=\"Script\" path=\"res://main.gd\" id=\"1\"]\n\
             [ext_resource type=\"PackedScene\" path=\"res://player.tscn\" id=\"2\"]\n\
             [node name=\"Main\" type=\"Node\"]\n\
             script = ExtResource(\"1\")\n\
             [node name=\"Player\" parent=\".\" instance=ExtResource(\"2\")]\n\
             [node name=\"Timer\" type=\"Timer\" parent=\".\"]\n\
             [connection signal=\"timeout\" from=\"Timer\" to=\".\" method=\"spawn\"]\n").unwrap();
        let graph = inspect_godot_graph(dir.path()).unwrap();
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.resources.len(), 2);
        assert_eq!(graph.signals.len(), 1);
        assert_eq!(graph.nodes[0].script_resource.as_deref(), Some("1"));
        assert_eq!(graph.nodes[1].instance_resource.as_deref(), Some("2"));
        assert!(graph.unsupported_behaviors.iter().any(|item| item.contains("GDScript")));
        assert!(graph.unsupported_behaviors.iter().any(|item| item.contains("Timer")));
    }
}
