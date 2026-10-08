//! Serializable, presentation-agnostic views of the registry.
//!
//! These are the single source the `list` subcommand renders from
//! (ARCHITECTURE §7.3) — pure data derived from the registry, with no knowledge
//! of colors or tables. The JSON shape matches TECH_SPEC §8.

use serde::Serialize;

use super::loader::Registry;
use super::types::{Link, Role};

/// A role, as exposed to consumers (TECH_SPEC §8). `multiple` is `true` only for
/// the role that may be filled by several tools at once (Infrastructure).
#[derive(Debug, Clone, Serialize)]
pub struct RoleView {
    pub id: &'static str,
    pub dir: &'static str,
    pub display: String,
    pub multiple: bool,
}

/// A tool, as exposed to consumers. Field set is intentionally user-facing —
/// no `system_deps`/`detect` (those are the doctor's concern, TECH_SPEC §8).
#[derive(Debug, Clone, Serialize)]
pub struct ToolView {
    pub id: String,
    pub name: String,
    pub description: String,
    pub website: String,
    pub community: Vec<Link>,
    pub languages: Vec<String>,
    pub roles: Vec<String>,
    /// `true` when the tool declares a `[fullstack]` template, i.e. it can fill
    /// on-chain + off-chain as one `protocol/` component (`--fullstack <tool>`).
    /// A capability, not a role — never appears in `roles`.
    pub fullstack: bool,
    /// `true` when the tool is **experimental**: it generates but is not yet
    /// build-green. Additive field (no `schema_version` bump); lets agents
    /// filter or warn before selecting it (selecting one needs
    /// `--allow-experimental`).
    pub experimental: bool,
}

/// All roles in canonical (`Role::ALL`) order.
pub fn role_views() -> Vec<RoleView> {
    Role::ALL
        .iter()
        .map(|role| RoleView {
            id: role.as_kebab(),
            dir: role.dir(),
            display: role.to_string(),
            multiple: role.multiple(),
        })
        .collect()
}

/// All tools, sorted by id; each tool's roles sorted (kebab).
pub fn tool_views(registry: &Registry) -> Vec<ToolView> {
    let mut tools: Vec<ToolView> = registry
        .all_tools()
        .iter()
        .map(|tool| {
            let mut roles: Vec<String> = tool
                .roles
                .keys()
                .map(|r| r.as_kebab().to_string())
                .collect();
            roles.sort();
            ToolView {
                id: tool.id.clone(),
                name: tool.name.clone(),
                description: tool.description.clone(),
                website: tool.website.clone(),
                community: tool.community.clone(),
                languages: tool.languages.clone(),
                roles,
                fullstack: tool.fullstack.is_some(),
                experimental: tool.experimental,
            }
        })
        .collect();
    tools.sort_by(|a, b| a.id.cmp(&b.id));
    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract;

    fn registry() -> Registry {
        Registry::load().expect("registry loads")
    }

    #[test]
    fn role_views_canonical_order_and_dirs() {
        let views = role_views();
        assert_eq!(views.len(), 5);
        let ids: Vec<&str> = views.iter().map(|r| r.id).collect();
        assert_eq!(
            ids,
            vec![
                "on-chain",
                "off-chain",
                "infrastructure",
                "devnet",
                "formal-methods"
            ]
        );
        // dirs match the interface contract
        let infra = views.iter().find(|r| r.id == "infrastructure").unwrap();
        assert_eq!(infra.dir, contract::DIR_INFRA);
        assert_eq!(infra.display, "Infrastructure");
    }

    #[test]
    fn only_infrastructure_is_multiple() {
        for r in role_views() {
            assert_eq!(
                r.multiple,
                r.id == "infrastructure",
                "unexpected multiple flag for {}",
                r.id
            );
        }
    }

    #[test]
    fn tool_views_sorted_by_id_with_sorted_roles() {
        let views = tool_views(&registry());
        let ids: Vec<&str> = views.iter().map(|t| t.id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "tools must be id-sorted");

        // scalus fills two roles; they must be sorted (off-chain before on-chain).
        let scalus = views.iter().find(|t| t.id == "scalus").unwrap();
        assert_eq!(scalus.roles, vec!["off-chain", "on-chain"]);
        // scalus declares a [fullstack] template; the capability is surfaced but
        // never leaks into `roles`.
        assert!(scalus.fullstack);
        assert!(
            !scalus
                .roles
                .iter()
                .any(|r| r == "protocol" || r == "fullstack")
        );

        let aiken = views.iter().find(|t| t.id == "aiken").unwrap();
        assert_eq!(aiken.roles, vec!["on-chain"]);
        assert!(!aiken.fullstack);
    }

    #[test]
    fn tool_views_surface_experimental_flag() {
        let views = tool_views(&registry());
        // Experimental status is carried through to the discovery surface so
        // `list` can flag not-yet-build-green tools.
        let blaster = views.iter().find(|t| t.id == "blaster").unwrap();
        assert!(blaster.experimental);
        let aiken = views.iter().find(|t| t.id == "aiken").unwrap();
        assert!(!aiken.experimental);
    }
}
