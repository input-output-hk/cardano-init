//! Era coherence of a selection. Each tool is parked on the ledger era its
//! template targets; a selection whose tools span several eras may not work
//! together, so it needs an explicit opt-in.

use std::collections::BTreeMap;

use super::compat::ProviderRef;
use super::loader::Registry;
use super::types::{Era, RoleAssignment};

/// The selected tools, grouped by the era each is parked on. Only built when
/// more than one era is present.
#[derive(Debug, Clone)]
pub struct EraMismatch {
    pub groups: BTreeMap<Era, Vec<ProviderRef>>,
}

/// `None` when every selected tool targets the same era.
pub fn check(assignments: &[RoleAssignment], registry: &Registry) -> Option<EraMismatch> {
    let mut groups: BTreeMap<Era, Vec<ProviderRef>> = BTreeMap::new();
    for tool in assignments.iter().filter_map(|a| registry.get(&a.tool_id)) {
        let tools = groups.entry(tool.era).or_default();
        if !tools.iter().any(|t| t.id == tool.id) {
            tools.push(ProviderRef {
                id: tool.id.clone(),
                name: tool.name.clone(),
            });
        }
    }
    (groups.len() > 1).then_some(EraMismatch { groups })
}

/// A change whose result mixes eras.
#[derive(Debug)]
pub enum EraChange {
    NewEra(EraMismatch),
    AlreadyMixed(EraMismatch),
}

/// Check changing `old` into `new`; `None` when `new` has a single era.
pub fn check_change(
    old: &[RoleAssignment],
    new: &[RoleAssignment],
    registry: &Registry,
) -> Option<EraChange> {
    let mismatch = check(new, registry)?;
    let known = |era: &Era| {
        old.iter()
            .filter_map(|a| registry.get(&a.tool_id))
            .any(|tool| tool.era == *era)
    };
    Some(if mismatch.groups.keys().all(known) {
        EraChange::AlreadyMixed(mismatch)
    } else {
        EraChange::NewEra(mismatch)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::types::Role;

    fn a(role: Role, tool: &str) -> RoleAssignment {
        RoleAssignment {
            role,
            tool_id: tool.to_string(),
        }
    }

    fn registry_with(dijkstra: &[&str]) -> Registry {
        let mut reg = Registry::load().unwrap();
        for id in dijkstra {
            reg.set_era_for_test(id, Era::Dijkstra);
        }
        reg
    }

    #[test]
    fn same_era_is_coherent() {
        let reg = registry_with(&[]);
        let sel = [
            a(Role::OnChain, "aiken"),
            a(Role::OffChain, "meshjs"),
            a(Role::Devnet, "yaci"),
        ];
        assert!(check(&sel, &reg).is_none());
    }

    #[test]
    fn mixed_eras_are_grouped() {
        let reg = registry_with(&["aiken"]);
        let sel = [
            a(Role::OnChain, "aiken"),
            a(Role::OffChain, "meshjs"),
            a(Role::Infrastructure, "kupo"),
        ];
        let mismatch = check(&sel, &reg).expect("aiken on dijkstra vs conway rest");
        let ids = |era| -> Vec<&str> {
            mismatch.groups[&era]
                .iter()
                .map(|t| t.id.as_str())
                .collect()
        };
        assert_eq!(ids(Era::Dijkstra), vec!["aiken"]);
        assert_eq!(ids(Era::Conway), vec!["meshjs", "kupo"]);
    }

    #[test]
    fn fullstack_tool_is_listed_once() {
        let reg = registry_with(&["scalus"]);
        let sel = [
            a(Role::OnChain, "scalus"),
            a(Role::OffChain, "scalus"),
            a(Role::Devnet, "yaci"),
        ];
        let mismatch = check(&sel, &reg).unwrap();
        assert_eq!(mismatch.groups[&Era::Dijkstra].len(), 1);
    }

    #[test]
    fn change_tells_a_new_era_from_an_existing_mix() {
        let reg = registry_with(&["aiken"]);
        let aiken = a(Role::OnChain, "aiken");
        let mesh = a(Role::OffChain, "meshjs");
        let yaci = a(Role::Devnet, "yaci");
        let conway = [mesh.clone()];
        let mixed = [aiken.clone(), mesh.clone()];
        let mixed_more = [aiken.clone(), mesh, yaci];

        let change = |old: &[_], new: &[_]| check_change(old, new, &reg);
        assert!(matches!(
            change(&conway, &mixed),
            Some(EraChange::NewEra(_))
        ));
        assert!(matches!(
            change(&mixed, &mixed_more),
            Some(EraChange::AlreadyMixed(_))
        ));
        assert!(matches!(
            change(&mixed_more, &mixed),
            Some(EraChange::AlreadyMixed(_))
        ));
        assert!(change(&mixed, &[aiken]).is_none());
    }

    #[test]
    fn empty_selection_is_coherent() {
        assert!(check(&[], &registry_with(&[])).is_none());
    }
}
