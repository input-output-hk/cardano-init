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
use crate::registry::types::{Role, SystemDep};
use crate::registry::version::{Version, VersionReq};

/// The task runner every project needs. Owned by no tool.
pub const BASE_DEP: &str = "just";

/// The `just` major the generated Justfiles run on (tech-spec §9.7). Pins are
/// prefix-exact, so the 1.37.0 the Justfiles' terminal escape constants
/// (`{{BOLD + CYAN}}`) need is not checked; package managers ship a newer 1.x.
pub const BASE_DEP_VERSION: &str = "1";

/// Component id the project scan reports for the aggregated `infra/`
/// component; its required deps are the union of all infra tools'
/// `system_deps`.
pub const INFRA_DRIVER_ID: &str = "cardano-up";

/// One tool's version pin on a dep.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pin {
    pub tool: String,
    pub version: VersionReq,
}

/// A dependency the project needs, with the version the selected tools pin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub id: String,
    /// The tightest pin every selected tool agrees on, if any sets one.
    pub version: Option<VersionReq>,
    /// Every pin, when no single version satisfies them all (then `version`
    /// is `None`): tools parked on different eras can disagree.
    pub conflicts: Vec<Pin>,
}

/// The required dependencies for a set of tool ids (tech-spec §9.1): the base
/// dep `just` plus each tool's `system_deps`, deduplicated and sorted. Pins on
/// the same dep merge into the most specific one (`1.1` + `1.1.24` →
/// `1.1.24`); pins that can't merge are reported as conflicts.
///
/// The scan reports the aggregated infra component under [`INFRA_DRIVER_ID`];
/// it needs the union of every infra tool's deps.
pub fn requirements<'a>(
    tool_ids: impl IntoIterator<Item = &'a str>,
    registry: &Registry,
) -> Vec<Requirement> {
    let mut pins: BTreeMap<String, Vec<Pin>> = BTreeMap::new();
    let mut require = |id: &str, tool: &str, version: Option<&VersionReq>| {
        let dep = pins.entry(id.to_string()).or_default();
        if let Some(version) = version {
            let pin = Pin {
                tool: tool.to_string(),
                version: version.clone(),
            };
            if !dep.contains(&pin) {
                dep.push(pin);
            }
        }
    };

    require(
        BASE_DEP,
        BASE_DEP,
        VersionReq::parse(BASE_DEP_VERSION).as_ref(),
    );
    for id in tool_ids {
        for (tool, dep) in component_deps(id, registry) {
            require(&dep.id, tool, dep.version.as_ref());
        }
    }

    pins.into_iter()
        .map(|(id, pins)| merge_pins(id, pins))
        .collect()
}

pub fn component_deps<'a>(
    component_id: &str,
    registry: &'a Registry,
) -> Vec<(&'a str, &'a SystemDep)> {
    let tools = if component_id == INFRA_DRIVER_ID {
        registry.tools_for_role(Role::Infrastructure)
    } else {
        registry.get(component_id).into_iter().collect()
    };
    tools
        .into_iter()
        .flat_map(|tool| {
            tool.system_deps
                .iter()
                .map(move |dep| (tool.id.as_str(), dep))
        })
        .collect()
}

fn merge_pins(id: String, pins: Vec<Pin>) -> Requirement {
    let merged = pins.iter().try_fold(None::<VersionReq>, |acc, pin| {
        Some(Some(match acc {
            None => pin.version.clone(),
            Some(acc) => acc.narrow(&pin.version)?,
        }))
    });
    match merged {
        Some(version) => Requirement {
            id,
            version,
            conflicts: Vec::new(),
        },
        None => Requirement {
            id,
            version: None,
            conflicts: pins,
        },
    }
}

/// One step of an install plan.
#[derive(Debug, Clone, Serialize)]
pub struct Step {
    pub installer: Installer,
    pub command: String,
    /// The dep is pinned but this command installs the installer's latest,
    /// which may not match the pin.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unpinned: bool,
}

impl Step {
    fn new(method: &InstallMethod, version: Option<&VersionReq>) -> Self {
        Step {
            installer: method.installer.clone(),
            command: method.render(version),
            unpinned: ignores_pin(method, version),
        }
    }
}

fn ignores_pin(method: &InstallMethod, version: Option<&VersionReq>) -> bool {
    version.is_some() && !method.uses_version
}

/// A recipe method other than the one the recommended plan uses (or, for a
/// mismatched dep, an upgrade command). `available` is whether its installer
/// is present on this host.
#[derive(Debug, Clone, Serialize)]
pub struct AltMethod {
    pub installer: Installer,
    pub command: String,
    pub available: bool,
    /// See [`Step::unpinned`].
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub unpinned: bool,
}

/// The resolved status of a single dependency.
#[derive(Debug, Clone, Serialize)]
pub struct DepStatus {
    pub id: String,
    pub required: bool,
    /// On `PATH`, at any version.
    pub present: bool,
    /// The installed version, when the dep is pinned and it was readable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<Version>,
    /// The version the selected tools pin (prefix-exact: `1.1` is `1.1.*`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub required_version: Option<VersionReq>,
    /// Present, but not matching `required_version`. A version that can't be
    /// read never mismatches: the doctor does not block on what it can't
    /// verify.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub version_mismatch: bool,
    /// The selected tools pin versions no single install satisfies.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub version_conflict: Vec<Pin>,
    /// Ordered install (or, for a mismatched dep, upgrade) steps for this
    /// host. Empty when satisfied or unresolved.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub plan: Vec<Step>,
    /// Other recipe methods beyond `plan`, available ones first (upgrade
    /// commands for a mismatched dep, minus those that install latest when
    /// the installed version is already past the pin).
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
    /// Every required dep is on `PATH` and matches its pinned version.
    pub all_required_satisfied: bool,
    pub deps: Vec<DepStatus>,
}

impl DepStatus {
    /// Present and matching its pinned version.
    pub fn satisfied(&self) -> bool {
        self.present && !self.version_mismatch
    }
}

impl Report {
    /// The required deps that are missing or mismatched, in report order.
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
///
/// For a pinned dep, pin-honoring methods are tried first (bootstrapping if
/// needed) before falling back to methods that install latest; within each
/// group, an already-available installer beats bootstrapping one.
fn resolve(
    dep_id: &str,
    version: Option<&VersionReq>,
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

    let mut seen = seen.clone();
    seen.insert(dep_id.to_string());
    plan_methods(recipe.install.iter(), version, true, env, catalog, &seen)
}

/// Pin-honoring `methods` first, then (when `allow_unpinned`) the ones that
/// install latest.
fn plan_methods<'a>(
    methods: impl Iterator<Item = &'a InstallMethod>,
    version: Option<&VersionReq>,
    allow_unpinned: bool,
    env: &Environment,
    catalog: &DepCatalog,
    seen: &HashSet<String>,
) -> Option<Vec<Step>> {
    let (honoring, fallback): (Vec<&InstallMethod>, Vec<&InstallMethod>) =
        methods.partition(|m| !ignores_pin(m, version));
    if let Some(steps) = plan_with(&honoring, version, env, catalog, seen) {
        return Some(steps);
    }
    if !allow_unpinned || fallback.is_empty() {
        return None;
    }
    plan_with(&fallback, version, env, catalog, seen)
}

/// The first of `methods` usable right now, else the first whose installer a
/// bootstrap dep can provide, in recipe order.
fn plan_with(
    methods: &[&InstallMethod],
    version: Option<&VersionReq>,
    env: &Environment,
    catalog: &DepCatalog,
    seen: &HashSet<String>,
) -> Option<Vec<Step>> {
    if let Some(method) = methods
        .iter()
        .find(|m| env.installers.contains(&m.installer))
    {
        return Some(vec![Step::new(method, version)]);
    }
    methods.iter().find_map(|method| {
        catalog
            .bootstrap(&method.installer)
            .iter()
            .find_map(|bdep| {
                let mut steps = resolve(bdep, None, env, catalog, seen)?;
                steps.push(Step::new(method, version));
                Some(steps)
            })
    })
}

/// Every recipe method whose installer differs from `chosen`, as the method
/// `effective` maps it to (install or upgrade): available ones first, then
/// within each group the ones that install the pin, in recipe order.
fn alt_methods<'a>(
    recipe: &'a DepRecipe,
    chosen: Option<&Installer>,
    env: &Environment,
    version: Option<&VersionReq>,
    effective: impl Fn(&'a InstallMethod) -> &'a InstallMethod,
) -> Vec<AltMethod> {
    let mut alts: Vec<AltMethod> = recipe
        .install
        .iter()
        .filter(|method| Some(&method.installer) != chosen)
        .map(|method| {
            let m = effective(method);
            AltMethod {
                installer: method.installer.clone(),
                command: m.render(version),
                available: env.installers.contains(&method.installer),
                unpinned: ignores_pin(m, version),
            }
        })
        .collect();
    alts.sort_by_key(|alt| (!alt.available, alt.unpinned));
    alts
}

fn dep_status(req: &Requirement, catalog: &DepCatalog, env: &Environment) -> DepStatus {
    let recipe = catalog.get(&req.id);
    let present = recipe.is_some_and(|r| dep_present(r, env));
    let pinned = req.version.is_some() || !req.conflicts.is_empty();
    let version = env
        .versions
        .get(&req.id)
        .filter(|_| present && pinned)
        .cloned();
    let version_mismatch =
        matches!((&version, &req.version), (Some(v), Some(want)) if !want.matches(v));
    let want = req.version.as_ref();
    let mut status = DepStatus {
        id: req.id.clone(),
        required: true,
        present,
        version,
        required_version: req.version.clone(),
        version_mismatch,
        version_conflict: req.conflicts.clone(),
        plan: Vec::new(),
        alternatives: Vec::new(),
        docs: None,
        support: recipe.map_or_else(Vec::new, |r| r.support.clone()),
    };
    let Some(recipe) = recipe else {
        return status;
    };
    if !present {
        status.plan = resolve(&status.id, want, env, catalog, &HashSet::new()).unwrap_or_default();
        let chosen = status.plan.last().map(|s| &s.installer);
        status.alternatives = alt_methods(recipe, chosen, env, want, |m| m);
        status.docs = Some(recipe.docs.clone());
    } else if version_mismatch {
        // Past the pin, a command that installs latest can't get back to it.
        let too_new = matches!((&status.version, want), (Some(v), Some(w)) if w.is_below(v));
        let seen = HashSet::from([status.id.clone()]);
        status.plan = plan_methods(
            recipe.install.iter().map(|m| recipe.upgrade_method(m)),
            want,
            !too_new,
            env,
            catalog,
            &seen,
        )
        .unwrap_or_default();
        let chosen = status.plan.last().map(|s| &s.installer);
        status.alternatives = alt_methods(recipe, chosen, env, want, |m| recipe.upgrade_method(m));
        status.alternatives.retain(|alt| !(too_new && alt.unpinned));
        if let Some(after) = &recipe.after_upgrade {
            let commands = status
                .plan
                .last_mut()
                .map(|s| &mut s.command)
                .into_iter()
                .chain(status.alternatives.iter_mut().map(|a| &mut a.command));
            for command in commands {
                *command = format!("{command} && {after}");
            }
        }
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
/// sorted and deduplicated. Installer-only deps (aikup, rustup, ...) are
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
                .flat_map(|t| t.system_deps.iter().map(|d| d.id.clone())),
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

    fn env(installers: &[&str], present_bins: &[&str]) -> Environment {
        Environment {
            os: Os::Linux,
            installers: installers.iter().map(|i| Installer::new(*i)).collect(),
            present_binaries: present_bins.iter().map(ToString::to_string).collect(),
            versions: std::collections::HashMap::new(),
        }
    }

    /// Requirements with no pinned version.
    fn reqs(ids: &[&str]) -> Vec<Requirement> {
        ids.iter()
            .map(|id| Requirement {
                id: (*id).to_string(),
                version: None,
                conflicts: Vec::new(),
            })
            .collect()
    }

    fn pinned(id: &str, version: &str) -> Requirement {
        Requirement {
            id: id.to_string(),
            version: VersionReq::parse(version),
            conflicts: Vec::new(),
        }
    }

    fn pin(tool: &str, version: &str) -> Pin {
        Pin {
            tool: tool.to_string(),
            version: VersionReq::parse(version).unwrap(),
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
        let report = resolve_all(&reqs(&["node"]), &catalog(), &env(&["brew"], &[]));
        let node = status(&report, "node");
        assert!(!node.present);
        assert_eq!(node.plan.len(), 1);
        assert_eq!(node.plan[0].installer, "brew");
        assert_eq!(node.plan[0].command, "brew install node");
        assert!(node.docs.is_some());
        assert!(!report.all_required_present);
    }

    #[test]
    fn multi_step_bootstrap_aiken_via_npm() {
        // aiken missing, no nix/aikup, but npm present → bootstrap aikup via npm,
        // then `aikup install` (bare ⇒ latest).
        let report = resolve_all(&reqs(&["aiken"]), &catalog(), &env(&["npm"], &[]));
        let aiken = status(&report, "aiken");
        assert!(!aiken.present);
        assert_eq!(aiken.plan.len(), 2);
        assert_eq!(aiken.plan[0].installer, "npm");
        assert_eq!(aiken.plan[0].command, "npm install -g @aiken-lang/aikup");
        assert_eq!(aiken.plan[1].installer, "aikup");
        assert_eq!(aiken.plan[1].command, "aikup install");
    }

    #[test]
    fn pinned_aiken_installs_the_pinned_version() {
        let report = resolve_all(
            &[pinned("aiken", "1.1.24")],
            &catalog(),
            &env(&["aikup"], &[]),
        );
        let aiken = status(&report, "aiken");
        assert_eq!(aiken.plan.len(), 1);
        assert_eq!(aiken.plan[0].command, "aikup install v1.1.24");
    }

    #[test]
    fn pinned_dep_flags_a_method_that_ignores_the_pin() {
        let report = resolve_all(
            &[pinned("aiken", "1.1.24")],
            &catalog(),
            &env(&["aikup", "nix"], &[]),
        );
        let aiken = status(&report, "aiken");
        assert_eq!(aiken.plan[0].command, "aikup install v1.1.24");
        assert!(!aiken.plan[0].unpinned);
        let nix = aiken
            .alternatives
            .iter()
            .find(|a| a.installer == "nix")
            .unwrap();
        assert_eq!(nix.command, "nix profile install nixpkgs#aiken");
        assert!(nix.unpinned);
    }

    #[test]
    fn nix_alone_still_reaches_the_pin_through_node_and_aikup() {
        let report = resolve_all(
            &[pinned("aiken", "1.1.24")],
            &catalog(),
            &env(&["nix"], &[]),
        );
        let commands: Vec<&str> = status(&report, "aiken")
            .plan
            .iter()
            .map(|s| s.command.as_str())
            .collect();
        assert_eq!(
            commands,
            [
                "nix profile install nixpkgs#nodejs",
                "npm install -g @aiken-lang/aikup",
                "aikup install v1.1.24"
            ]
        );
    }

    #[test]
    fn pin_falls_back_to_a_method_that_ignores_it_when_nothing_else_resolves() {
        let mut cat = catalog();
        cat.retain_install_for_test("aiken", "nix");
        let report = resolve_all(&[pinned("aiken", "1.1.24")], &cat, &env(&["nix"], &[]));
        let aiken = status(&report, "aiken");
        assert_eq!(aiken.plan[0].command, "nix profile install nixpkgs#aiken");
        assert!(aiken.plan[0].unpinned);
    }

    #[test]
    fn pinned_dep_bootstraps_rather_than_ignore_the_pin() {
        let report = resolve_all(
            &[pinned("aiken", "1.1.24")],
            &catalog(),
            &env(&["nix", "curl"], &[]),
        );
        let commands: Vec<&str> = status(&report, "aiken")
            .plan
            .iter()
            .map(|s| s.command.as_str())
            .collect();
        assert_eq!(
            commands,
            [
                "curl -sSfL https://install.aiken-lang.org | sh",
                "aikup install v1.1.24"
            ]
        );
    }

    #[test]
    fn unpinned_dep_never_flags_methods() {
        let report = resolve_all(&reqs(&["aiken"]), &catalog(), &env(&["nix"], &[]));
        let aiken = status(&report, "aiken");
        assert!(!aiken.plan[0].unpinned);
        let value = serde_json::to_value(aiken).unwrap();
        assert!(value["plan"][0].get("unpinned").is_none());
    }

    #[test]
    fn mismatch_lists_pin_honoring_upgrades_first() {
        let env = with_version(env(&["nix", "aikup"], &["aiken"]), "aiken", "1.1.21");
        let report = resolve_all(&[pinned("aiken", "1.1.24")], &catalog(), &env);
        let aiken = status(&report, "aiken");
        assert_eq!(aiken.plan[0].command, "aikup install v1.1.24");
        assert!(!aiken.plan[0].unpinned);
        assert_eq!(aiken.alternatives[0].command, "nix profile upgrade aiken");
        assert!(aiken.alternatives[0].unpinned);
    }

    #[test]
    fn mismatch_bootstraps_a_missing_installer() {
        let env = with_version(env(&["npm"], &["aiken"]), "aiken", "1.1.21");
        let report = resolve_all(&[pinned("aiken", "1.1.24")], &catalog(), &env);
        let commands: Vec<&str> = status(&report, "aiken")
            .plan
            .iter()
            .map(|s| s.command.as_str())
            .collect();
        assert_eq!(
            commands,
            ["npm install -g @aiken-lang/aikup", "aikup install v1.1.24"]
        );
    }

    #[test]
    fn past_the_pin_drops_commands_that_install_latest() {
        let env = with_version(env(&["nix"], &["aiken"]), "aiken", "1.2.0");
        let report = resolve_all(&[pinned("aiken", "1.1.24")], &catalog(), &env);
        let aiken = status(&report, "aiken");
        // nix → node → aikup, rather than `nix profile upgrade aiken`.
        let last = aiken.plan.last().unwrap();
        assert_eq!(last.command, "aikup install v1.1.24");
        assert!(!last.unpinned);
        assert!(aiken.alternatives.iter().all(|a| !a.unpinned));
    }

    #[test]
    fn past_a_partial_pin_leaves_only_docs() {
        let env = with_version(env(&["brew"], &["just"]), "just", "2.0.1");
        let report = resolve_all(&[pinned("just", "1")], &catalog(), &env);
        let just = status(&report, "just");
        assert!(just.version_mismatch);
        assert!(just.plan.is_empty());
        assert!(just.alternatives.is_empty());
        assert!(just.docs.is_some());
    }

    #[test]
    fn below_the_pin_still_offers_install_latest() {
        let env = with_version(env(&["brew"], &["just"]), "just", "0.9.0");
        let report = resolve_all(&[pinned("just", "1")], &catalog(), &env);
        let just = status(&report, "just");
        assert_eq!(just.plan[0].command, "brew upgrade just");
        assert!(just.plan[0].unpinned);
    }

    #[test]
    fn nix_short_circuits_aiken_to_one_step() {
        // With nix present, aiken resolves directly (no aikup needed).
        let report = resolve_all(&reqs(&["aiken"]), &catalog(), &env(&["nix"], &[]));
        let aiken = status(&report, "aiken");
        assert_eq!(aiken.plan.len(), 1);
        assert_eq!(aiken.plan[0].installer, "nix");
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
        let report = resolve_all(&reqs(&["sbt"]), &catalog(), &env(&["nix"], &[]));
        let sbt = status(&report, "sbt");
        assert_eq!(sbt.plan[0].installer, "nix");
        assert_eq!(sbt.alternatives.len(), 1);
        assert_eq!(sbt.alternatives[0].installer, "brew");
        assert!(!sbt.alternatives[0].available);
        assert_eq!(sbt.alternatives[0].command, "brew install sbt");
    }

    #[test]
    fn alternatives_sort_available_before_needs_installer() {
        // just recipe = [brew, apt, cargo, nix]. With cargo + nix present: plan
        // uses cargo (first available); the other available method (nix) is
        // offered before the ones that still need their installer (brew, apt).
        let report = resolve_all(&reqs(&["just"]), &catalog(), &env(&["cargo", "nix"], &[]));
        let just = status(&report, "just");
        assert_eq!(just.plan[0].installer, "cargo");
        // The chosen installer is never repeated as an alternative.
        assert!(just.alternatives.iter().all(|a| a.installer != "cargo"));
        // First alternative is the other available installer.
        assert_eq!(just.alternatives[0].installer, "nix");
        assert!(just.alternatives[0].available);
        // Unavailable ones come after and are flagged.
        assert!(
            just.alternatives
                .iter()
                .any(|a| a.installer == "brew" && !a.available)
        );
    }

    #[test]
    fn requirements_dedup_sort_and_include_base_pin() {
        let registry = Registry::load().unwrap();
        // aiken → ["aiken"], meshjs → ["node"]; plus the base dep "just".
        let required = requirements(["meshjs", "aiken", "meshjs"], &registry);
        let ids: Vec<&str> = required.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["aiken", "just", "node"]);
        let just = required.iter().find(|r| r.id == BASE_DEP).unwrap();
        assert_eq!(just.version, VersionReq::parse(BASE_DEP_VERSION));
        let aiken = required.iter().find(|r| r.id == "aiken").unwrap();
        assert_eq!(aiken.version, VersionReq::parse("1.1.24"));
    }

    #[test]
    fn requirements_for_yaci_include_its_cli_pin() {
        let registry = Registry::load().unwrap();
        let required = requirements(["yaci"], &registry);
        let yaci = required.iter().find(|r| r.id == "yaci-devkit").unwrap();
        assert_eq!(yaci.version, VersionReq::parse("0.12"));
        assert!(required.iter().any(|r| r.id == "node"));
    }

    #[test]
    fn pins_merge_into_the_most_specific() {
        let req = merge_pins("aiken".into(), vec![pin("a", "1.1"), pin("b", "1.1.24")]);
        assert_eq!(req.version, VersionReq::parse("1.1.24"));
        assert!(req.conflicts.is_empty());
    }

    #[test]
    fn disagreeing_pins_are_a_conflict() {
        let req = merge_pins("aiken".into(), vec![pin("a", "1.1.24"), pin("b", "1.2")]);
        assert!(req.version.is_none());
        assert_eq!(req.conflicts.len(), 2);
    }

    #[test]
    fn runtimes_are_unpinned() {
        let registry = Registry::load().unwrap();
        let required = requirements(["plinth", "plutarch", "meshjs", "tx3"], &registry);
        for id in ["ghc", "node"] {
            let req = required.iter().find(|r| r.id == id).unwrap();
            assert!(req.version.is_none(), "{id}");
            assert!(req.conflicts.is_empty(), "{id}");
        }
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
    fn other_version_mismatches_with_upgrade_advice() {
        let env = with_version(env(&["npm"], &["yaci-devkit"]), "yaci-devkit", "0.10.6");
        let report = resolve_all(&[pinned("yaci-devkit", "0.12")], &catalog(), &env);
        let yaci = status(&report, "yaci-devkit");
        assert!(yaci.present);
        assert!(yaci.version_mismatch);
        assert!(!yaci.satisfied());
        assert!(report.all_required_present);
        assert!(!report.all_required_satisfied);
        // The upgrade also refreshes the components the old CLI downloaded.
        assert_eq!(
            yaci.plan[0].command,
            "npm install -g @bloxbean/yaci-devkit@0.12 && yaci-devkit download --overwrite"
        );
        assert!(yaci.alternatives.is_empty());
        assert!(yaci.docs.is_some());
        assert_eq!(report.unsatisfied_required().count(), 1);
    }

    #[test]
    fn newer_version_than_the_pin_also_mismatches() {
        let env = with_version(env(&[], &["aiken"]), "aiken", "1.2.0");
        let report = resolve_all(&[pinned("aiken", "1.1.24")], &catalog(), &env);
        assert!(status(&report, "aiken").version_mismatch);
    }

    #[test]
    fn matching_version_is_satisfied() {
        let env = with_version(env(&[], &["ghc"]), "ghc", "9.8.4");
        let report = resolve_all(&[pinned("ghc", "9.8")], &catalog(), &env);
        let ghc = status(&report, "ghc");
        assert!(ghc.satisfied());
        assert_eq!(ghc.version.as_ref().unwrap().to_string(), "9.8.4");
        assert!(ghc.alternatives.is_empty());
        assert!(ghc.docs.is_none());
        assert!(report.all_required_satisfied);
    }

    #[test]
    fn unreadable_version_is_not_a_mismatch() {
        // `cardano-up` built with `go install` reports `devel`: never block on it.
        let report = resolve_all(
            &[pinned("cardano-up", "0.15")],
            &catalog(),
            &env(&[], &["cardano-up"]),
        );
        let up = status(&report, "cardano-up");
        assert!(up.satisfied());
        assert!(up.version.is_none());
        assert!(report.all_required_satisfied);
    }

    #[test]
    fn mismatch_json_shape() {
        let env = with_version(env(&["npm"], &["yaci-devkit"]), "yaci-devkit", "0.10.6");
        let report = resolve_all(&[pinned("yaci-devkit", "0.12")], &catalog(), &env);
        let value = serde_json::to_value(&report).unwrap();
        let dep = &value["deps"][0];
        assert_eq!(dep["version"], "0.10.6");
        assert_eq!(dep["required_version"], "0.12");
        assert_eq!(dep["version_mismatch"], true);
        assert!(dep.get("version_conflict").is_none());
        assert_eq!(value["all_required_present"], true);
        assert_eq!(value["all_required_satisfied"], false);
    }

    #[test]
    fn whole_catalog_resolves_without_infinite_recursion() {
        // Cycle guard: resolving every dep id terminates (worst case: empty env).
        let cat = catalog();
        let env = env(&[], &[]);
        for id in cat.dep_ids() {
            let _ = resolve(id, None, &env, &cat, &HashSet::new()); // must return, not hang
        }
    }

    #[test]
    fn report_json_shape_matches_spec() {
        // Present dep → only {id, required, present}; missing and resolved →
        // also {plan, docs} (tech-spec §9.4).
        let report = resolve_all(
            &reqs(&["just", "node"]),
            &catalog(),
            &env(&["brew"], &["just"]),
        );
        let value = serde_json::to_value(&report).unwrap();
        let deps = value["deps"].as_array().unwrap();

        let just = deps.iter().find(|d| d["id"] == "just").unwrap();
        assert_eq!(just["present"], true);
        assert!(just.get("plan").is_none(), "present dep must omit plan");
        assert!(
            just.get("version_mismatch").is_none(),
            "a false `version_mismatch` is omitted"
        );
        assert!(just.get("docs").is_none(), "present dep must omit docs");

        let node = deps.iter().find(|d| d["id"] == "node").unwrap();
        assert_eq!(node["present"], false);
        assert_eq!(node["plan"][0]["installer"], "brew");
        assert_eq!(node["plan"][0]["command"], "brew install node");
        assert!(node["docs"].is_string());
    }

    #[test]
    fn infra_component_needs_every_infra_tools_deps() {
        let registry = crate::registry::loader::Registry::load().unwrap();
        let ids: Vec<&str> = component_deps(INFRA_DRIVER_ID, &registry)
            .into_iter()
            .map(|(_, dep)| dep.id.as_str())
            .collect();
        assert!(ids.contains(&"docker"));
        assert!(ids.contains(&"cardano-up"));
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
        let tooling = present_tooling(&catalog(), &registry, &env(&[], &["rustup", "aikup"]));
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
                    cat.get(&dep.id).is_some(),
                    "tool '{}' requires dep '{}' with no registry/deps.toml entry",
                    tool.id,
                    dep.id
                );
            }
        }
    }
}
