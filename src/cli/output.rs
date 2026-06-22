use console::style;
use serde_json::json;

use super::{CliError, Format};
use crate::doctor::Report;
use crate::doctor::installers::Installer;
use crate::doctor::probe::{Environment, ScanResult};
use crate::registry::loader::Registry;
use crate::registry::types::{Role, Selection};
use crate::scaffold::planner::FilePlan;

// ---------------------------------------------------------------------------
// JSON envelope (TECH_SPEC §2.4)
// ---------------------------------------------------------------------------

const SCHEMA_VERSION: u32 = 1;

/// Print a success envelope to stdout: `{ schema_version, ok: true, data }`.
fn emit_json_ok(data: serde_json::Value) {
    let envelope = json!({ "schema_version": SCHEMA_VERSION, "ok": true, "data": data });
    println!(
        "{}",
        serde_json::to_string(&envelope).expect("envelope serializes")
    );
}

/// Render any error in the requested format.
///
/// In `json`, an error envelope `{ ok: false, error: { code, message, context } }`
/// is written to stderr. In `human`, the styled `error: …` line is written to
/// stderr — except for an interactive abort (exit code 0), which is silent.
pub fn print_error(err: &CliError, format: Format) {
    match format {
        Format::Json => {
            let envelope = json!({
                "schema_version": SCHEMA_VERSION,
                "ok": false,
                "error": {
                    "code": err.code(),
                    "message": err.to_string(),
                    "context": err.context(),
                },
            });
            eprintln!(
                "{}",
                serde_json::to_string(&envelope).expect("envelope serializes")
            );
        }
        Format::Human => {
            if err.exit_code() != 0 {
                eprintln!("{}: {}", style("error").red().bold(), err);
            }
        }
    }
}

/// Print the welcome banner for interactive mode.
pub fn print_welcome() {
    println!();
    println!(
        "  {} Let's set up your Cardano protocol project.",
        style("Welcome to cardano-init!").bold()
    );
    println!();
    println!("  A Cardano protocol typically has up to five components:");
    println!(
        "  {} Smart contract logic (validators) that runs on the ledger",
        style("On-chain:").cyan().bold()
    );
    println!(
        "  {} Code that builds and submits transactions",
        style("Off-chain:").cyan().bold()
    );
    println!(
        "  {} Indexers and services that read chain data",
        style("Infrastructure:").cyan().bold()
    );
    println!(
        "  {} Local throwaway chain to develop and test against",
        style("Devnet:").cyan().bold()
    );
    println!(
        "  {} Specification and automated verification tools",
        style("Formal methods:").cyan().bold()
    );
    println!();
}

/// Print a summary of the selection before generation.
pub fn print_summary(selection: &Selection, registry: &Registry) {
    println!();
    println!("  {}", style("Summary").bold().underlined());
    println!();
    println!("  Project:  {}", style(&selection.project_name).cyan());

    for assignment in &selection.assignments {
        let role_label = match assignment.role {
            Role::OnChain => "On-chain",
            Role::OffChain => "Off-chain",
            Role::Infrastructure => "Infra",
            Role::Devnet => "Devnet",
            Role::FormalMethods => "Formal methods",
        };

        let tool_info = if let Some(tool) = registry.get(&assignment.tool_id) {
            let lang = tool.languages.first().map(|s| s.as_str()).unwrap_or("?");
            format!("{} ({})", tool.name, lang)
        } else {
            assignment.tool_id.clone()
        };

        println!(
            "  {:<12}{}",
            format!("{}:", role_label),
            style(tool_info).cyan()
        );
    }

    println!("  Network:  {}", style(&selection.network).cyan());

    if selection.nix {
        println!("  Nix:      {}", style("yes").green());
    }
    println!();
}

/// Build the `[{ role, tool }]` component list for a selection.
fn components_json(selection: &Selection) -> serde_json::Value {
    let items: Vec<serde_json::Value> = selection
        .assignments
        .iter()
        .map(|a| json!({ "role": a.role.as_kebab(), "tool": a.tool_id }))
        .collect();
    serde_json::Value::Array(items)
}

/// Print the dry-run output: summary + nested file tree (human), or the planned
/// file list (json).
pub fn print_dry_run(selection: &Selection, registry: &Registry, plan: &FilePlan, format: Format) {
    if format == Format::Json {
        let files: Vec<&str> = plan
            .entries
            .iter()
            .map(|e| e.dest.to_str().expect("paths are UTF-8"))
            .collect();
        emit_json_ok(json!({
            "project": selection.project_name,
            "network": selection.network.to_string(),
            "nix": selection.nix,
            "dry_run": true,
            "generated": false,
            "components": components_json(selection),
            "files": files,
        }));
        return;
    }

    print_summary(selection, registry);

    println!("  {}", style(format!("{}/", selection.project_name)).bold());

    let paths: Vec<Vec<&str>> = plan
        .entries
        .iter()
        .map(|e| {
            e.dest
                .to_str()
                .expect("paths are UTF-8")
                .split('/')
                .collect()
        })
        .collect();

    print_tree(&paths, 0, 0, &mut String::new());

    println!();
    println!(
        "  {} files would be generated.",
        style(plan.entries.len()).bold()
    );
    println!();
}

/// Recursively print a directory tree from a sorted list of split paths.
///
/// `paths` contains only entries whose prefix (components 0..depth) matches
/// the current branch. `depth` is the current tree level. `indent` is the
/// prefix string built from the ancestors' box-drawing connectors.
fn print_tree(paths: &[Vec<&str>], depth: usize, _start: usize, indent: &mut String) {
    // Group entries by their component at `depth`.
    // Preserve insertion order so the tree follows the plan order.
    let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        if depth >= path.len() {
            continue;
        }
        let key = path[depth];
        if let Some(group) = groups.iter_mut().find(|(k, _)| *k == key) {
            group.1.push(i);
        } else {
            groups.push((key, vec![i]));
        }
    }

    let total = groups.len();
    for (gi, (name, indices)) in groups.iter().enumerate() {
        let is_last = gi == total - 1;
        let connector = if is_last { "└── " } else { "├── " };

        // Check if this is a directory (has children deeper than depth+1)
        let is_dir = indices.iter().any(|&i| paths[i].len() > depth + 1);

        if is_dir {
            println!(
                "  {}{}{}",
                indent,
                style(connector).dim(),
                style(format!("{name}/")).dim()
            );
        } else {
            println!("  {}{}{}", indent, style(connector).dim(), name);
        }

        // Recurse into children that have more components
        let children: Vec<Vec<&str>> = indices
            .iter()
            .filter(|&&i| paths[i].len() > depth + 1)
            .map(|&i| paths[i].clone())
            .collect();

        if !children.is_empty() {
            let extension = if is_last { "    " } else { "│   " };
            let prev_len = indent.len();
            indent.push_str(extension);
            print_tree(&children, depth + 1, 0, indent);
            indent.truncate(prev_len);
        }
    }
}

/// Print success after scaffolding, including the dependency check-and-advise
/// (TECH_SPEC §9). In `json`, emits one envelope carrying the selection plus the
/// dependency report.
pub fn print_success(selection: &Selection, report: &Report, format: Format) {
    if format == Format::Json {
        emit_json_ok(json!({
            "project": selection.project_name,
            "network": selection.network.to_string(),
            "nix": selection.nix,
            "generated": true,
            "components": components_json(selection),
            "dependencies": report,
        }));
        return;
    }

    println!();
    println!(
        "  {} Created {}",
        style("✔").green().bold(),
        style(&selection.project_name).cyan().bold()
    );

    for assignment in &selection.assignments {
        let role_label = match assignment.role {
            Role::OnChain => "on-chain",
            Role::OffChain => "off-chain",
            Role::Infrastructure => "infrastructure",
            Role::Devnet => "devnet",
            Role::FormalMethods => "formal-methods",
        };
        println!(
            "  {} Scaffolded {} ({})",
            style("✔").green().bold(),
            role_label,
            &assignment.tool_id
        );
    }

    // Check-and-advise: surface any missing required deps before "Next steps".
    print_dep_advice(report);

    println!();
    println!("  {}", style("Next steps:").bold());
    println!("    cd {}", selection.project_name);
    if !report.all_required_present {
        println!("    # install the missing dependencies listed above, then:");
    }
    println!("    just build");
    println!();
}

/// Print the dependency advice block (missing required deps + install plans).
/// Silent when everything required is already present.
fn print_dep_advice(report: &Report) {
    if report.all_required_present {
        println!();
        println!(
            "  {} All required dependencies are installed.",
            style("✔").green().bold()
        );
        return;
    }

    println!();
    println!("  {}", style("Missing dependencies:").yellow().bold());
    for dep in report.missing_required() {
        print_missing_dep(dep);
    }
}

/// Render one missing dependency: its id, ordered install commands, and docs.
fn print_missing_dep(dep: &crate::doctor::DepStatus) {
    println!();
    println!(
        "  {} {} (required)",
        style("✘").red().bold(),
        style(&dep.id).bold()
    );
    for step in &dep.plan {
        println!("      {}", style(&step.command).cyan());
    }
    if dep.plan.is_empty() {
        println!(
            "      {}",
            style("(no install method detected for this system)").dim()
        );
    }
    if let Some(docs) = &dep.docs {
        println!(
            "      {} {}",
            style("Docs:").dim(),
            style(docs).underlined()
        );
    }
}

/// Sorted installer keys detected on this host.
fn detected_installers(env: &Environment) -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = Installer::ALL
        .iter()
        .filter(|i| env.installers.contains(i))
        .map(|i| i.key())
        .collect();
    keys.sort_unstable();
    keys
}

/// Print the standalone `doctor` report: environment + detected components +
/// dependency status.
pub fn print_doctor(
    scan: &ScanResult,
    report: &Report,
    env: &Environment,
    registry: &Registry,
    format: Format,
) {
    let installers = detected_installers(env);

    if format == Format::Json {
        emit_json_ok(json!({
            "all_required_present": report.all_required_present,
            "environment": { "os": env.os, "installers": installers },
            "components": scan.components,
            "unrecognized": scan.unrecognized,
            "deps": report.deps,
        }));
        return;
    }

    println!();
    println!("  {}", style("Dependency check").bold().underlined());

    // Environment.
    println!();
    let installer_list = if installers.is_empty() {
        "none detected".to_string()
    } else {
        installers.join(", ")
    };
    println!(
        "  {} {:?}   {} {}",
        style("OS:").dim(),
        env.os,
        style("Installers:").dim(),
        installer_list
    );

    // Detected components. The mark reflects whether the tool's required
    // dependencies are present, not merely that the component was detected.
    let present: std::collections::HashSet<&str> = report
        .deps
        .iter()
        .filter(|d| d.present)
        .map(|d| d.id.as_str())
        .collect();

    println!();
    if scan.components.is_empty() && scan.unrecognized.is_empty() {
        println!(
            "  {}",
            style("No generated components detected in this directory.").dim()
        );
    } else {
        for comp in &scan.components {
            let tool = registry.get(&comp.tool_id);
            let name = tool.map(|t| t.name.as_str()).unwrap_or(&comp.tool_id);
            let missing: Vec<&str> = tool
                .map(|t| {
                    t.system_deps
                        .iter()
                        .map(|d| d.as_str())
                        .filter(|d| !present.contains(d))
                        .collect()
                })
                .unwrap_or_default();

            if missing.is_empty() {
                println!(
                    "  {} {}: {}",
                    style("✔").green().bold(),
                    comp.role,
                    style(name).cyan()
                );
            } else {
                println!(
                    "  {} {}: {} {}",
                    style("✘").red().bold(),
                    comp.role,
                    style(name).cyan(),
                    style(format!("(missing: {})", missing.join(", "))).yellow()
                );
            }
        }
        for un in &scan.unrecognized {
            println!(
                "  {} {}/ — unrecognized (renamed or modified?)",
                style("?").yellow().bold(),
                un.dir
            );
        }
    }

    // Dependency status.
    println!();
    for dep in &report.deps {
        if dep.present {
            println!("  {} {}", style("✔").green().bold(), dep.id);
        }
    }
    print_dep_advice(report);
    println!();
}

/// Print the registry (roles + tools) for `cardano-init list`: human-readable
/// by default, or the TECH_SPEC §8 JSON payload with `--format json`.
pub fn print_list(registry: &Registry, format: Format) {
    use crate::registry::view;

    if format == Format::Json {
        emit_json_ok(json!({
            "roles": view::role_views(),
            "tools": view::tool_views(registry),
        }));
        return;
    }

    println!();
    println!("  {}", style("Roles").bold().underlined());
    println!();
    for role in view::role_views() {
        let multi = if role.multiple { "  (multiple)" } else { "" };
        println!(
            "  {:<16}{:<18}{}{}",
            style(role.id).cyan(),
            role.display,
            style(format!("dir: {}", role.dir)).dim(),
            style(multi).dim()
        );
    }

    println!();
    println!("  {}", style("Tools").bold().underlined());
    // Reuse the same per-tool block as `--help` so the two can't drift; sort by
    // id to match the JSON ordering.
    let mut tools: Vec<&crate::registry::types::ToolDef> = registry.all_tools().iter().collect();
    tools.sort_by(|a, b| a.id.cmp(&b.id));
    let mut block = String::new();
    for tool in tools {
        block.push('\n');
        super::format_tool(&mut block, tool);
    }
    print!("{block}");
    println!();
}

/// Truncate a tool description to the first sentence for use in prompts.
pub fn first_sentence(desc: &str) -> &str {
    // Find the first period followed by whitespace or end-of-string
    if let Some(pos) = desc.find(". ") {
        &desc[..=pos]
    } else if let Some(pos) = desc.find(".\n") {
        &desc[..=pos]
    } else if desc.ends_with('.') {
        desc
    } else {
        // No sentence boundary — take first 80 chars
        let end = desc
            .char_indices()
            .nth(80)
            .map(|(i, _)| i)
            .unwrap_or(desc.len());
        &desc[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sentence_with_period_space() {
        assert_eq!(
            first_sentence("Hello world. More text here."),
            "Hello world."
        );
    }

    #[test]
    fn first_sentence_no_period() {
        assert_eq!(first_sentence("No period here"), "No period here");
    }

    #[test]
    fn first_sentence_ends_with_period() {
        assert_eq!(first_sentence("Ends with period."), "Ends with period.");
    }
}
