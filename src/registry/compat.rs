//! Off-chain ↔ provider compatibility, decided purely from registry `[compat]`
//! data.
//!
//! An off-chain tool consumes one or more seams (wire protocols); providers —
//! the selected devnet and infra tools — serve seams. The selection is
//! compatible when some selected provider serves a seam the off-chain tool
//! consumes. Tools or providers that declare no seams impose no constraint, and
//! a tool that bundles its own devnet (`self_contained_devnet`) is incompatible
//! with any separate devnet.

use std::collections::HashSet;

use super::loader::Registry;
use super::types::{Role, RoleAssignment, Seam, ToolDef};

/// A provider tool (devnet or infra) referenced in a compatibility report.
#[derive(Debug, Clone)]
pub struct ProviderRef {
    pub id: String,
    pub name: String,
}

/// An incompatibility between the selected off-chain tool and the selected
/// providers.
#[derive(Debug, Clone)]
pub struct Incompatibility {
    pub off_chain_id: String,
    pub off_chain_name: String,
    /// Human-readable explanation of why the pairing can't work.
    pub reason: String,
    /// Whether the off-chain tool bundles its own devnet.
    pub self_hosted: bool,
    /// The selected providers that can't serve the off-chain tool.
    pub providers: Vec<ProviderRef>,
    /// Registry provider ids that would serve the off-chain tool (empty when it
    /// self-hosts).
    pub compatible_providers: Vec<String>,
    /// Off-chain tool ids that would work with the selected providers.
    pub compatible_off_chain: Vec<String>,
}

/// Check a selection for an off-chain ↔ provider incompatibility. Returns
/// `None` when the selection is coherent.
pub fn check(assignments: &[RoleAssignment], registry: &Registry) -> Option<Incompatibility> {
    let off_chain = assignment_tool(assignments, Role::OffChain, registry)?;
    let devnet = assignment_tool(assignments, Role::Devnet, registry);

    // A self-hosting tool makes any separate devnet redundant.
    if let Some(d) = devnet.filter(|_| off_chain.compat.self_contained_devnet) {
        return Some(Incompatibility {
            off_chain_id: off_chain.id.clone(),
            off_chain_name: off_chain.name.clone(),
            reason: format!(
                "{} provides its own embedded devnet, so a separate devnet isn't used",
                off_chain.name
            ),
            self_hosted: true,
            providers: vec![provider_ref(d)],
            compatible_providers: compatible_providers_for(off_chain, registry),
            compatible_off_chain: off_chain_for(&[d], registry),
        });
    }

    let providers: Vec<&ToolDef> = devnet
        .into_iter()
        .chain(
            assignments
                .iter()
                .filter(|a| a.role == Role::Infrastructure)
                .filter_map(|a| registry.get(&a.tool_id)),
        )
        .filter(|p| !p.compat.serves.is_empty())
        .collect();

    // With nothing to match against, the off-chain tool falls back to a public
    // provider per the interface contract.
    if off_chain.compat.consumes.is_empty() || providers.is_empty() {
        return None;
    }

    let served = served_seams(&providers);
    if off_chain.compat.consumes.iter().any(|s| served.contains(s)) {
        return None;
    }

    Some(Incompatibility {
        off_chain_id: off_chain.id.clone(),
        off_chain_name: off_chain.name.clone(),
        reason: format!(
            "{} speaks {}, which none of the selected providers serve",
            off_chain.name,
            seam_list(&off_chain.compat.consumes),
        ),
        self_hosted: off_chain.compat.self_contained_devnet,
        providers: providers.iter().copied().map(provider_ref).collect(),
        compatible_providers: compatible_providers_for(off_chain, registry),
        compatible_off_chain: off_chain_for(&providers, registry),
    })
}

fn assignment_tool<'a>(
    assignments: &[RoleAssignment],
    role: Role,
    registry: &'a Registry,
) -> Option<&'a ToolDef> {
    assignments
        .iter()
        .find(|a| a.role == role)
        .and_then(|a| registry.get(&a.tool_id))
}

fn provider_ref(tool: &ToolDef) -> ProviderRef {
    ProviderRef {
        id: tool.id.clone(),
        name: tool.name.clone(),
    }
}

fn served_seams(providers: &[&ToolDef]) -> HashSet<Seam> {
    providers
        .iter()
        .flat_map(|p| p.compat.serves.iter().copied())
        .collect()
}

/// Devnet/infra tool ids whose served seams overlap the off-chain tool's
/// consumed seams.
fn compatible_providers_for(off_chain: &ToolDef, registry: &Registry) -> Vec<String> {
    let mut ids: Vec<String> = registry
        .all_tools()
        .iter()
        .filter(|t| {
            t.roles.contains_key(&Role::Devnet) || t.roles.contains_key(&Role::Infrastructure)
        })
        .filter(|t| {
            t.compat
                .serves
                .iter()
                .any(|s| off_chain.compat.consumes.contains(s))
        })
        .map(|t| t.id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// Off-chain tool ids that consume any seam these providers serve.
fn off_chain_for(providers: &[&ToolDef], registry: &Registry) -> Vec<String> {
    let served = served_seams(providers);
    let mut ids: Vec<String> = registry
        .tools_for_role(Role::OffChain)
        .iter()
        .filter(|o| o.compat.consumes.iter().any(|s| served.contains(s)))
        .map(|o| o.id.clone())
        .collect();
    ids.sort();
    ids
}

fn seam_list(seams: &[Seam]) -> String {
    seams
        .iter()
        .copied()
        .map(Seam::label)
        .collect::<Vec<_>>()
        .join(" or ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::load().expect("registry should load")
    }

    fn a(role: Role, tool: &str) -> RoleAssignment {
        RoleAssignment {
            role,
            tool_id: tool.to_string(),
        }
    }

    #[test]
    fn blockfrost_offchain_with_yaci_is_compatible() {
        let reg = registry();
        let sel = vec![a(Role::OffChain, "evolution"), a(Role::Devnet, "yaci")];
        assert!(check(&sel, &reg).is_none());
    }

    #[test]
    fn mesh_with_dolos_infra_is_compatible() {
        let reg = registry();
        // Mesh speaks UTxORPC; Dolos infra serves it.
        let sel = vec![
            a(Role::OffChain, "meshjs"),
            a(Role::Infrastructure, "dolos"),
        ];
        assert!(check(&sel, &reg).is_none());
    }

    #[test]
    fn evolution_with_kupmios_is_compatible() {
        let reg = registry();
        let sel = vec![
            a(Role::OffChain, "evolution"),
            a(Role::Infrastructure, "kupo"),
            a(Role::Infrastructure, "ogmios"),
        ];
        assert!(check(&sel, &reg).is_none());
    }

    #[test]
    fn evolution_with_dolos_infra_is_incompatible() {
        let reg = registry();
        // Evolution has no UTxORPC provider, so Dolos (u5c only) can't feed it.
        let sel = vec![
            a(Role::OffChain, "evolution"),
            a(Role::Infrastructure, "dolos"),
        ];
        let inc = check(&sel, &reg).expect("evolution + dolos infra should be incompatible");
        assert_eq!(inc.off_chain_id, "evolution");
        assert!(inc.providers.iter().any(|p| p.id == "dolos"));
    }

    #[test]
    fn tx3_with_yaci_is_incompatible() {
        let reg = registry();
        let sel = vec![a(Role::OffChain, "tx3"), a(Role::Devnet, "yaci")];
        let inc = check(&sel, &reg).expect("tx3 + yaci should be incompatible");
        assert_eq!(inc.off_chain_id, "tx3");
        assert!(inc.self_hosted);
        assert!(inc.providers.iter().any(|p| p.id == "yaci"));
        assert!(inc.compatible_providers.is_empty());
    }

    #[test]
    fn tx3_with_incompatible_infra_is_incompatible() {
        let reg = registry();
        // Tx3 speaks TRP; Kupo can't serve it, and Tx3 self-hosts anyway.
        let sel = vec![a(Role::OffChain, "tx3"), a(Role::Infrastructure, "kupo")];
        let inc = check(&sel, &reg).expect("tx3 + kupo should be incompatible");
        assert!(inc.providers.iter().any(|p| p.id == "kupo"));
    }

    #[test]
    fn compatible_devnet_saves_an_incompatible_infra() {
        let reg = registry();
        // Yaci feeds Evolution (Blockfrost); an extra Dolos infra doesn't break it.
        let sel = vec![
            a(Role::OffChain, "evolution"),
            a(Role::Devnet, "yaci"),
            a(Role::Infrastructure, "dolos"),
        ];
        assert!(check(&sel, &reg).is_none());
    }

    #[test]
    fn supplementary_infra_imposes_no_constraint() {
        let reg = registry();
        // cardano-node serves no off-chain seam, so it never conflicts.
        let sel = vec![
            a(Role::OffChain, "evolution"),
            a(Role::Infrastructure, "cardano-node"),
        ];
        assert!(check(&sel, &reg).is_none());
    }

    #[test]
    fn tx3_without_provider_is_compatible() {
        let reg = registry();
        assert!(check(&[a(Role::OffChain, "tx3")], &reg).is_none());
    }

    #[test]
    fn no_offchain_is_compatible() {
        let reg = registry();
        assert!(check(&[a(Role::Devnet, "yaci")], &reg).is_none());
    }
}
