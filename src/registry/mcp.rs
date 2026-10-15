//! MCP server resolution and `.mcp.json` rendering.
//!
//! Pure logic over the registry's `mcp/` layer: which servers a selection ends
//! up with, and the exact bytes of the `.mcp.json` the planner emits
//! (TECH_SPEC §3.6, §6.1).

use super::loader::Registry;
use super::types::{McpDef, Selection};

/// The servers a selection actually gets: `selection.mcp` ∩ registry ∩
/// applicable to its assignments. Order is the registry's (id-sorted at load).
/// Ids that are unknown, or whose `requires` is no longer met, are dropped
/// silently — the one-shot/interactive edges validate explicit picks up front,
/// and a reconstructed selection may legitimately carry a stale id after a
/// `remove`.
pub fn resolve<'r>(selection: &Selection, registry: &'r Registry) -> Vec<&'r McpDef> {
    registry
        .mcp_servers()
        .iter()
        .filter(|m| selection.mcp.contains(&m.id))
        .filter(|m| m.applies_to(&selection.assignments))
        .collect()
}

/// Registry default servers that apply to `new` but did not apply to `old`:
/// what an update should wire in when a role change makes a server available
/// (e.g. `add --devnet yaci` with a default `yaci-devkit`). Id order.
pub fn newly_applicable_defaults(
    registry: &Registry,
    old: &[super::types::RoleAssignment],
    new: &[super::types::RoleAssignment],
) -> Vec<String> {
    registry
        .mcp_servers()
        .iter()
        .filter(|m| m.default && m.applies_to(new) && !m.applies_to(old))
        .map(|m| m.id.clone())
        .collect()
}

/// The result of merging the resolved server set into an existing `.mcp.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpMerge {
    /// Write this content (created, or changed).
    Write(String),
    /// The file would hold no servers at all: delete it.
    Remove,
    /// Nothing to do: no file and no servers, the file already matches, or it
    /// is not something this tool wrote (unparsable / not an object) and is
    /// left alone.
    Unchanged,
}

/// Merge `servers` (the resolved set for the new selection) into the project's
/// current `.mcp.json` (`existing`, `None` when absent) at key level: every
/// **registry** id is set to its current rendering when resolved and removed
/// when not; every other key — a server the user added by hand — is kept
/// verbatim. This is how `add`/`remove` keep `.mcp.json`, `AGENTS.md` and the
/// JSON `mcp` field in agreement without clobbering user entries (§15.1).
pub fn merge_mcp_json(
    existing: Option<&str>,
    servers: &[&McpDef],
    registry: &Registry,
) -> McpMerge {
    use serde_json::{Map, Value};

    let mut root: Map<String, Value> = match existing {
        None => Map::new(),
        Some(text) => match serde_json::from_str::<Value>(text) {
            Ok(Value::Object(obj)) => obj,
            // Not ours to rewrite.
            _ => return McpMerge::Unchanged,
        },
    };
    let mut entries: Map<String, Value> = match root.remove("mcpServers") {
        Some(Value::Object(obj)) => obj,
        Some(_) => return McpMerge::Unchanged,
        None => Map::new(),
    };
    for def in registry.mcp_servers() {
        match servers.iter().find(|m| m.id == def.id) {
            Some(m) => {
                let value = serde_json::to_value(&m.server).expect("MCP server serializes");
                entries.insert(m.id.clone(), value);
            }
            None => {
                entries.remove(&def.id);
            }
        }
    }
    if entries.is_empty() && root.is_empty() {
        return if existing.is_some() {
            McpMerge::Remove
        } else {
            McpMerge::Unchanged
        };
    }
    root.insert("mcpServers".to_string(), Value::Object(entries));
    let mut text = serde_json::to_string_pretty(&Value::Object(root)).expect("JSON serializes");
    text.push('\n');
    if existing == Some(text.as_str()) {
        McpMerge::Unchanged
    } else {
        McpMerge::Write(text)
    }
}

/// The ids chosen when the user expresses no preference: every applicable
/// server with `default = true`, in id order.
pub fn default_ids(
    registry: &Registry,
    assignments: &[super::types::RoleAssignment],
) -> Vec<String> {
    registry
        .mcp_applicable(assignments)
        .into_iter()
        .filter(|m| m.default)
        .map(|m| m.id.clone())
        .collect()
}

/// Render the `.mcp.json` content for these servers: a `mcpServers` map keyed
/// by id, all keys sorted, pretty-printed with a trailing newline. Byte-stable
/// for a given server set (§11), and byte-identical to what
/// [`merge_mcp_json`] produces from no existing file — so an `add`/`remove`
/// right after `init` never rewrites an unchanged file.
pub fn render_mcp_json(servers: &[&McpDef], registry: &Registry) -> String {
    match merge_mcp_json(None, servers, registry) {
        McpMerge::Write(text) => text,
        // No servers: the planner never asks for this (it omits the file).
        McpMerge::Remove | McpMerge::Unchanged => "{\n  \"mcpServers\": {}\n}\n".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::types::{Network, Role, RoleAssignment};

    fn registry() -> Registry {
        Registry::load().expect("registry should load")
    }

    fn sel(assignments: Vec<RoleAssignment>, mcp: Vec<&str>) -> Selection {
        Selection {
            project_name: "demo".into(),
            assignments,
            network: Network::Preview,
            nix: false,
            spec: true,
            mcp: mcp.into_iter().map(String::from).collect(),
        }
    }

    fn a(role: Role, tool: &str) -> RoleAssignment {
        RoleAssignment {
            role,
            tool_id: tool.into(),
        }
    }

    #[test]
    fn resolve_drops_unknown_and_inapplicable_ids() {
        let reg = registry();
        // yaci-devkit requires devnet=yaci; "bogus" is unknown; order is by id
        // regardless of the order given.
        let s = sel(
            vec![a(Role::OnChain, "aiken")],
            vec!["yaci-devkit", "bogus", "cardano-debug"],
        );
        let ids: Vec<&str> = resolve(&s, &reg).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["cardano-debug"]);

        let s = sel(
            vec![a(Role::OnChain, "aiken"), a(Role::Devnet, "yaci")],
            vec!["yaci-devkit", "cardano-debug"],
        );
        let ids: Vec<&str> = resolve(&s, &reg).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["cardano-debug", "yaci-devkit"]);
    }

    #[test]
    fn default_ids_follow_applicability() {
        let reg = registry();
        assert_eq!(
            default_ids(&reg, &[a(Role::OnChain, "aiken")]),
            vec!["cardano-debug"]
        );
        // yaci-devkit applies with a yaci devnet but is opt-in (`default = false`).
        assert_eq!(
            default_ids(&reg, &[a(Role::Devnet, "yaci")]),
            vec!["cardano-debug"]
        );
        assert!(!reg.mcp("yaci-devkit").unwrap().default);
    }

    #[test]
    fn render_is_valid_claude_code_config() {
        let reg = registry();
        let s = sel(
            vec![a(Role::Devnet, "yaci")],
            vec!["cardano-debug", "yaci-devkit"],
        );
        let text = render_mcp_json(&resolve(&s, &reg), &reg);
        assert!(text.ends_with("}\n"));
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let servers = v["mcpServers"].as_object().unwrap();
        let keys: Vec<&String> = servers.keys().collect();
        assert_eq!(keys, vec!["cardano-debug", "yaci-devkit"]);

        let debug = &servers["cardano-debug"];
        assert_eq!(debug["type"], "stdio");
        assert_eq!(debug["command"], "npx");
        assert_eq!(debug["args"][1], "@cardananium/cardano-debug-mcp");
        assert_eq!(debug["env"]["KOIOS_API_KEY"], "${KOIOS_API_KEY:-}");
        assert_eq!(debug["timeout"], 600000);

        let yaci = &servers["yaci-devkit"];
        assert_eq!(yaci["type"], "http");
        assert_eq!(yaci["url"], "http://localhost:10000/mcp");
        assert!(yaci.get("command").is_none());
    }

    #[test]
    fn merge_adds_and_removes_registry_keys_but_keeps_user_keys() {
        let reg = registry();
        let existing = r#"{ "mcpServers": { "my-own": { "type": "http", "url": "http://x" }, "yaci-devkit": { "type": "http", "url": "stale" } } }"#;
        // New selection has only cardano-debug: yaci-devkit (registry) goes,
        // cardano-debug (registry) comes, my-own (user) stays untouched.
        let s = sel(vec![a(Role::OnChain, "aiken")], vec!["cardano-debug"]);
        let McpMerge::Write(text) = merge_mcp_json(Some(existing), &resolve(&s, &reg), &reg) else {
            panic!("expected a write");
        };
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&String> = v["mcpServers"].as_object().unwrap().keys().collect();
        assert_eq!(keys, vec!["cardano-debug", "my-own"]);
        assert_eq!(v["mcpServers"]["my-own"]["url"], "http://x");
        // Re-merging the result is a no-op.
        assert_eq!(
            merge_mcp_json(Some(&text), &resolve(&s, &reg), &reg),
            McpMerge::Unchanged
        );
    }

    #[test]
    fn merge_creates_removes_and_leaves_foreign_files_alone() {
        let reg = registry();
        let s = sel(vec![a(Role::OnChain, "aiken")], vec!["cardano-debug"]);
        let none = sel(vec![a(Role::OnChain, "aiken")], vec![]);
        // No file + servers → create (same bytes as a fresh render).
        let McpMerge::Write(text) = merge_mcp_json(None, &resolve(&s, &reg), &reg) else {
            panic!("expected a write");
        };
        assert_eq!(text, render_mcp_json(&resolve(&s, &reg), &reg));
        // No file + no servers → nothing.
        assert_eq!(merge_mcp_json(None, &[], &reg), McpMerge::Unchanged);
        // File holding only registry keys + no servers → remove the file.
        assert_eq!(
            merge_mcp_json(Some(&text), &resolve(&none, &reg), &reg),
            McpMerge::Remove
        );
        // Unparsable / non-object → never touched.
        assert_eq!(
            merge_mcp_json(Some("not json"), &resolve(&s, &reg), &reg),
            McpMerge::Unchanged
        );
        assert_eq!(
            merge_mcp_json(Some("[1]"), &resolve(&s, &reg), &reg),
            McpMerge::Unchanged
        );
    }

    #[test]
    fn newly_applicable_defaults_follow_role_changes() {
        let reg = registry();
        let before = [a(Role::OnChain, "aiken")];
        let after = [a(Role::OnChain, "aiken"), a(Role::Devnet, "yaci")];
        // yaci-devkit is applicable only after; whether it is returned depends
        // on its `default` flag — assert against the registry rather than a
        // hard-coded answer so flipping the flag doesn't break this test.
        let yaci_default = reg.mcp("yaci-devkit").unwrap().default;
        let got = newly_applicable_defaults(&reg, &before, &after);
        assert_eq!(got.contains(&"yaci-devkit".to_string()), yaci_default);
        assert!(
            !got.contains(&"cardano-debug".to_string()),
            "applied before too"
        );
        assert!(newly_applicable_defaults(&reg, &after, &before).is_empty());
    }

    #[test]
    fn render_is_deterministic() {
        let reg = registry();
        let s = sel(
            vec![a(Role::Devnet, "yaci")],
            vec!["yaci-devkit", "cardano-debug"],
        );
        let first = render_mcp_json(&resolve(&s, &reg), &reg);
        let second = render_mcp_json(&resolve(&s, &reg), &reg);
        assert_eq!(first, second);
    }
}
