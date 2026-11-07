//! The dependency doctor (tech-spec §9).
//!
//! Checks which required dependencies are present and, for missing ones,
//! produces an OS-aware, ordered install plan. The resolver here, `installers`
//! and `catalog` are pure; only `probe` touches the system. Nothing here
//! depends on `cli`.

pub mod catalog;
pub mod installers;
pub mod probe;

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

use catalog::{DepCatalog, DepRecipe, InstallMethod, SupportLink};
use installers::Installer;
use probe::Environment;

use crate::registry::loader::Registry;
use crate::registry::types::Role;
use crate::registry::version::Version;

/// The task runner every project needs. Owned by no tool.
pub const BASE_DEP: &str = "just";

/// The oldest `just` the generated Justfiles run on: they use the terminal
/// escape constants (`{{BOLD + CYAN}}`), added in just 1.37.0.
pub const BASE_DEP_MIN_VERSION: &str = "1.37.0";

/// Component id the project scan reports for the aggregated `infra/`
/// component; its required deps are the union of all infra tools'
/// `system_deps`.
pub const INFRA_DRIVER_ID: &str = "cardano-up";

/// A dependency the project needs, with the lowest version that works when
/// any selected tool sets one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub id: String,
    pub min_version: Option<Version>,
}

/// The required dependencies for a set of tool ids (tech-spec §9.1): the base
/// dep `just` plus each tool's `system_deps`, deduplicated and sorted. When
/// several tools set a minimum version for the same dep, the highest wins.
///
/// The scan reports the aggregated infra component under [`INFRA_DRIVER_ID`];
/// it needs the union of every infra tool's deps.
pub fn requirements<'a>(
    tool_ids: impl IntoIterator<Item = &'a str>,
    registry: &Registry,
) -> Vec<Requirement> {
    let mut floors: BTreeMap<String, Option<Version>> = BTreeMap::new();
    let mut require = |id: &str, min: Option<&Version>| {
        let floor = floors.entry(id.to_string()).or_default();
        if let Some(min) = min.filter(|min| floor.as_ref().is_none_or(|f| *min > f)) {
            *floor = Some(min.clone());
        }
    };

    let base_min = Version::parse(BASE_DEP_MIN_VERSION);
    require(BASE_DEP, base_min.as_ref());
    for id in tool_ids {
        let tools = if id == INFRA_DRIVER_ID {
            registry.tools_for_role(Role::Infrastructure)
        } else {
            registry.get(id).into_iter().collect()
        };
        for tool in tools {
            for dep in &tool.system_deps {
                require(dep, tool.min_versions.get(dep));
            }
        }
    }

    floors
        .into_iter()
        .map(|(id, min_version)| Requirement { id, min_version })
        .collect()
}

/// One step of an install plan.
#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub installer: Installer,
    pub command: String,
}

impl Step {
    fn new(method: &InstallMethod) -> Self {
        Step {
            installer: method.installer,
            command: method.installer.command(&method.arg),
        }
    }
}

/// A recipe method other than the one the recommended plan uses (or, for an
/// outdated dep, an upgrade command). `available` is whether its installer is
/// present on this host.
#[derive(Debug, Clone, Serialize)]
pub struct AltMethod {
    pub installer: Installer,
    pub command: String,
    pub available: bool,
}

/// The resolved status of a single dependency.
#[derive(Debug, Clone, Serialize)]
pub struct DepStatus {
    pub id: String,
    pub required: bool,
    /// On `PATH`, at any version.
    pub present: bool,
    /// The installed version, when the dep has a minimum and it was readable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Version>,
    /// The lowest version the selected tools work with, if any sets one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_version: Option<Version>,
    /// Present, but older than `min_version`. A version that can't be read is
    /// never outdated: the doctor does not block on what it can't verify.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub outdated: bool,
    /// Ordered install steps for this host. Empty when present or unresolved.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plan: Vec<Step>,
    /// Other recipe methods beyond `plan`, available ones first. For an
    /// outdated dep: every method's upgrade command, available ones first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub alternatives: Vec<AltMethod>,
    /// Docs URL, the fallback so advice is never empty. `None` when the dep
    /// is satisfied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub support: Vec<SupportLink>,
}

/// The full doctor report for a set of required dependencies.
#[derive(Debug, Clone, Serialize)]
pub struct Report {
    /// Every required dep is on `PATH`, at any version.
    pub all_required_present: bool,
    /// Every required dep is on `PATH` and none is outdated.
    pub all_required_satisfied: bool,
    pub deps: Vec<DepStatus>,
}

impl DepStatus {
    /// Present and not outdated.
    pub fn satisfied(&self) -> bool {
        self.present && !self.outdated
    }
}

impl Report {
    /// The required deps that are missing or outdated, in report order.
    pub fn unsatisfied_required(&self) -> impl Iterator<Item = &DepStatus> {
        self.deps.iter().filter(|d| d.required && !d.satisfied())
    }
}

fn dep_present(recipe: &DepRecipe, env: &Environment) -> bool {
    recipe
        .binaries
        .iter()
        .any(|b| env.present_binaries.contains(b))
}

/// Resolve a dep to an ordered install plan for this host (empty when already
/// present), or `None` when no plan exists. Recurses over installer
/// `bootstrap` edges; `seen` guards against cycles.
fn resolve(
    dep_id: &str,
    env: &Environment,
    catalog: &DepCatalog,
    seen: &HashSet<String>,
) -> Option<Vec<Step>> {
    if seen.contains(dep_id) {
        return None;
    }
    let recipe = catalog.get(dep_id)?;
    if dep_present(recipe, env) {
        return Some(Vec::new());
    }

    // Prefer any method whose installer is usable right now, so e.g. `aiken`
    // resolves directly via `nix` without bootstrapping `aikup`.
    if let Some(method) = recipe
        .install
        .iter()
        .find(|m| env.installers.contains(&m.installer))
    {
        return Some(vec![Step::new(method)]);
    }

    // Otherwise bootstrap an installer, in recipe order.
    let mut seen = seen.clone();
    seen.insert(dep_id.to_string());
    recipe.install.iter().find_map(|method| {
        method.installer.bootstrap().iter().find_map(|bdep| {
            let mut steps = resolve(bdep, env, catalog, &seen)?;
            steps.push(Step::new(method));
            Some(steps)
        })
    })
}

/// Every recipe method whose installer differs from `chosen`, rendered by
/// `render` (install or upgrade), available ones first, each group in recipe
/// preference order.
fn alt_methods(
    recipe: &DepRecipe,
    chosen: Option<Installer>,
    env: &Environment,
    render: fn(Installer, &str) -> String,
) -> Vec<AltMethod> {
    let (available, needs_installer): (Vec<_>, Vec<_>) = recipe
        .install
        .iter()
        .filter(|method| Some(method.installer) != chosen)
        .map(|method| AltMethod {
            installer: method.installer,
            command: render(method.installer, &method.arg),
            available: env.installers.contains(&method.installer),
        })
        .partition(|alt| alt.available);
    available.into_iter().chain(needs_installer).collect()
}

fn dep_status(req: &Requirement, catalog: &DepCatalog, env: &Environment) -> DepStatus {
    let recipe = catalog.get(&req.id);
    let present = recipe.is_some_and(|r| dep_present(r, env));
    let version = req
        .min_version
        .as_ref()
        .and(env.versions.get(&req.id))
        .filter(|_| present)
        .cloned();
    let outdated = matches!((&version, &req.min_version), (Some(v), Some(min)) if v < min);
    let mut status = DepStatus {
        id: req.id.clone(),
        required: true,
        present,
        version,
        min_version: req.min_version.clone(),
        outdated,
        plan: Vec::new(),
        alternatives: Vec::new(),
        docs: None,
        support: recipe.map_or_else(Vec::new, |r| r.support.clone()),
    };
    let Some(recipe) = recipe else {
        return status;
    };
    if !present {
        status.plan = resolve(&status.id, env, catalog, &HashSet::new()).unwrap_or_default();
        let chosen = status.plan.last().map(|s| s.installer);
        status.alternatives = alt_methods(recipe, chosen, env, Installer::command);
        status.docs = Some(recipe.docs.clone());
    } else if outdated {
        // Which installer put it there is unknown, so offer every method's
        // upgrade, available ones first, rather than one recommended plan.
        status.alternatives = alt_methods(recipe, None, env, Installer::upgrade_command);
        status.docs = Some(recipe.docs.clone());
    }
    status
}

/// Resolve the required dependencies into a `Report`, in `required` order
/// (`requirements` returns them deduplicated and sorted).
pub fn resolve_all(required: &[Requirement], catalog: &DepCatalog, env: &Environment) -> Report {
    let deps: Vec<DepStatus> = required
        .iter()
        .map(|req| dep_status(req, catalog, env))
        .collect();
    Report {
        all_required_present: deps.iter().all(|d| d.present),
        all_required_satisfied: deps.iter().all(DepStatus::satisfied),
        deps,
    }
}

/// The Cardano tooling installed on this host regardless of what the project
/// requires: the base dep plus every tool's `system_deps` that are present,
/// sorted and deduplicated. Installer-only deps (aikup, rustup, go, ...) are
/// excluded; they surface as installers.
pub fn present_tooling(
    catalog: &DepCatalog,
    registry: &Registry,
    env: &Environment,
) -> Vec<String> {
    let mut ids: Vec<String> = std::iter::once(BASE_DEP.to_string())
        .chain(
            registry
                .all_tools()
                .iter()
                .flat_map(|t| t.system_deps.iter().cloned()),
        )
        .filter(|id| catalog.get(id).is_some_and(|r| dep_present(r, env)))
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doctor::probe::Os;

    fn env(installers: &[Installer], present_bins: &[&str]) -> Environment {
        Environment {
            os: Os::Linux,
            installers: installers.iter().copied().collect(),
            present_binaries: present_bins.iter().map(ToString::to_string).collect(),
            versions: std::collections::HashMap::new(),
        }
    }

    /// Requirements with no minimum version.
    fn reqs(ids: &[&str]) -> Vec<Requirement> {
        ids.iter()
            .map(|id| Requirement {
                id: (*id).to_string(),
                min_version: None,
            })
            .collect()
    }

    fn floor(id: &str, min: &str) -> Requirement {
        Requirement {
            id: id.to_string(),
            min_version: Version::parse(min),
        }
    }

    fn with_version(mut env: Environment, id: &str, version: &str) -> Environment {
        env.versions
            .insert(id.to_string(), Version::parse(version).unwrap());
        env
    }

    fn catalog() -> DepCatalog {
        DepCatalog::load().expect("embedded catalog loads")
    }

    fn status<'a>(report: &'a Report, id: &str) -> &'a DepStatus {
        report
            .deps
            .iter()
            .find(|d| d.id == id)
            .expect("dep present")
    }

    #[test]
    fn present_dep_yields_empty_plan() {
        let report = resolve_all(&reqs(&["just"]), &catalog(), &env(&[], &["just"]));
        let just = status(&report, "just");
        assert!(just.present);
        assert!(just.plan.is_empty());
        assert!(just.docs.is_none());
        assert!(report.all_required_present);
    }

    #[test]
    fn single_step_when_installer_detected() {
        // node missing, brew available → one-step `brew install node`.
        let report = resolve_all(&reqs(&["node"]), &catalog(), &env(&[Installer::Brew], &[]));
        let node = status(&report, "node");
        assert!(!node.present);
        assert_eq!(node.plan.len(), 1);
        assert_eq!(node.plan[0].installer, Installer::Brew);
        assert_eq!(node.plan[0].command, "brew install node");
        assert!(node.docs.is_some());
        assert!(!report.all_required_present);
    }

    #[test]
    fn multi_step_bootstrap_aiken_via_npm() {
        // aiken missing, no nix/aikup, but npm present → bootstrap aikup via npm,
        // then `aikup install` (bare ⇒ latest).
        let report = resolve_all(&reqs(&["aiken"]), &catalog(), &env(&[Installer::Npm], &[]));
        let aiken = status(&report, "aiken");
        assert!(!aiken.present);
        assert_eq!(aiken.plan.len(), 2);
        assert_eq!(aiken.plan[0].installer, Installer::Npm);
        assert_eq!(aiken.plan[0].command, "npm install -g @aiken-lang/aikup");
        assert_eq!(aiken.plan[1].installer, Installer::Aikup);
        assert_eq!(aiken.plan[1].command, "aikup install");
    }

    #[test]
    fn nix_short_circuits_aiken_to_one_step() {
        // With nix present, aiken resolves directly (no aikup needed).
        let report = resolve_all(&reqs(&["aiken"]), &catalog(), &env(&[Installer::Nix], &[]));
        let aiken = status(&report, "aiken");
        assert_eq!(aiken.plan.len(), 1);
        assert_eq!(aiken.plan[0].installer, Installer::Nix);
        assert_eq!(aiken.plan[0].command, "nix profile install nixpkgs#aiken");
    }

    #[test]
    fn unresolved_falls_back_to_docs_only() {
        // aiken missing and no usable installer at all → no plan, docs present.
        let report = resolve_all(&reqs(&["aiken"]), &catalog(), &env(&[], &[]));
        let aiken = status(&report, "aiken");
        assert!(!aiken.present);
        assert!(aiken.plan.is_empty());
        assert_eq!(
            aiken.docs.as_deref(),
            Some("https://aiken-lang.org/installation-instructions")
        );
    }

    #[test]
    fn alternatives_offer_other_methods_available_first() {
        // sbt recipe = [brew, nix]. With only nix present: plan uses nix, and
        // brew is offered as an alternative flagged unavailable.
        let report = resolve_all(&reqs(&["sbt"]), &catalog(), &env(&[Installer::Nix], &[]));
        let sbt = status(&report, "sbt");
        assert_eq!(sbt.plan[0].installer, Installer::Nix);
        assert_eq!(sbt.alternatives.len(), 1);
        assert_eq!(sbt.alternatives[0].installer, Installer::Brew);
        assert!(!sbt.alternatives[0].available);
        assert_eq!(sbt.alternatives[0].command, "brew install sbt");
    }

    #[test]
    fn alternatives_sort_available_before_needs_installer() {
        // just recipe = [brew, apt, cargo, nix]. With cargo + nix present: plan
        // uses cargo (first available); the other available method (nix) is
        // offered before the ones that still need their installer (brew, apt).
        let report = resolve_all(
            &reqs(&["just"]),
            &catalog(),
            &env(&[Installer::Cargo, Installer::Nix], &[]),
        );
        let just = status(&report, "just");
        assert_eq!(just.plan[0].installer, Installer::Cargo);
        // The chosen installer is never repeated as an alternative.
        assert!(
            just.alternatives
                .iter()
                .all(|a| a.installer != Installer::Cargo)
        );
        // First alternative is the other available installer.
        assert_eq!(just.alternatives[0].installer, Installer::Nix);
        assert!(just.alternatives[0].available);
        // Unavailable ones come after and are flagged.
        assert!(
            just.alternatives
                .iter()
                .any(|a| a.installer == Installer::Brew && !a.available)
        );
    }

    #[test]
    fn requirements_dedup_sort_and_include_base_floor() {
        let registry = Registry::load().unwrap();
        // aiken → ["aiken"], meshjs → ["node"]; plus the base dep "just".
        let required = requirements(["meshjs", "aiken", "meshjs"], &registry);
        let ids: Vec<&str> = required.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["aiken", "just", "node"]);
        let just = required.iter().find(|r| r.id == BASE_DEP).unwrap();
        assert_eq!(just.min_version, Version::parse(BASE_DEP_MIN_VERSION));
    }

    #[test]
    fn requirements_for_yaci_include_its_cli_floor() {
        let registry = Registry::load().unwrap();
        let required = requirements(["yaci"], &registry);
        let yaci = required.iter().find(|r| r.id == "yaci-devkit").unwrap();
        assert_eq!(yaci.min_version, Version::parse("0.12.0"));
        assert!(required.iter().any(|r| r.id == "node"));
    }

    #[test]
    fn requirements_take_the_highest_floor_across_tools() {
        // Plinth needs GHC 9.6, Plutarch 9.8: together, 9.8 wins.
        let registry = Registry::load().unwrap();
        let ghc = |tools: &[&str]| {
            requirements(tools.iter().copied(), &registry)
                .into_iter()
                .find(|r| r.id == "ghc")
                .unwrap()
                .min_version
                .unwrap()
                .to_string()
        };
        assert_eq!(ghc(&["plinth"]), "9.6");
        assert_eq!(ghc(&["plinth", "plutarch"]), "9.8");
        assert_eq!(ghc(&["plutarch", "plinth"]), "9.8");
    }

    #[test]
    fn requirements_resolve_infra_driver_to_union() {
        let registry = Registry::load().unwrap();
        let required = requirements([INFRA_DRIVER_ID], &registry);
        let ids: Vec<&str> = required.iter().map(|r| r.id.as_str()).collect();
        assert!(ids.contains(&"just"));
        assert!(ids.contains(&"docker"));
        assert!(ids.contains(&"cardano-up"));
    }

    #[test]
    fn old_version_is_outdated_with_upgrade_advice() {
        let env = with_version(
            env(&[Installer::Npm], &["yaci-devkit"]),
            "yaci-devkit",
            "0.10.6",
        );
        let report = resolve_all(&[floor("yaci-devkit", "0.12.0")], &catalog(), &env);
        let yaci = status(&report, "yaci-devkit");
        assert!(yaci.present);
        assert!(yaci.outdated);
        assert!(!yaci.satisfied());
        assert!(report.all_required_present);
        assert!(!report.all_required_satisfied);
        assert!(yaci.plan.is_empty());
        assert_eq!(
            yaci.alternatives[0].command,
            "npm install -g @bloxbean/yaci-devkit@latest"
        );
        assert!(yaci.alternatives[0].available);
        assert!(yaci.docs.is_some());
        assert_eq!(report.unsatisfied_required().count(), 1);
    }

    #[test]
    fn new_enough_version_is_satisfied() {
        let env = with_version(env(&[], &["ghc"]), "ghc", "9.8.4");
        let report = resolve_all(&[floor("ghc", "9.8")], &catalog(), &env);
        let ghc = status(&report, "ghc");
        assert!(ghc.satisfied());
        assert_eq!(ghc.version.as_ref().unwrap().to_string(), "9.8.4");
        assert!(ghc.alternatives.is_empty());
        assert!(ghc.docs.is_none());
        assert!(report.all_required_satisfied);
    }

    #[test]
    fn unreadable_version_is_not_outdated() {
        // `cardano-up` built with `go install` reports `devel`: never block on it.
        let report = resolve_all(
            &[floor("cardano-up", "0.15.0")],
            &catalog(),
            &env(&[], &["cardano-up"]),
        );
        let up = status(&report, "cardano-up");
        assert!(up.satisfied());
        assert!(up.version.is_none());
        assert!(report.all_required_satisfied);
    }

    #[test]
    fn outdated_json_shape() {
        let env = with_version(
            env(&[Installer::Npm], &["yaci-devkit"]),
            "yaci-devkit",
            "0.10.6",
        );
        let report = resolve_all(&[floor("yaci-devkit", "0.12.0")], &catalog(), &env);
        let value = serde_json::to_value(&report).unwrap();
        let dep = &value["deps"][0];
        assert_eq!(dep["version"], "0.10.6");
        assert_eq!(dep["min_version"], "0.12.0");
        assert_eq!(dep["outdated"], true);
        assert_eq!(value["all_required_present"], true);
        assert_eq!(value["all_required_satisfied"], false);
    }

    #[test]
    fn whole_catalog_resolves_without_infinite_recursion() {
        // Cycle guard: resolving every dep id terminates (worst case: empty env).
        let cat = catalog();
        let env = env(&[], &[]);
        for id in cat.dep_ids() {
            let _ = resolve(id, &env, &cat, &HashSet::new()); // must return, not hang
        }
    }

    #[test]
    fn report_json_shape_matches_spec() {
        // Present dep → only {id, required, present}; missing and resolved →
        // also {plan, docs} (tech-spec §9.4).
        let report = resolve_all(
            &reqs(&["just", "node"]),
            &catalog(),
            &env(&[Installer::Brew], &["just"]),
        );
        let value = serde_json::to_value(&report).unwrap();
        let deps = value["deps"].as_array().unwrap();

        let just = deps.iter().find(|d| d["id"] == "just").unwrap();
        assert_eq!(just["present"], true);
        assert!(just.get("plan").is_none(), "present dep must omit plan");
        assert!(
            just.get("outdated").is_none(),
            "a false `outdated` is omitted"
        );
        assert!(just.get("docs").is_none(), "present dep must omit docs");

        let node = deps.iter().find(|d| d["id"] == "node").unwrap();
        assert_eq!(node["present"], false);
        assert_eq!(node["plan"][0]["installer"], "brew");
        assert_eq!(node["plan"][0]["command"], "brew install node");
        assert!(node["docs"].is_string());
    }

    #[test]
    fn present_tooling_lists_installed_tools_regardless_of_requirement() {
        let registry = crate::registry::loader::Registry::load().unwrap();
        let tooling = present_tooling(&catalog(), &registry, &env(&[], &["aiken", "just"]));
        assert_eq!(tooling, vec!["aiken", "just"]);
    }

    #[test]
    fn present_tooling_excludes_installer_only_deps() {
        let registry = crate::registry::loader::Registry::load().unwrap();
        let tooling = present_tooling(&catalog(), &registry, &env(&[], &["go", "aikup"]));
        assert!(tooling.is_empty());
    }

    // Referential integrity (tech-spec §9.5).

    #[test]
    fn every_system_dep_plus_base_has_a_recipe() {
        let registry = crate::registry::loader::Registry::load().unwrap();
        let cat = catalog();
        assert!(
            cat.get(BASE_DEP).is_some(),
            "base dep '{BASE_DEP}' must have a recipe"
        );
        for tool in registry.all_tools() {
            for dep in &tool.system_deps {
                assert!(
                    cat.get(dep).is_some(),
                    "tool '{}' requires dep '{}' with no registry/deps.toml entry",
                    tool.id,
                    dep
                );
            }
        }
    }

    #[test]
    fn every_bootstrap_dep_has_a_recipe() {
        let cat = catalog();
        for installer in Installer::ALL {
            for bdep in installer.bootstrap() {
                assert!(
                    cat.get(bdep).is_some(),
                    "installer '{installer}' bootstraps via dep '{bdep}' with no recipe"
                );
            }
        }
    }
}
