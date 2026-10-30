//! Context phase: turn a `Selection` into the data the templates render from.

use std::collections::BTreeMap;

use serde::Serialize;

use super::{ScaffoldError, planner};
use crate::contract;
use crate::registry::loader::Registry;
use crate::registry::types::{EnvMapping, Role, Selection, ToolDef};

/// `cardano-up`'s variable for the cardano-node socket, present whenever a
/// node-backed package is installed: the default source for the contract's
/// `NODE_SOCKET_PATH`. Providers with their own socket (e.g. dolos) override it
/// through their `[infra].env` mappings.
const CARDANO_UP_NODE_SOCKET_VAR: &str = "CARDANO_NODE_SOCKET_PATH";

/// Contract `.env` keys seeded empty in every project, for whichever component
/// provisions the endpoint to fill in.
const SEEDED_ENV_KEYS: &[&str] = &[
    contract::ENV_INDEXER_URL,
    contract::ENV_INDEXER_PORT,
    contract::ENV_NODE_SOCKET_PATH,
    contract::ENV_OGMIOS_URL,
    contract::ENV_TX_SUBMIT_URL,
    contract::ENV_DOLOS_GRPC_URL,
    contract::ENV_CARDANO_NODE_API_URL,
];

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// One component's tool, as seen by templates.
#[derive(Debug, Clone, Serialize)]
pub struct RoleContext {
    pub tool_id: String,
    pub tool_name: String,
    pub language: String,
    pub dir: String,
    /// The tool's official site, linked from the generated `AGENTS.md`.
    pub website: String,
}

impl RoleContext {
    fn new(tool: &ToolDef, dir: &str) -> Self {
        Self {
            tool_id: tool.id.clone(),
            tool_name: tool.name.clone(),
            language: tool.languages.first().cloned().unwrap_or_default(),
            dir: dir.to_string(),
            website: tool.website.clone(),
        }
    }
}

/// One selected infrastructure provider. All providers share the single
/// `infra/` driver component, so this carries what the driver needs rather
/// than a per-component [`RoleContext`].
#[derive(Debug, Clone, Serialize)]
pub struct InfraToolContext {
    pub tool_id: String,
    pub tool_name: String,
    /// The provider's official site, linked from the generated `AGENTS.md`.
    pub website: String,
    pub cardano_up_package: String,
    pub env: Vec<EnvMapping>,
}

/// Everything templates can reference; serialized into `MiniJinja`.
#[derive(Debug, Serialize)]
pub struct TemplateContext {
    pub project_name: String,
    pub network: String,

    pub has_on_chain: bool,
    pub has_off_chain: bool,
    pub has_infra: bool,
    pub has_devnet: bool,
    pub has_formal_methods: bool,
    /// One tool fills both on-chain and off-chain as a fused `protocol/`
    /// component. Then `has_on_chain` and `has_off_chain` are false.
    pub has_fullstack: bool,
    /// `blueprint/` is scaffolded (any non-infrastructure role is present).
    pub has_blueprint: bool,

    pub on_chain: Option<RoleContext>,
    pub off_chain: Option<RoleContext>,
    /// The fused component; when `Some`, `on_chain` and `off_chain` are `None`.
    pub fullstack: Option<RoleContext>,
    /// Infrastructure providers, sorted by tool id.
    pub infra_tools: Vec<InfraToolContext>,
    pub devnet: Option<RoleContext>,
    pub formal_methods: Option<RoleContext>,

    /// The `cardano-up` context name the infra component drives.
    pub infra_context_name: String,
    /// The `.env` mappings the infra driver writes, unique by key (see
    /// [`resolve_infra_env`]). Empty without infrastructure.
    pub infra_env: Vec<EnvMapping>,

    pub blueprint_path: String,
    /// Sorted by key, so the rendered `.env` is deterministic.
    pub env_vars: BTreeMap<String, String>,

    pub nix: bool,
    /// Packages for the top-level Nix shell, deduped, in canonical role order.
    pub nix_packages: Vec<String>,
    /// Directories of components whose tool ships its own flake; the root flake
    /// composes each one's dev shell. Deduped, in canonical role order.
    pub nix_component_flakes: Vec<String>,

    /// Whether the `spec/` folder is generated.
    pub spec: bool,
    /// Whether the worked example `spec/giftcard.md` is generated.
    pub spec_example: bool,
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Build the [`TemplateContext`] for `selection`.
///
/// Assignments are walked in canonical order so every list in the context is
/// independent of the order the selection was built in (determinism, §11).
///
/// # Errors
/// [`ScaffoldError::ToolNotFound`] for an unknown tool and
/// [`ScaffoldError::RoleMismatch`] for a tool assigned to a role it lacks.
pub fn build_context(
    selection: &Selection,
    registry: &Registry,
) -> Result<TemplateContext, ScaffoldError> {
    let fullstack_id = planner::fullstack_tool_id(selection, registry);
    let fullstack_id = fullstack_id.as_deref();

    let mut on_chain = None;
    let mut off_chain = None;
    let mut fullstack = None;
    let mut infra_tools = Vec::new();
    let mut devnet = None;
    let mut formal_methods = None;
    let mut nix_packages: Vec<String> = Vec::new();
    let mut nix_component_flakes: Vec<String> = Vec::new();

    for assignment in planner::canonical_assignments(selection) {
        let tool = planner::tool_for(registry, assignment)?;
        if !tool.roles.contains_key(&assignment.role) {
            return Err(planner::role_mismatch(assignment));
        }
        let dir = planner::component_dir(assignment, fullstack_id);

        // A self-contained tool's packages can't be built by the plain top-level
        // shell, so the root flake composes its component's own shell instead.
        if tool.nix_self_contained {
            if assignment.role != Role::Infrastructure
                && !nix_component_flakes.iter().any(|d| d == dir)
            {
                nix_component_flakes.push(dir.to_string());
            }
        } else {
            for pkg in &tool.nix_packages {
                if !nix_packages.contains(pkg) {
                    nix_packages.push(pkg.clone());
                }
            }
        }

        let fused = planner::is_fullstack_member(assignment, fullstack_id);
        match assignment.role {
            Role::Infrastructure => infra_tools.push(infra_tool_context(tool)),
            Role::OnChain if fused => fullstack = Some(RoleContext::new(tool, dir)),
            // The fused component was already built from the on-chain half.
            Role::OffChain if fused => {}
            Role::OnChain => on_chain = Some(RoleContext::new(tool, dir)),
            Role::OffChain => off_chain = Some(RoleContext::new(tool, dir)),
            Role::Devnet => devnet = Some(RoleContext::new(tool, dir)),
            Role::FormalMethods => formal_methods = Some(RoleContext::new(tool, dir)),
        }
    }

    Ok(TemplateContext {
        project_name: selection.project_name.clone(),
        network: selection.network.to_string(),

        has_on_chain: on_chain.is_some(),
        has_off_chain: off_chain.is_some(),
        has_infra: !infra_tools.is_empty(),
        has_devnet: devnet.is_some(),
        has_formal_methods: formal_methods.is_some(),
        has_fullstack: fullstack.is_some(),
        has_blueprint: planner::blueprint_dir_present(selection),

        infra_env: resolve_infra_env(&infra_tools),
        on_chain,
        off_chain,
        fullstack,
        infra_tools,
        devnet,
        formal_methods,

        infra_context_name: selection.project_name.clone(),

        blueprint_path: contract::BLUEPRINT_PATH.to_string(),
        env_vars: seeded_env_vars(selection),

        nix: selection.nix,
        nix_packages,
        nix_component_flakes,

        spec: selection.spec,
        spec_example: selection.spec && planner::spec_example_present(selection),
    })
}

fn infra_tool_context(tool: &ToolDef) -> InfraToolContext {
    let infra = tool
        .infra
        .as_ref()
        .expect("infra tool declares [infra] (validated at registry load)");
    InfraToolContext {
        tool_id: tool.id.clone(),
        tool_name: tool.name.clone(),
        website: tool.website.clone(),
        cardano_up_package: infra.cardano_up_package.clone(),
        env: infra.env.clone(),
    }
}

/// The `.env` mappings the infra driver writes: the cardano-up node-socket
/// default, then each provider's mappings in order. Unique by target key, with a
/// later mapping replacing an earlier one, so an explicit provider mapping wins
/// over the default (§5.4). Empty when there is no infrastructure.
fn resolve_infra_env(infra_tools: &[InfraToolContext]) -> Vec<EnvMapping> {
    if infra_tools.is_empty() {
        return Vec::new();
    }
    let node_socket_default = EnvMapping {
        from: CARDANO_UP_NODE_SOCKET_VAR.to_string(),
        to: contract::ENV_NODE_SOCKET_PATH.to_string(),
    };
    let mut env = vec![node_socket_default];
    for mapping in infra_tools.iter().flat_map(|t| &t.env) {
        upsert_env(&mut env, mapping.clone());
    }
    env
}

/// Insert `mapping`, replacing any existing mapping with the same target key.
fn upsert_env(env: &mut Vec<EnvMapping>, mapping: EnvMapping) {
    match env.iter_mut().find(|e| e.to == mapping.to) {
        Some(existing) => *existing = mapping,
        None => env.push(mapping),
    }
}

/// The `.env` vocabulary: the network, plus every contract endpoint seeded empty.
fn seeded_env_vars(selection: &Selection) -> BTreeMap<String, String> {
    let mut vars: BTreeMap<String, String> = SEEDED_ENV_KEYS
        .iter()
        .map(|key| ((*key).to_string(), String::new()))
        .collect();
    vars.insert(
        contract::ENV_NETWORK.to_string(),
        selection.network.to_string(),
    );
    vars
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{a, registry};
    use super::*;
    use crate::registry::types::{Network, RoleAssignment};

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
            a(Role::OnChain, "aiken"),
            a(Role::OffChain, "meshjs"),
            a(Role::Devnet, "yaci"),
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
            a(Role::OnChain, "scalus"),
            a(Role::OffChain, "scalus"),
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
        let sel = selection(vec![a(Role::OnChain, "aiken")]);
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
        let sel = selection(vec![a(Role::OffChain, "meshjs")]);
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
            a(Role::OnChain, "aiken"),
            a(Role::FormalMethods, "blaster"),
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert!(ctx.has_on_chain);
        assert!(ctx.has_formal_methods);
        assert_eq!(ctx.formal_methods.as_ref().unwrap().tool_id, "blaster");
        assert_eq!(ctx.formal_methods.as_ref().unwrap().dir, "formal-methods");
    }

    #[test]
    fn contract_constants_propagated() {
        let sel = selection(vec![a(Role::OnChain, "aiken")]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert_eq!(ctx.blueprint_path, "blueprint/plutus.json");
        assert_eq!(ctx.network, "preview");
        assert!(ctx.env_vars.contains_key("CARDANO_NETWORK"));
    }

    #[test]
    fn role_dirs_match_contract() {
        let sel = selection(vec![
            a(Role::OnChain, "aiken"),
            a(Role::OffChain, "meshjs"),
            a(Role::Devnet, "yaci"),
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();

        assert_eq!(ctx.on_chain.as_ref().unwrap().dir, "on-chain");
        assert_eq!(ctx.off_chain.as_ref().unwrap().dir, "off-chain");
        assert_eq!(ctx.devnet.as_ref().unwrap().dir, "devnet");
    }

    #[test]
    fn unknown_tool_errors() {
        let sel = selection(vec![a(Role::OnChain, "nonexistent")]);
        let result = build_context(&sel, &registry());
        assert!(matches!(result, Err(ScaffoldError::ToolNotFound { .. })));
    }

    #[test]
    fn role_mismatch_errors() {
        let sel = selection(vec![a(Role::Devnet, "aiken")]);
        let result = build_context(&sel, &registry());
        assert!(matches!(result, Err(ScaffoldError::RoleMismatch { .. })));
    }

    #[test]
    fn nix_packages_collected() {
        let sel = selection(vec![a(Role::OnChain, "aiken")]);
        let ctx = build_context(&sel, &registry()).unwrap();
        assert!(ctx.nix_packages.contains(&"aiken".to_string()));
    }

    #[test]
    fn nix_packages_order_is_assignment_order_independent() {
        // A fresh scaffold and the same selection built up via `add` (which
        // appends) must render the same `flake.nix` (determinism, §11).
        let on_chain = || a(Role::OnChain, "aiken");
        let off_chain = || a(Role::OffChain, "meshjs");

        let canonical =
            build_context(&selection(vec![on_chain(), off_chain()]), &registry()).unwrap();
        let reversed =
            build_context(&selection(vec![off_chain(), on_chain()]), &registry()).unwrap();

        assert_eq!(canonical.nix_packages, reversed.nix_packages);
        assert_eq!(canonical.nix_packages, vec!["aiken", "nodejs"]);
    }

    #[test]
    fn infra_context_aggregates_providers() {
        let sel = selection(vec![
            a(Role::Infrastructure, "ogmios"),
            a(Role::Infrastructure, "kupo"),
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
        let sel = selection(vec![a(Role::OnChain, "aiken")]);
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
            a(Role::OnChain, "scalus"),
            a(Role::OffChain, "scalus"),
        ]);
        let ctx = build_context(&sel, &registry()).unwrap();
        // sbt and jdk should appear only once each
        assert_eq!(ctx.nix_packages.iter().filter(|p| *p == "sbt").count(), 1);
        assert_eq!(ctx.nix_packages.iter().filter(|p| *p == "jdk").count(), 1);
    }
}
