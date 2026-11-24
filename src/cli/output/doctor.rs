//! The `doctor` presenter and the dependency advice block that the generating
//! commands also print after scaffolding.

use std::collections::HashMap;

use comfy_table::ContentArrangement;
use serde_json::json;

use super::emit_json_ok;
use super::layout::{borderless_table, print_dim_rule, print_panel, print_table};
use crate::cli::theme;
use crate::doctor::installers::Installer;
use crate::doctor::probe::{Environment, ScanResult};
use crate::doctor::{self, DepStatus, Report};
use crate::registry::loader::Registry;
use crate::registry::types::Link;

/// Print the standalone `doctor` report: environment, detected components, and
/// dependency status with install advice. The project's editor tooling and
/// help channels are shown only with `links`; otherwise one line offers them
/// (`--json` always carries the tooling as `deps[].support`).
pub fn print_doctor(
    scan: &ScanResult,
    report: &Report,
    env: &Environment,
    tooling: &[String],
    registry: &Registry,
    json: bool,
    links: bool,
) {
    let installers = detected_installers(env);
    let project_detected = !(scan.components.is_empty() && scan.unrecognized.is_empty());

    if json {
        emit_json_ok(&json!({
            "all_required_present": report.all_required_present,
            "all_required_satisfied": report.all_required_satisfied,
            "environment": { "os": env.os, "installers": installers, "tooling": tooling },
            "project_detected": project_detected,
            "components": scan.components,
            "unrecognized": scan.unrecognized,
            "deps": report.deps,
        }));
        return;
    }

    println!();
    print_panel(
        "environment",
        None,
        &environment_rows(env, &installers, tooling),
    );

    println!();
    if project_detected {
        print_components(scan, report, registry);
    } else {
        print_no_project_hint();
    }

    if project_detected || !report.all_required_satisfied {
        print_dep_advice(report);
    }
    if project_detected {
        let support = report
            .deps
            .iter()
            .map(|dep| (dep.id.as_str(), dep.support.as_slice()));
        let community = scan
            .components
            .iter()
            .filter_map(|comp| registry.get(&comp.tool_id))
            .map(|tool| (tool.name.as_str(), tool.community.as_slice()));
        if links {
            print_link_table("Supporting tools", support);
            print_link_table("Get help", community);
        } else if support.chain(community).any(|(_, links)| !links.is_empty()) {
            println!();
            println!(
                "  {} {}",
                theme::dim("Editor tooling and help channels for this project:"),
                theme::command("cardano-init doctor --links")
            );
        }
    }
    println!();
}

/// Sorted installer keys detected on this host.
fn detected_installers(env: &Environment) -> Vec<&str> {
    let mut keys: Vec<&str> = env.installers.iter().map(Installer::key).collect();
    keys.sort_unstable();
    keys
}

/// The `os` / `installers` / `tooling` rows of the environment panel.
fn environment_rows(env: &Environment, installers: &[&str], tooling: &[String]) -> Vec<String> {
    fn list_or_none<S: AsRef<str>>(items: &[S]) -> String {
        if items.is_empty() {
            "none detected".to_string()
        } else {
            items
                .iter()
                .map(AsRef::as_ref)
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    let w = "installers".len();
    vec![
        format!("{:<w$}  {}", "os", theme::accent(format!("{:?}", env.os))),
        format!(
            "{:<w$}  {}",
            "installers",
            theme::accent(list_or_none(installers))
        ),
        format!(
            "{:<w$}  {}",
            "tooling",
            theme::accent(list_or_none(tooling))
        ),
    ]
}

/// One row per detected component, marked by whether the tool's required
/// dependencies are present and new enough (not merely whether it was
/// detected), plus one row per unrecognized directory.
fn print_components(scan: &ScanResult, report: &Report, registry: &Registry) {
    let deps: HashMap<&str, &DepStatus> = report.deps.iter().map(|d| (d.id.as_str(), d)).collect();

    let mut table = borderless_table();
    for comp in &scan.components {
        let tool = registry.get(&comp.tool_id);
        let name = tool.map_or(comp.tool_id.as_str(), |t| t.name.as_str());
        let mut tool_deps: Vec<&DepStatus> = doctor::component_deps(&comp.tool_id, registry)
            .into_iter()
            .filter_map(|(_, d)| deps.get(d.id.as_str()).copied())
            .collect();
        tool_deps.sort_by_key(|d| d.id.as_str());
        tool_deps.dedup_by_key(|d| d.id.as_str());
        let missing: Vec<&str> = tool_deps
            .iter()
            .filter(|d| !d.present)
            .map(|d| d.id.as_str())
            .collect();
        // The versions are in the advice below; here just flag it.
        let mismatch = tool_deps.iter().any(|d| d.version_mismatch);

        let problems: Vec<String> = [
            (!missing.is_empty()).then(|| format!("missing: {}", missing.join(", "))),
            mismatch.then(|| "wrong version".to_string()),
        ]
        .into_iter()
        .flatten()
        .collect();
        let (mark, note) = if problems.is_empty() {
            (theme::ok().to_string(), String::new())
        } else {
            (
                theme::caution().to_string(),
                theme::warn(problems.join("; ")).to_string(),
            )
        };
        table.add_row(vec![
            mark,
            format!("{}:", comp.kind),
            format!(
                "{}{}",
                theme::icon_prefix_or_blank(tool),
                theme::accent(name)
            ),
            note,
        ]);
    }
    for un in &scan.unrecognized {
        table.add_row(vec![
            theme::unknown().to_string(),
            format!("{}/", un.dir),
            theme::dim("unrecognized (renamed or modified?)").to_string(),
            String::new(),
        ]);
    }
    print_table(table, theme::PAD);
}

fn print_no_project_hint() {
    println!(
        "  {} {}",
        theme::warn_mark(),
        theme::strong("No Cardano project detected in this directory.")
    );
    println!(
        "    {} {} {} {}",
        theme::accent("→"),
        theme::dim("run"),
        theme::command("cardano-init"),
        theme::dim("to scaffold one, or cd into a generated project")
    );
}

/// A titled `owner  name  url` table, with each owner named on its first link
/// only, all dimmed: reference material, secondary to the dependency status.
/// Prints nothing when there are no links.
fn print_link_table<'a>(title: &str, groups: impl Iterator<Item = (&'a str, &'a [Link])>) {
    let mut table = borderless_table();
    table.set_content_arrangement(ContentArrangement::Disabled);
    for (owner, links) in groups {
        for (i, link) in links.iter().enumerate() {
            let label = if i == 0 { owner } else { "" };
            table.add_row(vec![
                theme::dim(label).to_string(),
                theme::dim(&link.name).to_string(),
                theme::dim(theme::link(&link.url)).to_string(),
            ]);
        }
    }
    if table.is_empty() {
        return;
    }
    println!();
    print_dim_rule(title);
    print_table(table, theme::PAD);
}

/// Print the dependency status badge and, when required deps are missing or
/// at another version, how to install each of them.
pub(super) fn print_dep_advice(report: &Report) {
    println!();
    let conflicts = print_version_conflicts(report);
    if report.all_required_satisfied {
        let note = if conflicts {
            " (version pins conflict, see above)"
        } else {
            ""
        };
        println!(
            "  {}  all required dependencies present{note}",
            theme::badge_ok("DEPS OK")
        );
        return;
    }

    let (badge, lead_in) = if report.all_required_present {
        ("DEPS MISMATCH", "install the pinned versions below")
    } else if report.deps.iter().any(|d| d.version_mismatch) {
        ("DEPS MISSING", "install or upgrade the dependencies below")
    } else {
        ("DEPS MISSING", "install the required dependencies below")
    };
    println!("  {}  {lead_in}", theme::badge_warn(badge));
    for dep in report.unsatisfied_required() {
        print_unsatisfied_dep(dep);
    }
}

/// Deps whose selected tools pin versions no single install satisfies —
/// typically tools parked on different eras. Returns whether any was printed.
fn print_version_conflicts(report: &Report) -> bool {
    let mut any = false;
    for dep in report
        .deps
        .iter()
        .filter(|d| !d.version_conflict.is_empty())
    {
        any = true;
        let pins = dep
            .version_conflict
            .iter()
            .map(|p| format!("{} needs {}", p.tool, p.version.label()))
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "  {} {}  {}",
            theme::caution(),
            theme::strong(&dep.id),
            theme::warn(format!("conflicting version pins: {pins}"))
        );
    }
    any
}

/// Mark a command that installs the installer's latest instead of the pin.
fn with_unpinned_note(status: String, unpinned: bool) -> String {
    if unpinned {
        format!("{status} {}", theme::dim("· installs latest"))
    } else {
        status
    }
}

/// Indent for a dependency's detail lines, one level under its header.
const DEP_INDENT: &str = "      ";

/// One missing or mismatched dependency: a header, an aligned `command  status`
/// table (the recommended path, then the alternatives), and the docs link.
fn print_unsatisfied_dep(dep: &DepStatus) {
    println!();

    let has_methods = !dep.plan.is_empty() || !dep.alternatives.is_empty();
    let lead_in = match (dep.version_mismatch, &dep.version, &dep.required_version) {
        (true, Some(version), Some(want)) if has_methods => {
            format!(
                "{version} installed, needs {} — install it with any of:",
                want.label()
            )
        }
        (true, Some(version), Some(want)) => {
            format!(
                "{version} installed, needs {} — no automatic install method for this system",
                want.label()
            )
        }
        _ if has_methods => "not installed — install with any of:".to_string(),
        _ => "not installed — no automatic install method for this system".to_string(),
    };
    println!(
        "  {} {}  {}",
        theme::caution(),
        theme::strong(&dep.id),
        theme::dim(lead_in)
    );

    if has_methods {
        let mut table = borderless_table();
        // Keep long commands (curl one-liners) on one line.
        table.set_content_arrangement(ContentArrangement::Disabled);

        if !dep.plan.is_empty() {
            // A bootstrap chain becomes one copy-pasteable line.
            let command = dep
                .plan
                .iter()
                .map(|s| s.command.as_str())
                .collect::<Vec<_>>()
                .join(" && ");
            let unpinned = dep.plan.last().is_some_and(|s| s.unpinned);
            table.add_row(vec![
                theme::command(command).to_string(),
                with_unpinned_note(theme::good("recommended").to_string(), unpinned),
            ]);
        }
        for alt in &dep.alternatives {
            let status = if alt.available {
                theme::dim("ready").to_string()
            } else {
                theme::warn(format!("requires {}", alt.installer)).to_string()
            };
            table.add_row(vec![
                theme::command(&alt.command).to_string(),
                with_unpinned_note(status, alt.unpinned),
            ]);
        }
        print_table(table, DEP_INDENT);
    }

    if let Some(docs) = &dep.docs {
        println!(
            "{}{} {}",
            DEP_INDENT,
            theme::dim("docs:"),
            theme::link(docs)
        );
    }
}
