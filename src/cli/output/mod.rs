//! The presenter: renders results and errors as styled human text or as the
//! JSON envelope (tech spec §2.4). All terminal styling goes through
//! [`theme`].

mod doctor;
mod layout;
mod list;
mod notice;
mod update;

use serde_json::json;

pub use doctor::print_doctor;
pub use list::print_list;
pub use notice::print_update_notice;
pub use update::{print_update_plan, print_update_success};

use self::doctor::print_dep_advice;
use self::layout::{panel_lines, print_panel, print_rule, wrap};
use super::CliError;
use super::git::InitOutcome;
use super::theme;
use crate::contract::DIR_PROTOCOL;
use crate::doctor::Report;
use crate::registry::compat::Incompatibility;
use crate::registry::loader::Registry;
use crate::registry::types::{Role, Selection, ToolDef};
use crate::scaffold::planner::{self, FilePlan};

const SCHEMA_VERSION: u32 = 1;

/// Print a success envelope to stdout: `{ schema_version, ok: true, data }`.
fn emit_json_ok(data: &serde_json::Value) {
    let envelope = json!({ "schema_version": SCHEMA_VERSION, "ok": true, "data": data });
    println!("{envelope}");
}

/// Render an error to stderr: the JSON error envelope with `json`, otherwise a
/// red ` ERR (<code>) ` pill, the message, and the `help` remedy in a panel. An
/// interactive abort (exit code 0) is a user choice and prints nothing.
pub fn print_error(err: &CliError, json: bool) {
    if json {
        let envelope = json!({
            "schema_version": SCHEMA_VERSION,
            "ok": false,
            "error": {
                "code": err.code(),
                "message": err.to_string(),
                "context": err.context(),
            },
        });
        eprintln!("{envelope}");
    } else if err.exit_code() != 0 {
        print_human_error(err);
    }
}

fn print_human_error(err: &CliError) {
    eprintln!();

    // The message starts on the pill's line; continuation lines align under it.
    let pill = theme::badge_err(&format!("ERR ({})", err.code()));
    let text_col = theme::PAD.len() + console::measure_text_width(&pill.to_string()) + 2;
    let indent = " ".repeat(text_col);
    let wrap_width = 80usize.saturating_sub(text_col).max(36);
    let lines = wrap(&err.to_string(), wrap_width);

    match lines.split_first() {
        Some((first, rest)) => {
            eprintln!("{}{}  {}", theme::PAD, pill, first);
            for line in rest {
                eprintln!("{indent}{line}");
            }
        }
        None => eprintln!("{}{}", theme::PAD, pill),
    }

    if let Some(help) = miette::Diagnostic::help(err).map(|h| h.to_string()) {
        let rows: Vec<String> = help.lines().map(str::to_string).collect();
        eprintln!();
        for line in panel_lines("help", None, &rows) {
            eprintln!("{line}");
        }
    }
    eprintln!();
}

/// Print the welcome banner for interactive mode.
pub fn print_welcome() {
    println!();
    println!(
        "  {} Let's set up your Cardano protocol project.",
        theme::strong("Welcome to cardano-init!")
    );
    println!();
    println!("  A Cardano protocol typically has up to five components:");
    println!(
        "  {} Smart contract logic (validators) that runs on the ledger",
        theme::accent_strong("On-chain:")
    );
    println!(
        "  {} Code that builds and submits transactions",
        theme::accent_strong("Off-chain:")
    );
    println!(
        "  {} Indexers and services that read chain data",
        theme::accent_strong("Infrastructure:")
    );
    println!(
        "  {} Local throwaway chain to develop and test against",
        theme::accent_strong("Devnet:")
    );
    println!(
        "  {} Specification and automated verification tools",
        theme::accent_strong("Formal methods:")
    );
    println!();
}

/// The tag appended to an experimental tool's name in prompts (`""` for
/// released tools).
pub fn experimental_tag(tool: &ToolDef) -> &'static str {
    if tool.experimental {
        " [experimental]"
    } else {
        ""
    }
}

/// The distinct experimental tools in a selection, in assignment order (a
/// fullstack tool assigned to two roles is reported once).
pub fn selected_experimental_tools<'a>(
    selection: &Selection,
    registry: &'a Registry,
) -> Vec<&'a ToolDef> {
    let mut out: Vec<&ToolDef> = Vec::new();
    for a in &selection.assignments {
        if let Some(tool) = registry.get(&a.tool_id)
            && tool.experimental
            && !out.iter().any(|t| t.id == tool.id)
        {
            out.push(tool);
        }
    }
    out
}

/// Remind the user that the selection includes experimental tools (they have
/// already opted in). Prints nothing otherwise.
fn print_experimental_warning(selection: &Selection, registry: &Registry) {
    let experimental_tools = selected_experimental_tools(selection, registry);
    if experimental_tools.is_empty() {
        return;
    }

    let names = experimental_tools
        .iter()
        .map(|t| t.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    println!();
    println!(
        "  {} {} {}",
        theme::warn_mark(),
        theme::warn_strong("Experimental:"),
        theme::warn_strong(names)
    );
    println!(
        "    {}",
        theme::warn("These tools may be unstable or incomplete (the tool itself and/or its")
    );
    println!(
        "    {}",
        theme::warn("integration). Expect rough edges and breaking changes.")
    );
}

/// Warn that an incompatible off-chain ↔ provider pairing is being scaffolded
/// anyway (`--ignore-warning`).
pub fn print_incompatibility_warning(inc: &Incompatibility) {
    let providers = inc
        .providers
        .iter()
        .map(|p| p.name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    println!();
    println!(
        "  {} {} {} + {}",
        theme::warn_mark(),
        theme::warn_strong("Incompatible (ignored):"),
        theme::warn_strong(&inc.off_chain_name),
        theme::warn_strong(providers),
    );
    println!("    {}", theme::warn(&inc.reason));
    println!(
        "    {}",
        theme::warn(
            "Scaffolding anyway (--ignore-warning); the generated project may not run as-is."
        )
    );
}

/// Row labels for the non-component lines of the summary panel.
const NETWORK_LABEL: &str = "network";
const NIX_LABEL: &str = "nix";
const SPEC_LABEL: &str = "spec";

/// The aligned `role  tool · lang` rows (then `network`, `nix` if set, and
/// `spec`) of the selection panel, shared by the summary and the success panel.
fn summary_rows(selection: &Selection, registry: &Registry) -> Vec<String> {
    let comps = components(selection, registry);

    let label_w = comps
        .iter()
        .map(|c| c.kebab.len())
        .chain([NETWORK_LABEL.len(), NIX_LABEL.len(), SPEC_LABEL.len()])
        .max()
        .unwrap_or(0);
    let name_w = comps
        .iter()
        .map(|c| {
            registry
                .get(c.tool_id)
                .map_or(c.tool_id.len(), |t| t.name.len())
        })
        .max()
        .unwrap_or(0);

    let mut rows: Vec<String> = Vec::new();
    for c in &comps {
        let tool = registry.get(c.tool_id);
        let name = tool.map_or(c.tool_id, |t| t.name.as_str());
        let lang = tool.and_then(|t| t.languages.first());

        let name_cell = theme::accent(format!("{name:<name_w$}"));
        let lang_cell = lang.map_or_else(String::new, |l| theme::dim(format!("· {l}")).to_string());
        let tag = if tool.is_some_and(|t| t.experimental) {
            theme::warn(" [experimental]").to_string()
        } else {
            String::new()
        };
        rows.push(format!(
            "{:<label_w$}  {name_cell}  {lang_cell}{tag}",
            c.kebab
        ));
    }
    rows.push(format!(
        "{NETWORK_LABEL:<label_w$}  {}",
        theme::accent(&selection.network)
    ));
    if selection.nix {
        rows.push(format!("{NIX_LABEL:<label_w$}  {}", theme::good("yes")));
    }
    let spec = if selection.spec {
        theme::good("yes").to_string()
    } else {
        theme::dim("no").to_string()
    };
    rows.push(format!("{SPEC_LABEL:<label_w$}  {spec}"));
    rows
}

/// Print the selection as a panel titled with the project name, before
/// generation (interactive confirm step and dry-run).
pub fn print_summary(selection: &Selection, registry: &Registry) {
    println!();
    print_panel(
        &selection.project_name,
        None,
        &summary_rows(selection, registry),
    );
    print_experimental_warning(selection, registry);
    println!();
}

/// A generated component as reported to the user.
struct Component<'a> {
    /// Role kebab, or `protocol` for a collapsed fullstack component.
    kebab: &'static str,
    tool_id: &'a str,
}

/// The components actually scaffolded: a fullstack on-chain + off-chain pair is
/// folded into one `protocol` component, matching the generated tree.
fn components<'a>(selection: &'a Selection, registry: &Registry) -> Vec<Component<'a>> {
    let fullstack_id = planner::fullstack_tool_id(selection, registry);
    planner::canonical_assignments(selection)
        .into_iter()
        .filter_map(|a| {
            let in_protocol = planner::is_fullstack_member(a, fullstack_id.as_deref());
            let kebab = match (in_protocol, a.role) {
                // The on-chain half stands for the pair.
                (true, Role::OffChain) => return None,
                (true, _) => DIR_PROTOCOL,
                (false, role) => role.as_kebab(),
            };
            Some(Component {
                kebab,
                tool_id: &a.tool_id,
            })
        })
        .collect()
}

/// The `[{ role, tool, experimental }]` component list (collapse-aware).
fn components_json(selection: &Selection, registry: &Registry) -> serde_json::Value {
    components(selection, registry)
        .iter()
        .map(|c| {
            let experimental = registry.get(c.tool_id).is_some_and(|t| t.experimental);
            json!({ "role": c.kebab, "tool": c.tool_id, "experimental": experimental })
        })
        .collect()
}

/// The selection fields every generating command reports, merged with the
/// command-specific `extra` object.
fn selection_json(
    selection: &Selection,
    registry: &Registry,
    extra: serde_json::Value,
) -> serde_json::Value {
    let mut data = json!({
        "project": selection.project_name,
        "network": selection.network.to_string(),
        "nix": selection.nix,
        "spec": selection.spec,
        "components": components_json(selection, registry),
    });
    if let (Some(data), serde_json::Value::Object(extra)) = (data.as_object_mut(), extra) {
        data.extend(extra);
    }
    data
}

/// Print the dry-run: the summary and the file tree, or the planned file list
/// as JSON.
pub fn print_dry_run(selection: &Selection, registry: &Registry, plan: &FilePlan, json: bool) {
    let files: Vec<&str> = plan
        .entries
        .iter()
        .map(|e| e.dest.to_str().expect("paths are UTF-8"))
        .collect();

    if json {
        emit_json_ok(&selection_json(
            selection,
            registry,
            json!({ "dry_run": true, "generated": false, "files": files }),
        ));
        return;
    }

    print_summary(selection, registry);
    println!(
        "  {}",
        theme::strong(format!("{}/", selection.project_name))
    );
    let paths: Vec<Vec<&str>> = files.iter().map(|f| f.split('/').collect()).collect();
    let paths: Vec<&[&str]> = paths.iter().map(Vec::as_slice).collect();
    print_tree(&paths, 0, &mut String::new());

    println!();
    println!(
        "  {} files would be generated.",
        theme::strong(plan.entries.len())
    );
    println!();
}

/// Print the level-`depth` entries of `paths` (split file paths, in plan order)
/// as a box-drawing tree, recursing into directories. `indent` is the prefix
/// built from the ancestors' connectors.
fn print_tree(paths: &[&[&str]], depth: usize, indent: &mut String) {
    // Group by the component at `depth`, keeping first-seen order.
    let mut groups: Vec<(&str, Vec<&[&str]>)> = Vec::new();
    for &path in paths {
        let Some(&name) = path.get(depth) else {
            continue;
        };
        match groups.iter_mut().find(|(n, _)| *n == name) {
            Some((_, members)) => members.push(path),
            None => groups.push((name, vec![path])),
        }
    }

    for (i, (name, members)) in groups.iter().enumerate() {
        let is_last = i + 1 == groups.len();
        let connector = theme::dim(if is_last { "└── " } else { "├── " });
        let children: Vec<&[&str]> = members
            .iter()
            .copied()
            .filter(|p| p.len() > depth + 1)
            .collect();

        if children.is_empty() {
            println!("  {indent}{connector}{name}");
        } else {
            println!("  {indent}{connector}{}", theme::dim(format!("{name}/")));
            let prev_len = indent.len();
            indent.push_str(if is_last { "    " } else { "│   " });
            print_tree(&children, depth + 1, indent);
            indent.truncate(prev_len);
        }
    }
}

/// Report a scaffolded project: the selection panel with a `CREATED` badge,
/// dependency advice (tech spec §9) and next steps; or one JSON envelope.
pub fn print_success(
    selection: &Selection,
    registry: &Registry,
    report: &Report,
    git: InitOutcome,
    json: bool,
) {
    if json {
        emit_json_ok(&selection_json(
            selection,
            registry,
            json!({ "generated": true, "git": git.as_str(), "dependencies": report }),
        ));
        return;
    }

    println!();
    print_panel(
        &selection.project_name,
        Some(theme::badge_ok("CREATED")),
        &summary_rows(selection, registry),
    );
    // Repeated after generation: a one-shot run never showed the summary.
    print_experimental_warning(selection, registry);
    // The git outcome is reported only in JSON; a new repo needs no note.
    print_dep_advice(report);

    println!();
    print_rule("Next steps");
    println!(
        "    {} {}",
        theme::accent("→"),
        theme::command(format!("cd {}", selection.project_name))
    );
    if !report.all_required_present {
        println!(
            "    {}",
            theme::dim("# install the missing dependencies listed above, then:")
        );
    }
    println!(
        "    {} {}",
        theme::accent("→"),
        theme::command("just build")
    );
    println!();
}

/// The first sentence of a tool description (or its first 80 chars), for
/// prompts.
pub fn first_sentence(desc: &str) -> &str {
    if let Some(pos) = desc.find(". ") {
        &desc[..=pos]
    } else if let Some(pos) = desc.find(".\n") {
        &desc[..=pos]
    } else if desc.ends_with('.') {
        desc
    } else {
        let end = desc.char_indices().nth(80).map_or(desc.len(), |(i, _)| i);
        &desc[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::types::{Network, RoleAssignment};

    fn selection_with(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "demo".to_string(),
            assignments,
            network: Network::Preview,
            nix: false,
            spec: true,
        }
    }

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

    #[test]
    fn experimental_tag_only_for_experimental_tools() {
        let reg = Registry::load().unwrap();
        assert_eq!(
            experimental_tag(reg.get("blaster").unwrap()),
            " [experimental]"
        );
        assert_eq!(experimental_tag(reg.get("aiken").unwrap()), "");
    }

    #[test]
    fn components_json_marks_experimental_component() {
        let reg = Registry::load().unwrap();
        let sel = selection_with(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "aiken".to_string(),
            },
            RoleAssignment {
                role: Role::FormalMethods,
                tool_id: "blaster".to_string(),
            },
        ]);
        let json = components_json(&sel, &reg);
        let items = json.as_array().unwrap();
        let aiken = items.iter().find(|c| c["tool"] == "aiken").unwrap();
        let blaster = items.iter().find(|c| c["tool"] == "blaster").unwrap();
        assert_eq!(aiken["experimental"], json!(false));
        assert_eq!(blaster["experimental"], json!(true));
    }

    #[test]
    fn components_fold_fullstack_pair_into_protocol() {
        let reg = Registry::load().unwrap();
        let sel = selection_with(vec![
            RoleAssignment {
                role: Role::OnChain,
                tool_id: "scalus".to_string(),
            },
            RoleAssignment {
                role: Role::OffChain,
                tool_id: "scalus".to_string(),
            },
        ]);
        let json = components_json(&sel, &reg);
        assert_eq!(
            json,
            json!([{ "role": "protocol", "tool": "scalus", "experimental": false }])
        );
    }

    #[test]
    fn selected_experimental_tools_dedups_and_filters() {
        let reg = Registry::load().unwrap();
        let none = selection_with(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".to_string(),
        }]);
        assert!(selected_experimental_tools(&none, &reg).is_empty());

        let some = selection_with(vec![RoleAssignment {
            role: Role::FormalMethods,
            tool_id: "blaster".to_string(),
        }]);
        let experimental = selected_experimental_tools(&some, &reg);
        assert_eq!(experimental.len(), 1);
        assert_eq!(experimental[0].id, "blaster");
    }
}
