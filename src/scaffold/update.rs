//! In-place project updates: apply [`Mutation`]s to a `Selection` and diff the
//! old and new selections into an [`UpdatePlan`] (tech spec §16).
//!
//! Pure: the diff works at component-directory granularity and only renders from
//! embedded assets; [`super::writer::apply_update`] performs the I/O. Because
//! components are standalone, a directory whose tool is unchanged never appears
//! in the plan, so the user's code there is never touched.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::renderer::RenderedFile;
use super::{ScaffoldError, context, planner, renderer};
use crate::registry::loader::Registry;
use crate::registry::types::{Role, RoleAssignment, Selection};

// ---------------------------------------------------------------------------
// Mutations
// ---------------------------------------------------------------------------

/// A single edit to a project's `Selection`.
#[derive(Debug, Clone)]
pub enum Mutation {
    /// Assign a tool to a role: replaces the current tool of a single-tool role,
    /// or appends an infrastructure provider (ignored if already present). A
    /// fullstack tool is two `Add`s (on-chain and off-chain, same tool).
    Add(RoleAssignment),
    /// Remove a role entirely (all infrastructure providers, if Infrastructure).
    Remove(Role),
    /// Remove a single infrastructure provider by tool id.
    RemoveInfra(String),
}

/// The selection that results from applying `mutation` to `old`.
pub fn apply(old: &Selection, mutation: &Mutation) -> Selection {
    let mut assignments = old.assignments.clone();
    match mutation {
        Mutation::Add(new_asg) if new_asg.role == Role::Infrastructure => {
            let already = assignments
                .iter()
                .any(|a| a.role == Role::Infrastructure && a.tool_id == new_asg.tool_id);
            if !already {
                assignments.push(new_asg.clone());
            }
        }
        Mutation::Add(new_asg) => {
            assignments.retain(|a| a.role != new_asg.role);
            assignments.push(new_asg.clone());
        }
        Mutation::Remove(role) => assignments.retain(|a| a.role != *role),
        Mutation::RemoveInfra(tool_id) => {
            assignments.retain(|a| !(a.role == Role::Infrastructure && a.tool_id == *tool_id));
        }
    }
    Selection {
        project_name: old.project_name.clone(),
        assignments,
        network: old.network,
        nix: old.nix,
        spec: old.spec,
    }
}

/// Apply `mutations` left to right.
pub fn apply_all(old: &Selection, mutations: &[Mutation]) -> Selection {
    mutations.iter().fold(old.clone(), |acc, m| apply(&acc, m))
}

// ---------------------------------------------------------------------------
// Change set
// ---------------------------------------------------------------------------

/// What happens to one component directory between the old and new selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotOp {
    /// A component that did not exist: create its directory and files.
    Create(PathBuf),
    /// A component that is going away: remove its directory.
    Remove(PathBuf),
    /// A component whose tool changed: remove the old directory, write the new.
    Replace(PathBuf),
    /// The aggregated `infra/` component whose provider set changed: overwrite
    /// its managed files in place (the directory stays).
    RerenderInfra(PathBuf),
}

impl SlotOp {
    /// The directory this op acts on.
    pub fn dir(&self) -> &Path {
        match self {
            SlotOp::Create(p)
            | SlotOp::Remove(p)
            | SlotOp::Replace(p)
            | SlotOp::RerenderInfra(p) => p,
        }
    }
}

/// The full plan for turning the old project into the new one.
#[derive(Debug)]
pub struct UpdatePlan {
    /// Per-component-directory operations.
    pub slot_ops: Vec<SlotOp>,
    /// Files for directories being created, replaced, or re-rendered.
    pub creates: Vec<RenderedFile>,
    /// The re-rendered shared top-level layer (content-diffed at write time).
    pub shared_files: Vec<RenderedFile>,
    /// Shared files that no longer apply to the new selection.
    pub shared_removals: Vec<PathBuf>,
}

/// The tool(s) occupying one component directory.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SlotTool {
    Single(String),
    Infra(BTreeSet<String>),
}

/// The selection's occupied component directories, laid out as the planner
/// lays them out (fullstack pair in `protocol/`, all providers in `infra/`).
fn slots(sel: &Selection, registry: &Registry) -> BTreeMap<String, SlotTool> {
    let fullstack_id = planner::fullstack_tool_id(sel, registry);
    let mut map: BTreeMap<String, SlotTool> = BTreeMap::new();
    for asg in &sel.assignments {
        let dir = planner::component_dir(asg, fullstack_id.as_deref()).to_string();
        if asg.role == Role::Infrastructure {
            let slot = map
                .entry(dir)
                .or_insert_with(|| SlotTool::Infra(BTreeSet::new()));
            if let SlotTool::Infra(providers) = slot {
                providers.insert(asg.tool_id.clone());
            }
        } else {
            map.insert(dir, SlotTool::Single(asg.tool_id.clone()));
        }
    }
    map
}

/// The tool id(s) occupying each component directory in `sel`, sorted within
/// each directory.
pub fn slot_tools(sel: &Selection, registry: &Registry) -> BTreeMap<String, Vec<String>> {
    slots(sel, registry)
        .into_iter()
        .map(|(dir, tool)| {
            let ids = match tool {
                SlotTool::Single(id) => vec![id],
                SlotTool::Infra(set) => set.into_iter().collect(),
            };
            (dir, ids)
        })
        .collect()
}

/// Compute the change set that turns the project for `old` into the one for `new`.
///
/// # Errors
/// Any [`ScaffoldError`] from planning, context building, or rendering `new`.
pub fn plan_update(
    old: &Selection,
    new: &Selection,
    registry: &Registry,
) -> Result<UpdatePlan, ScaffoldError> {
    let old_slots = slots(old, registry);
    let new_slots = slots(new, registry);

    let dirs: BTreeSet<&String> = old_slots.keys().chain(new_slots.keys()).collect();
    let mut slot_ops = Vec::new();
    for dir in dirs {
        let path = PathBuf::from(dir);
        match (old_slots.get(dir), new_slots.get(dir)) {
            (None, Some(_)) => slot_ops.push(SlotOp::Create(path)),
            (Some(_), None) => slot_ops.push(SlotOp::Remove(path)),
            (Some(o), Some(n)) if o == n => {} // KEEP — untouched
            (Some(SlotTool::Infra(_)), Some(SlotTool::Infra(_))) => {
                slot_ops.push(SlotOp::RerenderInfra(path));
            }
            (Some(_), Some(_)) => slot_ops.push(SlotOp::Replace(path)),
            // `dirs` is the union of both key sets.
            (None, None) => {}
        }
    }

    let plan = planner::plan(new, registry)?;
    let ctx = context::build_context(new, registry)?;
    let files = renderer::render(&plan, &ctx)?;

    let write_dirs: BTreeSet<PathBuf> = slot_ops
        .iter()
        .filter(|op| !matches!(op, SlotOp::Remove(_)))
        .map(|op| op.dir().to_path_buf())
        .collect();

    let mut shared_files = Vec::new();
    let mut creates = Vec::new();
    for file in files {
        if is_shared_layer(&file.dest) {
            shared_files.push(file);
        } else if top_dir(&file.dest).is_some_and(|top| write_dirs.contains(&top)) {
            creates.push(file);
        }
        // Anything else belongs to a kept component or `spec/`: left untouched.
    }

    let mut shared_removals = Vec::new();
    if planner::blueprint_dir_present(old) && !planner::blueprint_dir_present(new) {
        shared_removals.push(PathBuf::from(planner::BLUEPRINT_MARKER));
    }
    // Not reachable through role mutations today, but keeps the diff honest.
    if old.nix && !new.nix {
        shared_removals.extend(
            planner::NIX_FILES
                .iter()
                .map(|(dest, _)| PathBuf::from(dest)),
        );
    }

    Ok(UpdatePlan {
        slot_ops,
        creates,
        shared_files,
        shared_removals,
    })
}

fn is_shared_layer(dest: &Path) -> bool {
    planner::shared_layer_dests().any(|shared| dest == Path::new(shared))
}

/// The top-level directory of a nested path (`on-chain/aiken.toml` ->
/// `on-chain`); `None` for a root-level file.
fn top_dir(dest: &Path) -> Option<PathBuf> {
    let mut components = dest.components();
    let first = components.next()?;
    components.next().map(|_| PathBuf::from(first.as_os_str()))
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{a, registry};
    use super::*;
    use crate::registry::types::Network;

    fn sel(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "proj".into(),
            assignments,
            network: Network::Preview,
            nix: false,
            spec: true,
        }
    }

    fn dirs_touched(plan: &UpdatePlan) -> Vec<String> {
        plan.slot_ops
            .iter()
            .map(|op| op.dir().to_string_lossy().into_owned())
            .collect()
    }

    fn creates_under(plan: &UpdatePlan, dir: &str) -> bool {
        plan.creates.iter().any(|f| f.dest.starts_with(dir))
    }

    #[test]
    fn add_offchain_creates_only_that_slot() {
        let reg = registry();
        let old = sel(vec![a(Role::OnChain, "aiken")]);
        let new = apply(&old, &Mutation::Add(a(Role::OffChain, "meshjs")));
        let plan = plan_update(&old, &new, &reg).unwrap();

        assert_eq!(
            plan.slot_ops,
            vec![SlotOp::Create(PathBuf::from("off-chain"))]
        );
        assert!(creates_under(&plan, "off-chain"));
        // KEEP guarantee: on-chain is untouched — no op, no re-render.
        assert!(!dirs_touched(&plan).contains(&"on-chain".to_string()));
        assert!(!creates_under(&plan, "on-chain"));
        // Shared layer always re-renders (root Justfile now wires off-chain).
        assert!(
            plan.shared_files
                .iter()
                .any(|f| f.dest.to_str() == Some("Justfile"))
        );
    }

    #[test]
    fn spec_is_user_owned_and_never_rewritten() {
        // spec/ is the user's design doc once generated: add/remove neither
        // rewrite nor delete it.
        let reg = registry();
        let old = sel(vec![a(Role::OnChain, "aiken")]);
        for new in [
            apply(&old, &Mutation::Add(a(Role::OffChain, "meshjs"))),
            apply(&old, &Mutation::Add(a(Role::Devnet, "yaci"))),
        ] {
            assert!(new.spec, "spec carries over");
            let plan = plan_update(&old, &new, &reg).unwrap();
            assert!(!creates_under(&plan, "spec"));
            assert!(!plan.shared_files.iter().any(|f| f.dest.starts_with("spec")));
            assert!(!plan.shared_removals.iter().any(|p| p.starts_with("spec")));
        }
    }

    #[test]
    fn add_protocol_role_does_not_link_missing_spec_example() {
        let reg = registry();
        let old = sel(vec![a(Role::Devnet, "yaci")]);
        let new = apply(&old, &Mutation::Add(a(Role::OnChain, "aiken")));
        let plan = plan_update(&old, &new, &reg).unwrap();
        let agents = plan
            .shared_files
            .iter()
            .find(|f| f.dest == Path::new("AGENTS.md"))
            .expect("AGENTS.md is re-rendered");
        let agents = std::str::from_utf8(&agents.content).unwrap();
        assert!(agents.contains("[`spec/`](spec/)"));
        assert!(!agents.contains("spec/giftcard.md"));
    }

    #[test]
    fn remove_offchain_leaves_onchain_untouched() {
        let reg = registry();
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let new = apply(&old, &Mutation::Remove(Role::OffChain));
        let plan = plan_update(&old, &new, &reg).unwrap();

        assert_eq!(
            plan.slot_ops,
            vec![SlotOp::Remove(PathBuf::from("off-chain"))]
        );
        assert!(!dirs_touched(&plan).contains(&"on-chain".to_string()));
        assert!(!creates_under(&plan, "on-chain"));
    }

    #[test]
    fn swap_offchain_is_a_replace() {
        let reg = registry();
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let new = apply(&old, &Mutation::Add(a(Role::OffChain, "tx3")));
        let plan = plan_update(&old, &new, &reg).unwrap();

        assert_eq!(
            plan.slot_ops,
            vec![SlotOp::Replace(PathBuf::from("off-chain"))]
        );
        assert!(creates_under(&plan, "off-chain"));
        assert!(!dirs_touched(&plan).contains(&"on-chain".to_string()));
    }

    #[test]
    fn fusion_removes_two_dirs_and_creates_protocol() {
        let reg = registry();
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        // Switch both roles to a fullstack tool (scalus) → protocol/.
        let new = apply_all(
            &old,
            &[
                Mutation::Add(a(Role::OnChain, "scalus")),
                Mutation::Add(a(Role::OffChain, "scalus")),
            ],
        );
        let plan = plan_update(&old, &new, &reg).unwrap();

        let mut touched = dirs_touched(&plan);
        touched.sort();
        assert_eq!(touched, vec!["off-chain", "on-chain", "protocol"]);
        assert!(
            plan.slot_ops
                .contains(&SlotOp::Remove(PathBuf::from("on-chain")))
        );
        assert!(
            plan.slot_ops
                .contains(&SlotOp::Remove(PathBuf::from("off-chain")))
        );
        assert!(
            plan.slot_ops
                .contains(&SlotOp::Create(PathBuf::from("protocol")))
        );
        assert!(creates_under(&plan, "protocol"));
    }

    #[test]
    fn adding_infra_provider_rerenders_infra_in_place() {
        let reg = registry();
        let old = sel(vec![
            a(Role::OnChain, "aiken"),
            a(Role::Infrastructure, "kupo"),
        ]);
        let new = apply(&old, &Mutation::Add(a(Role::Infrastructure, "ogmios")));
        let plan = plan_update(&old, &new, &reg).unwrap();

        assert_eq!(
            plan.slot_ops,
            vec![SlotOp::RerenderInfra(PathBuf::from("infra"))]
        );
        assert!(creates_under(&plan, "infra"));
        assert!(!dirs_touched(&plan).contains(&"on-chain".to_string()));
    }

    #[test]
    fn removing_last_noninfra_role_drops_blueprint_marker() {
        let reg = registry();
        let old = sel(vec![
            a(Role::OnChain, "aiken"),
            a(Role::Infrastructure, "kupo"),
        ]);
        let new = apply(&old, &Mutation::Remove(Role::OnChain));
        let plan = plan_update(&old, &new, &reg).unwrap();

        assert_eq!(
            plan.slot_ops,
            vec![SlotOp::Remove(PathBuf::from("on-chain"))]
        );
        assert!(
            plan.shared_removals
                .contains(&PathBuf::from("blueprint/.gitkeep"))
        );
    }

    #[test]
    fn identical_selection_is_a_noop() {
        let reg = registry();
        let old = sel(vec![a(Role::OnChain, "aiken"), a(Role::OffChain, "meshjs")]);
        let plan = plan_update(&old, &old, &reg).unwrap();
        assert!(plan.slot_ops.is_empty());
        assert!(plan.shared_removals.is_empty());
    }

    #[test]
    fn add_infra_dedup_is_noop() {
        let old = sel(vec![a(Role::Infrastructure, "kupo")]);
        let new = apply(&old, &Mutation::Add(a(Role::Infrastructure, "kupo")));
        assert_eq!(new, old);
    }
}
