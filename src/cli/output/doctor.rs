//! The `doctor` presenter and the dependency advice block that the generating
//! commands also print after scaffolding.

use std::collections::HashSet;

use comfy_table::ContentArrangement;
use serde_json::json;

use super::emit_json_ok;
use super::layout::{borderless_table, print_panel, print_rule, print_table};
use crate::cli::theme;
use crate::doctor::installers::Installer;
use crate::doctor::probe::{Environment, ScanResult};
use crate::doctor::{DepStatus, Report};
use crate::registry::loader::Registry;
use crate::registry::types::Link;

/// Print the standalone `doctor` report: environment, detected components, and
/// dependency status with install advice.
pub fn print_doctor(
    scan: &ScanResult,
    report: &Report,
    env: &Environment,
    tooling: &[String],
    registry: &Registry,
    json: bool,
) {
    let installers = detected_installers(env);
    let project_detected = !(scan.components.is_empty() && scan.unrecognized.is_empty());

    if json {
        emit_json_ok(&json!({
            "all_required_present": report.all_required_present,
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

    if project_detected || !report.all_required_present {
        print_dep_advice(report);
    }
    if project_detected {
        print_link_table(
            "Supporting tools",
            report
                .deps
                .iter()
                .map(|dep| (dep.id.as_str(), dep.support.as_slice())),
        );
        print_link_table(
            "Get help",
            scan.components
                .iter()
                .filter_map(|comp| registry.get(&comp.tool_id))
                .map(|tool| (tool.name.as_str(), tool.community.as_slice())),
        );
    }
    println!();
}

/// Sorted installer keys detected on this host.
fn detected_installers(env: &Environment) -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = Installer::ALL
        .iter()
        .filter(|i| env.installers.contains(i))
        .copied()
        .map(Installer::key)
        .collect();
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
/// dependencies are present (not merely whether it was detected), plus one row
/// per unrecognized directory.
fn print_components(scan: &ScanResult, report: &Report, registry: &Registry) {
    let present: HashSet<&str> = report
        .deps
        .iter()
        .filter(|d| d.present)
        .map(|d| d.id.as_str())
        .collect();

    let mut table = borderless_table();
    for comp in &scan.components {
        let tool = registry.get(&comp.tool_id);
        let name = tool.map_or(comp.tool_id.as_str(), |t| t.name.as_str());
        let missing: Vec<&str> = tool
            .map(|t| {
                t.system_deps
                    .iter()
                    .map(String::as_str)
                    .filter(|d| !present.contains(d))
                    .collect()
            })
            .unwrap_or_default();

        let (mark, note) = if missing.is_empty() {
            (theme::ok().to_string(), String::new())
        } else {
            (
                theme::caution().to_string(),
                theme::warn(format!("missing: {}", missing.join(", "))).to_string(),
            )
        };
        table.add_row(vec![
            mark,
            format!("{}:", comp.kind),
            theme::accent(name).to_string(),
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
/// only. Prints nothing when there are no links.
fn print_link_table<'a>(title: &str, groups: impl Iterator<Item = (&'a str, &'a [Link])>) {
    let mut table = borderless_table();
    table.set_content_arrangement(ContentArrangement::Disabled);
    for (owner, links) in groups {
        for (i, link) in links.iter().enumerate() {
            let label = if i == 0 { owner } else { "" };
            table.add_row(vec![
                theme::strong(label).to_string(),
                link.name.clone(),
                theme::link(&link.url).to_string(),
            ]);
        }
    }
    if table.is_empty() {
        return;
    }
    println!();
    print_rule(title);
    print_table(table, theme::PAD);
}

/// Print the dependency status badge and, when required deps are missing, how
/// to install each of them.
pub(super) fn print_dep_advice(report: &Report) {
    println!();
    if report.all_required_present {
        println!(
            "  {}  all required dependencies present",
            theme::badge_ok("DEPS OK")
        );
        return;
    }

    println!(
        "  {}  install the required dependencies below",
        theme::badge_warn("DEPS MISSING")
    );
    for dep in report.missing_required() {
        print_missing_dep(dep);
    }
}

/// Indent for a dependency's detail lines, one level under its header.
const DEP_INDENT: &str = "      ";

/// One missing dependency: a header, an aligned `command  status` table (the
/// recommended path, then the alternatives), and the docs link.
fn print_missing_dep(dep: &DepStatus) {
    println!();

    let has_methods = !dep.plan.is_empty() || !dep.alternatives.is_empty();
    let lead_in = if has_methods {
        "not installed — install with any of:"
    } else {
        "not installed — no automatic install method for this system"
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
            table.add_row(vec![
                theme::command(command).to_string(),
                theme::good("recommended").to_string(),
            ]);
        }
        for alt in &dep.alternatives {
            let status = if alt.available {
                theme::dim("ready").to_string()
            } else {
                theme::warn(format!("requires {}", alt.installer)).to_string()
            };
            table.add_row(vec![theme::command(&alt.command).to_string(), status]);
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
