use std::collections::BTreeMap;

use serde::Serialize;

use super::ScaffoldError;
use crate::contract;
use crate::registry::loader::Registry;
use crate::registry::types::{EnvMapping, Role, Selection};

/// `cardano-up`'s output var for the cardano-node UNIX socket path. It appears in
/// `cardano-up context env` whenever a node-backed package (kupo/ogmios/…) is
/// installed, so it is the default source for the contract's `NODE_SOCKET_PATH`.
/// A provider that supplies its own node socket (e.g. dolos) overrides this via
/// its own `[infra].env` mapping (infra-via-cardano-up proposal §5.4).
const CARDANO_UP_NODE_SOCKET_VAR: &str = "CARDANO_NODE_SOCKET_PATH";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Per-role information available to templates.
#[derive(Debug, Clone, Serialize)]
pub struct RoleContext {
    pub tool_id: String,
    pub tool_name: String,
    pub language: String,
    pub dir: String,
}

/// One selected infrastructure provider, available to the shared cardano-up
/// driver template. Infra tools aggregate into a single `infra/` component, so
/// this carries the data the driver needs: the `cardano-up` package id (for the
/// install set) and the tool's env mappings (folded into `infra_env`).
#[derive(Debug, Clone, Serialize)]
pub struct InfraToolContext {
    pub tool_id: String,
    pub tool_name: String,
    pub cardano_up_package: String,
    pub env: Vec<EnvMapping>,
}

/// The complete context passed to MiniJinja templates.
#[derive(Debug, Serialize)]
pub struct TemplateContext {
    pub project_name: String,
    pub network: String,

    pub has_on_chain: bool,
    pub has_off_chain: bool,
    pub has_infra: bool,
    pub has_devnet: bool,
    pub has_formal_methods: bool,

    /// True when the `blueprint/` directory is scaffolded: any non-infrastructure
    /// role is present (i.e. the project is not infrastructure-only). Mirrors the
    /// planner's blueprint predicate (TECH_SPEC §6.2).
    pub has_blueprint: bool,

    pub on_chain: Option<RoleContext>,
    pub off_chain: Option<RoleContext>,
    pub infra_tools: Vec<InfraToolContext>,
    pub devnet: Option<RoleContext>,
    pub formal_methods: Option<RoleContext>,

    /// The `cardano-up` context name the infra component drives (= project name).
    pub infra_context_name: String,
    /// Resolved, key-unique `.env` emissions for the infra driver: the base node
    /// socket default plus each provider's mappings, explicit-over-default, in
    /// canonical order (proposal §5.4). Empty when no infra role is present.
    pub infra_env: Vec<EnvMapping>,

    pub blueprint_path: String,
    /// Backed by a `BTreeMap` so it serializes in sorted-key order (determinism, §11).
    pub env_vars: BTreeMap<String, String>,

    pub nix: bool,
    pub nix_packages: Vec<String>,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Insert or replace an env mapping keyed by its `.env` target (`to`). A later
/// mapping for an already-present key replaces the earlier one, giving explicit
/// provider mappings precedence over the base default (proposal §5.4).
fn upsert_env(env: &mut Vec<EnvMapping>, mapping: EnvMapping) {
    match env.iter_mut().find(|e| e.to == mapping.to) {
        Some(existing) => *existing = mapping,
        None => env.push(mapping),
    }
}

/// Build a `TemplateContext` from a `Selection` and the tool `Registry`.
pub fn build_context(
    selection: &Selection,
    registry: &Registry,
) -> Result<TemplateContext, ScaffoldError> {
    let mut on_chain = None;
    let mut off_chain = None;
    let mut infra_tools = Vec::new();
    let mut devnet = None;
    let mut formal_methods = None;
    let mut nix_packages = Vec::new();

    for assignment in &selection.assignments {
        let tool =
            registry
                .get(&assignment.tool_id)
                .ok_or_else(|| ScaffoldError::ToolNotFound {
                    tool_id: assignment.tool_id.clone(),
                })?;

        if !tool.roles.contains_key(&assignment.role) {
            return Err(ScaffoldError::RoleMismatch {
                tool_id: assignment.tool_id.clone(),
                role: assignment.role.to_string(),
            });
        }

        for pkg in &tool.nix_packages {
            if !nix_packages.contains(pkg) {
                nix_packages.push(pkg.clone());
            }
        }

        // Infrastructure tools aggregate into a single component, so they carry
        // cardano-up data rather than a per-component RoleContext.
        if assignment.role == Role::Infrastructure {
            let infra = tool
                .infra
                .as_ref()
                .expect("infra tool must declare [infra] (validated at registry load)");
            infra_tools.push(InfraToolContext {
                tool_id: tool.id.clone(),
                tool_name: tool.name.clone(),
                cardano_up_package: infra.cardano_up_package.clone(),
                env: infra.env.clone(),
            });
            continue;
        }

        let rc = RoleContext {
            tool_id: tool.id.clone(),
            tool_name: tool.name.clone(),
            language: tool.languages.first().cloned().unwrap_or_default(),
            dir: assignment.role.dir().to_string(),
        };
        match assignment.role {
            Role::OnChain => on_chain = Some(rc),
            Role::OffChain => off_chain = Some(rc),
            Role::Devnet => devnet = Some(rc),
            Role::FormalMethods => formal_methods = Some(rc),
            Role::Infrastructure => unreachable!("infra handled above"),
        }
    }

    // Canonical order for the only multi-tool role: sorted by tool id (§11).
    // Mirrors the planner's infra ordering so context and plan agree.
    infra_tools.sort_by(|a, b| a.tool_id.cmp(&b.tool_id));

    // Resolve the infra `.env` emissions once, at generation time (§5.4): the
    // base node-socket default, then each provider's mappings in canonical order,
    // key-unique by `to` so an explicit mapping replaces the default.
    let mut infra_env: Vec<EnvMapping> = Vec::new();
    if !infra_tools.is_empty() {
        upsert_env(
            &mut infra_env,
            EnvMapping {
                from: CARDANO_UP_NODE_SOCKET_VAR.to_string(),
                to: contract::ENV_NODE_SOCKET_PATH.to_string(),
            },
        );
        for t in &infra_tools {
            for m in &t.env {
                upsert_env(&mut infra_env, m.clone());
            }
        }
    }

    let mut env_vars = BTreeMap::new();
    env_vars.insert(
        contract::ENV_NETWORK.to_string(),
        selection.network.to_string(),
    );
    env_vars.insert(contract::ENV_INDEXER_URL.to_string(), String::new());
    env_vars.insert(contract::ENV_INDEXER_PORT.to_string(), String::new());
    env_vars.insert(contract::ENV_NODE_SOCKET_PATH.to_string(), String::new());
    env_vars.insert(contract::ENV_OGMIOS_URL.to_string(), String::new());
    env_vars.insert(contract::ENV_TX_SUBMIT_URL.to_string(), String::new());
    env_vars.insert(contract::ENV_DOLOS_GRPC_URL.to_string(), String::new());
    env_vars.insert(
        contract::ENV_CARDANO_NODE_API_URL.to_string(),
        String::new(),
    );

    Ok(TemplateContext {
        project_name: selection.project_name.clone(),
        network: selection.network.to_string(),

        has_on_chain: on_chain.is_some(),
        has_off_chain: off_chain.is_some(),
        has_infra: !infra_tools.is_empty(),
        has_devnet: devnet.is_some(),
        has_formal_methods: formal_methods.is_some(),

        has_blueprint: on_chain.is_some()
            || off_chain.is_some()
            || devnet.is_some()
            || formal_methods.is_some(),

        on_chain,
        off_chain,
        infra_tools,
        devnet,
        formal_methods,

        infra_context_name: selection.project_name.clone(),
        infra_env,

        blueprint_path: contract::BLUEPRINT_PATH.to_string(),
        env_vars,

        nix: selection.nix,
        nix_packages,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::types::{Network, RoleAssignment};

    fn registry() -> Registry {
        Registry::load().expect("registry should load")
    }

    fn selection(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "test-project".to_string(),
            assignments,
            network: Network::Preview,
            nix: false,
        }
    }

    #[test]
    fn context_with_all_roles() {
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "meshjs".into(),
            },
            RoleAssignment {
                role: Role::Devnet,
                tool_id: "yaci".into(),
            },
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert!(ctx.has_on_chain);
        assert!(ctx.has_off_chain);
        assert!(!ctx.has_infra);
        assert!(ctx.has_devnet);

        assert_eq!(ctx.on_chain.as_ref().unwrap().tool_id, "aiken");
        assert_eq!(ctx.off_chain.as_ref().unwrap().tool_id, "meshjs");
        assert_eq!(ctx.devnet.as_ref().unwrap().tool_id, "yaci");
    }

    #[test]
    fn context_on_chain_only() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert!(ctx.has_on_chain);
        assert!(!ctx.has_off_chain);
        assert!(!ctx.has_infra);
        assert!(!ctx.has_devnet);
        assert!(!ctx.has_formal_methods);
        assert!(ctx.off_chain.is_none());
        assert!(ctx.devnet.is_none());
        assert!(ctx.formal_methods.is_none());
        assert!(ctx.infra_tools.is_empty());
    }

    #[test]
    fn has_flags_match_assignments() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OffChain,
            tool_id: "meshjs".into(),
        }]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert!(!ctx.has_on_chain);
        assert!(ctx.has_off_chain);
        assert!(!ctx.has_infra);
        assert!(!ctx.has_devnet);
        assert!(!ctx.has_formal_methods);
    }

    #[test]
    fn context_with_formal_methods() {
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
            RoleAssignment {
                role: Role::FormalMethods,
                tool_id: "blaster".into(),
            },
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert!(ctx.has_on_chain);
        assert!(ctx.has_formal_methods);
        assert_eq!(ctx.formal_methods.as_ref().unwrap().tool_id, "blaster");
        assert_eq!(ctx.formal_methods.as_ref().unwrap().dir, "formal-methods");
    }

    #[test]
    fn contract_constants_propagated() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert_eq!(ctx.blueprint_path, "blueprint/plutus.json");
        assert_eq!(ctx.network, "preview");
        assert!(ctx.env_vars.contains_key("CARDANO_NETWORK"));
    }

    #[test]
    fn role_dirs_match_contract() {
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "meshjs".into(),
            },
            RoleAssignment {
                role: Role::Devnet,
                tool_id: "yaci".into(),
            },
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert_eq!(ctx.on_chain.as_ref().unwrap().dir, "on-chain");
        assert_eq!(ctx.off_chain.as_ref().unwrap().dir, "off-chain");
        assert_eq!(ctx.devnet.as_ref().unwrap().dir, "devnet");
    }

    #[test]
    fn unknown_tool_errors() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "nonexistent".into(),
        }]);
        let result = build_context(&sel, &registry());
        assert!(matches!(result, Err(ScaffoldError::ToolNotFound { .. })));
    }

    #[test]
    fn role_mismatch_errors() {
        let sel = selection(vec![RoleAssignment {
            role: Role::Devnet,
            tool_id: "aiken".into(),
        }]);
        let result = build_context(&sel, &registry());
        assert!(matches!(result, Err(ScaffoldError::RoleMismatch { .. })));
    }

    #[test]
    fn nix_packages_collected() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let ctx = build_context(&sel, &registry()).unwrap();
        assert!(ctx.nix_packages.contains(&"aiken".to_string()));
    }

    #[test]
    fn infra_context_aggregates_providers() {
        let sel = selection(vec![
            RoleAssignment {
                role: Role::Infrastructure,
                tool_id: "ogmios".into(),
            },
            RoleAssignment {
                role: Role::Infrastructure,
                tool_id: "kupo".into(),
            },
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert!(ctx.has_infra);
        assert_eq!(ctx.infra_context_name, "test-project");
        // Canonical order: sorted by tool_id (kupo before ogmios).
        let ids: Vec<&str> = ctx.infra_tools.iter().map(|t| t.tool_id.as_str()).collect();
        assert_eq!(ids, vec!["kupo", "ogmios"]);

        // Resolved infra_env: base NODE_SOCKET_PATH default, then kupo→INDEXER_URL,
        // then ogmios→OGMIOS_URL — key-unique, in canonical order (§5.4).
        let env: Vec<(&str, &str)> = ctx
            .infra_env
            .iter()
            .map(|m| (m.to.as_str(), m.from.as_str()))
            .collect();
        assert_eq!(
            env,
            vec![
                ("NODE_SOCKET_PATH", "CARDANO_NODE_SOCKET_PATH"),
                ("INDEXER_URL", "KUPO_URL"),
                ("OGMIOS_URL", "OGMIOS_URL"),
            ]
        );
    }

    #[test]
    fn no_infra_env_without_infra() {
        let sel = selection(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        }]);
        let ctx = build_context(&sel, &registry()).unwrap();
        assert!(ctx.infra_env.is_empty());
        // OGMIOS_URL is still seeded into the always-present .env vocabulary.
        assert!(ctx.env_vars.contains_key("OGMIOS_URL"));
    }

    #[test]
    fn upsert_env_replaces_on_duplicate_key() {
        // Explicit-over-default: a later mapping for the same `to` replaces the
        // earlier one (the dolos-style node-socket override path, §5.4).
        let mut env = Vec::new();
        upsert_env(
            &mut env,
            EnvMapping {
                from: "CARDANO_NODE_SOCKET_PATH".into(),
                to: "NODE_SOCKET_PATH".into(),
            },
        );
        upsert_env(
            &mut env,
            EnvMapping {
                from: "DOLOS_SOCKET_PATH".into(),
                to: "NODE_SOCKET_PATH".into(),
            },
        );
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].from, "DOLOS_SOCKET_PATH");
    }

    #[test]
    fn nix_packages_deduped_across_tools() {
        // Scalus on-chain + scalus off-chain — same tool, same nix_packages
        let sel = selection(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "scalus".into(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "scalus".into(),
            },
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();
        // sbt and jdk should appear only once each
        assert_eq!(ctx.nix_packages.iter().filter(|p| *p == "sbt").count(), 1);
        assert_eq!(ctx.nix_packages.iter().filter(|p| *p == "jdk").count(), 1);
    }
}
