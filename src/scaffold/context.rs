use std::collections::BTreeMap;

use serde::Serialize;

use super::{ScaffoldError, planner};
use crate::contract;
use crate::registry::loader::Registry;
use crate::registry::types::{EnvMapping, Role, Selection};

/// `cardano-up`'s output var for the cardano-node UNIX socket path. It appears in
/// `cardano-up context env` whenever a node-backed package (kupo/ogmios/…) is
/// installed, so it is the default source for the contract's `NODE_SOCKET_PATH`.
/// A provider that supplies its own node socket (e.g. dolos) overrides this via
/// its own `[infra].env` mapping (TECH_SPEC §3.2).
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
    /// The tool's official site/docs URL (registry `website`). Surfaced in the
    /// generated `AGENTS.md` so agents jump straight to authoritative docs.
    pub website: String,
}

/// One selected infrastructure provider, available to the shared cardano-up
/// driver template. Infra tools aggregate into a single `infra/` component, so
/// this carries the data the driver needs: the `cardano-up` package id (for the
/// install set) and the tool's env mappings (folded into `infra_env`).
#[derive(Debug, Clone, Serialize)]
pub struct InfraToolContext {
    pub tool_id: String,
    pub tool_name: String,
    /// The provider's official site/docs URL (registry `website`), surfaced in
    /// the generated `AGENTS.md`.
    pub website: String,
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
    /// True when a single tool fills both on-chain and off-chain and they collapse
    /// into one `protocol/` component (TECH_SPEC §3.4). When true, `has_on_chain`
    /// and `has_off_chain` are false (the two roles are represented by `fullstack`).
    pub has_fullstack: bool,

    /// True when the `blueprint/` directory is scaffolded: any non-infrastructure
    /// role is present (i.e. the project is not infrastructure-only). Mirrors the
    /// planner's blueprint predicate (TECH_SPEC §6.2).
    pub has_blueprint: bool,

    pub on_chain: Option<RoleContext>,
    pub off_chain: Option<RoleContext>,
    /// The fused on-chain+off-chain component (`dir = "protocol"`). `Some` only
    /// when one tool fills both roles and declares a `[fullstack]` template; then
    /// `on_chain`/`off_chain` are `None`.
    pub fullstack: Option<RoleContext>,
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
    /// Component directories whose tool ships its own Nix flake
    /// (`nix_self_contained`). The top-level flake references each as a
    /// `path:./<dir>` input and pulls its dev shell in via `inputsFrom`, so the
    /// whole toolchain composes into the root shell. Canonical (`Role::ALL`)
    /// order, deduped. Empty unless a self-contained tool is selected.
    pub nix_component_flakes: Vec<String>,

    /// Whether the `spec/` protocol-specification folder is generated.
    pub spec: bool,
    /// Whether `spec/giftcard.md` (the worked example) is generated.
    pub spec_example: bool,
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
    let mut fullstack = None;
    let mut infra_tools = Vec::new();
    let mut devnet = None;
    let mut formal_methods = None;
    let mut nix_packages = Vec::new();
    // (canonical index, component dir) for tools shipping their own flake.
    let mut nix_flake_components: Vec<(usize, String)> = Vec::new();

    // When one tool fills both on-chain and off-chain via a `[fullstack]` template,
    // the two collapse into a single `protocol/` component. Derived here (and in
    // the planner) from the same shared helper so context and plan agree.
    let fullstack_id = super::planner::fullstack_tool_id(selection, registry);

    // Walk assignments in canonical order (Role::ALL, then tool id) rather than
    // the order they were supplied/added, so every order-sensitive field derived
    // below is reproducible for a given selection (§11). Without this, `nix_packages`
    // — accumulated in first-encountered order — would differ between a fresh
    // scaffold (assignments already canonical) and the same selection reached via
    // `add` (which appends), yielding a byte-different `flake.nix`.
    let mut ordered_assignments: Vec<&_> = selection.assignments.iter().collect();
    ordered_assignments.sort_by_key(|a| {
        let role_idx = Role::ALL
            .iter()
            .position(|r| *r == a.role)
            .expect("role is in Role::ALL");
        (role_idx, a.tool_id.clone())
    });

    for assignment in ordered_assignments {
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

        // A self-contained tool ships its own component-local flake (emitted
        // under `--nix`), so its packages must not pollute the naive top-level
        // shell, which cannot build it. See `ToolDef::nix_self_contained`.
        if !tool.nix_self_contained {
            for pkg in &tool.nix_packages {
                if !nix_packages.contains(pkg) {
                    nix_packages.push(pkg.clone());
                }
            }
        } else if assignment.role != Role::Infrastructure {
            // Self-contained tool: record its component dir so the top-level
            // flake composes that component's own dev shell (`inputsFrom`).
            let is_fullstack_member = fullstack_id.as_deref() == Some(assignment.tool_id.as_str())
                && matches!(assignment.role, Role::OnChain | Role::OffChain);
            let dir = if is_fullstack_member {
                contract::DIR_PROTOCOL.to_string()
            } else {
                assignment.role.dir().to_string()
            };
            let idx = Role::ALL
                .iter()
                .position(|r| *r == assignment.role)
                .expect("role is in Role::ALL");
            if !nix_flake_components.iter().any(|(_, d)| *d == dir) {
                nix_flake_components.push((idx, dir));
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
                website: tool.website.clone(),
                cardano_up_package: infra.cardano_up_package.clone(),
                env: infra.env.clone(),
            });
            continue;
        }

        // Fullstack collapse: the on-chain + off-chain assignments of this tool
        // become one `protocol/` component. Build its RoleContext once (on the
        // on-chain assignment); the off-chain half adds nothing further (its
        // nix_packages were already unioned above).
        if fullstack_id.as_deref() == Some(assignment.tool_id.as_str())
            && matches!(assignment.role, Role::OnChain | Role::OffChain)
        {
            if assignment.role == Role::OnChain {
                fullstack = Some(RoleContext {
                    tool_id: tool.id.clone(),
                    tool_name: tool.name.clone(),
                    language: tool.languages.first().cloned().unwrap_or_default(),
                    dir: contract::DIR_PROTOCOL.to_string(),
                    website: tool.website.clone(),
                });
            }
            continue;
        }

        let rc = RoleContext {
            tool_id: tool.id.clone(),
            tool_name: tool.name.clone(),
            language: tool.languages.first().cloned().unwrap_or_default(),
            dir: assignment.role.dir().to_string(),
            website: tool.website.clone(),
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
        has_fullstack: fullstack.is_some(),

        has_blueprint: on_chain.is_some()
            || off_chain.is_some()
            || fullstack.is_some()
            || devnet.is_some()
            || formal_methods.is_some(),

        on_chain,
        off_chain,
        fullstack,
        infra_tools,
        devnet,
        formal_methods,

        infra_context_name: selection.project_name.clone(),
        infra_env,

        blueprint_path: contract::BLUEPRINT_PATH.to_string(),
        env_vars,

        spec: selection.spec,
        spec_example: selection.spec && planner::spec_example_present(selection),
        nix: selection.nix,
        nix_packages,
        nix_component_flakes: {
            // Canonical (Role::ALL) order; dirs already deduped above.
            nix_flake_components.sort_by_key(|(idx, _)| *idx);
            nix_flake_components
                .into_iter()
                .map(|(_, dir)| dir)
                .collect()
        },
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
            spec: true,
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
    fn context_fullstack_collapses_roles() {
        // Scalus on both roles + [fullstack] template → the context represents one
        // `protocol` component, not separate on-chain/off-chain contexts.
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

        assert!(ctx.has_fullstack);
        assert!(!ctx.has_on_chain);
        assert!(!ctx.has_off_chain);
        assert!(ctx.on_chain.is_none());
        assert!(ctx.off_chain.is_none());
        let fs = ctx.fullstack.as_ref().expect("fullstack context");
        assert_eq!(fs.tool_id, "scalus");
        assert_eq!(fs.dir, "protocol");
        assert_eq!(fs.language, "scala");
        // Protocol is a blueprint producer/consumer, so the dir is present.
        assert!(ctx.has_blueprint);
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
    fn nix_packages_order_is_assignment_order_independent() {
        // The same logical selection reached in a different assignment order (e.g.
        // a fresh scaffold vs. one built up via `add`) must yield the identical
        // `nix_packages` list, or `flake.nix` would differ byte-for-byte for equal
        // inputs (determinism contract, §11). Regression test for #65.
        let on_chain = || RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".into(),
        };
        let off_chain = || RoleAssignment {
            role: Role::OffChain,
            tool_id: "meshjs".into(),
        };

        let canonical =
            build_context(&selection(vec![on_chain(), off_chain()]), &registry()).unwrap();
        // Reversed order, as an `add` of the on-chain role onto an off-chain-only
        // project would produce (append, not canonical insert).
        let reversed =
            build_context(&selection(vec![off_chain(), on_chain()]), &registry()).unwrap();

        assert_eq!(canonical.nix_packages, reversed.nix_packages);
        // And the canonical order is Role::ALL order: on-chain (aiken) before
        // off-chain (nodejs).
        assert_eq!(canonical.nix_packages, vec!["aiken", "nodejs"]);
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
