use serde_json::json;

use super::theme;
use super::{CliError, Format};
use crate::doctor::Report;
use crate::doctor::installers::Installer;
use crate::doctor::probe::{Environment, ScanResult};
use crate::registry::loader::Registry;
use crate::registry::types::{Role, Selection, ToolDef};
use crate::scaffold::planner::FilePlan;
use crate::scaffold::update::{self, SlotOp, UpdatePlan};

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

/// A borderless table (no rules, no outer frame) with content-fit columns —
/// used for the aligned `doctor`/`list` sections. Coloring stays in the cells'
/// `console`-styled strings; the `custom_styling` feature measures their width
/// correctly so columns line up regardless of ANSI codes.
fn borderless_table() -> comfy_table::Table {
    use comfy_table::{ContentArrangement, Table, presets};
    let mut table = Table::new();
    table.load_preset(presets::NOTHING);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table
}

/// Print a table as a left-indented block, matching the two-space offset of the
/// surrounding human output. Left cell-padding is zeroed (right padding of 2
/// separates columns) so the first column sits flush under [`theme::PAD`], and
/// each row is re-prefixed with it since `comfy-table` renders from column zero.
fn print_table(mut table: comfy_table::Table, indent: &str) {
    for column in table.column_iter_mut() {
        column.set_padding((0, 2));
    }
    for line in table.to_string().lines() {
        println!("{}{}", indent, line.trim_end());
    }
}

/// Print a rounded, cyan-framed panel wrapping the pre-styled `rows`, with the
/// `title` centered in the top border. An optional `badge` is printed on its own
/// line directly above the frame, flush-left at [`theme::PAD`] — so a scaffold
/// result's ` CREATED ` pill lines up with the ` DEPS OK ` pill further down.
/// Widths are measured with ANSI stripped (`measure_text_width`) so colored
/// content still aligns inside the frame.
fn print_panel(title: &str, badge: Option<console::StyledObject<String>>, rows: &[String]) {
    for line in panel_lines(title, badge, rows) {
        println!("{line}");
    }
}

/// Build the styled, `PAD`-indented lines of a panel (see [`print_panel`]).
/// Returned rather than printed so callers can route them to stdout (normal
/// output) or stderr (the error presenter).
fn panel_lines(
    title: &str,
    badge: Option<console::StyledObject<String>>,
    rows: &[String],
) -> Vec<String> {
    let title_w = console::measure_text_width(title);
    let interior = rows
        .iter()
        .map(|r| console::measure_text_width(r))
        .chain(std::iter::once(title_w))
        .max()
        .unwrap_or(0);
    // Run length between the two corner glyphs: 2 left margin + interior + 1 right.
    let span = interior + 3;

    let mut out = Vec::new();
    if let Some(badge) = badge {
        out.push(format!("{}{}", theme::PAD, badge));
    }

    // ╭──── <title centered> ────╮ — dashes fill each side of " title ".
    let pad_total = span - 2 - title_w; // dashes shared between the two sides
    let left = pad_total / 2;
    let right = pad_total - left;
    out.push(format!(
        "{}{}{} {} {}{}",
        theme::PAD,
        theme::accent("╭"),
        theme::accent("─".repeat(left)),
        theme::accent_strong(title),
        theme::accent("─".repeat(right)),
        theme::accent("╮"),
    ));
    // │  <row padded to interior>  │
    for row in rows {
        let gap = interior - console::measure_text_width(row);
        out.push(format!(
            "{}{}  {}{} {}",
            theme::PAD,
            theme::accent("│"),
            row,
            " ".repeat(gap),
            theme::accent("│"),
        ));
    }
    // ╰──────────────╯
    out.push(format!(
        "{}{}",
        theme::PAD,
        theme::accent(format!("╰{}╯", "─".repeat(span))),
    ));
    out
}

/// Print a section label followed by a horizontal rule — the lighter framing
/// element that pairs with [`print_panel`] (used for section headings and the
/// "Next steps" block).
fn print_rule(label: &str) {
    const WIDTH: usize = 44;
    let dashes = WIDTH.saturating_sub(console::measure_text_width(label) + 1);
    println!(
        "{}{} {}",
        theme::PAD,
        theme::strong(label),
        theme::dim("─".repeat(dashes)),
    );
}

/// Render any error in the requested format (always to stderr).
///
/// In `json`, an error envelope `{ ok: false, error: { code, message, context } }`.
/// In `human`, a red ` ERR (<code>) ` pill, the explanation, and — when the
/// error carries one — a framed `help` panel with the remedy. An interactive
/// abort (exit code 0) is a user choice, not an error, so it stays silent.
pub fn print_error(err: CliError, format: Format) {
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
                print_human_error(&err);
            }
        }
    }
}

/// Wrap `text` to `width` columns on word boundaries.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// Render a `CliError` to stderr: a red ` ERR (<code>) ` pill, the wrapped
/// explanation, then (if present) a framed `help` panel. The remedy comes from
/// the error's miette `Diagnostic::help`; the pill name is the stable `code()`.
fn print_human_error(err: &CliError) {
    eprintln!();

    // The explanation begins on the pill's line (like the DEPS pills), so it
    // reads as the error's description; wrapped continuation lines align under
    // where the text starts.
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

/// The inline tag appended after an experimental tool's name in list-style
/// output (`""` for released tools). Kept here so `list`, `--help`, and the
/// interactive prompts render the marker identically.
pub fn experimental_tag(tool: &ToolDef) -> &'static str {
    if tool.experimental {
        " [experimental]"
    } else {
        ""
    }
}

/// The distinct experimental tools in a selection, in role order and
/// de-duplicated (a fullstack tool assigned to both roles is reported once).
/// Shared by the generation-time warning and the `--allow-experimental` gate.
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

/// Warn — loudly — when the selection includes any experimental tool, so a user
/// never generates an unstable/incomplete tool without knowing it. Printed inside
/// the pre-generation summary and again on success. (The hard gate is
/// `--allow-experimental` / the interactive confirm; this is the reminder that
/// fires once the tool is allowed through.) Silent when nothing experimental is
/// selected.
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
/// anyway (the user passed `--ignore-warning`). The hard stop is the default;
/// this is the reminder that fires once the combination is forced through.
pub fn print_incompatibility_warning(inc: &crate::registry::compat::Incompatibility) {
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

/// The aligned `role  tool · lang` rows (plus `network` and, if set, `nix`) that
/// fill the selection panel. Shared by the pre-generation summary and the
/// post-generation success panel so they can't drift.
fn summary_rows(selection: &Selection, registry: &Registry) -> Vec<String> {
    let comps = components(selection, registry);

    // Column widths for the aligned rows (labels are the role kebabs plus the
    // trailing `network`/`nix` rows).
    let label_w = comps
        .iter()
        .map(|c| c.kebab.len())
        .chain([NETWORK_LABEL.len(), NIX_LABEL.len()])
        .max()
        .unwrap_or(0);
    let name_w = comps
        .iter()
        .map(|c| {
            registry
                .get(&c.tool_id)
                .map_or(c.tool_id.len(), |t| t.name.len())
        })
        .max()
        .unwrap_or(0);

    let mut rows: Vec<String> = Vec::new();
    for c in &comps {
        let tool = registry.get(&c.tool_id);
        let name = tool.map_or_else(|| c.tool_id.clone(), |t| t.name.clone());
        let lang = tool.and_then(|t| t.languages.first());
        let experimental = tool.is_some_and(|t| t.experimental);

        let name_cell = theme::accent(format!("{name:<name_w$}"));
        let lang_cell = lang.map_or_else(String::new, |l| theme::dim(format!("· {l}")).to_string());
        let tag = if experimental {
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
    rows
}

/// Print a summary of the selection *before* generation, as a framed panel
/// titled with the project name. Used for the interactive confirm step and the
/// dry-run plan (the one-shot generate path skips straight to the success
/// panel, which carries the same rows plus a `CREATED` badge).
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

/// Row labels for the non-component lines of the summary panel.
const NETWORK_LABEL: &str = "network";
const NIX_LABEL: &str = "nix";

/// A generated component as reported to the user: its role (kebab) and tool id.
struct Component {
    /// Role kebab, or `"protocol"` for a collapsed fullstack component.
    kebab: String,
    tool_id: String,
}

/// The components actually scaffolded, with the fullstack on-chain+off-chain pair
/// folded into a single `protocol` component (matching the generated tree). This
/// is what the summary and JSON report, so they reflect reality rather than the
/// two raw assignments.
fn components(selection: &Selection, registry: &Registry) -> Vec<Component> {
    let fullstack_id = crate::scaffold::planner::fullstack_tool_id(selection, registry);
    let mut out = Vec::new();
    for a in &selection.assignments {
        let is_fullstack_member = fullstack_id.as_deref() == Some(a.tool_id.as_str())
            && matches!(a.role, Role::OnChain | Role::OffChain);
        if is_fullstack_member {
            // Emit one `protocol` entry (on the on-chain half); skip the off-chain.
            if a.role == Role::OffChain {
                continue;
            }
            out.push(Component {
                kebab: crate::contract::DIR_PROTOCOL.to_string(),
                tool_id: a.tool_id.clone(),
            });
            continue;
        }
        out.push(Component {
            kebab: a.role.as_kebab().to_string(),
            tool_id: a.tool_id.clone(),
        });
    }
    out
}

/// Build the `[{ role, tool }]` component list for a selection (collapse-aware).
fn components_json(selection: &Selection, registry: &Registry) -> serde_json::Value {
    let items: Vec<serde_json::Value> = components(selection, registry)
        .iter()
        .map(|c| {
            // `experimental` is additive: it signals to agents that a generated
            // component is an experimental tool.
            let experimental = registry.get(&c.tool_id).is_some_and(|t| t.experimental);
            json!({ "role": c.kebab, "tool": c.tool_id, "experimental": experimental })
        })
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
            "components": components_json(selection, registry),
            "files": files,
        }));
        return;
    }

    print_summary(selection, registry);

    println!(
        "  {}",
        theme::strong(format!("{}/", selection.project_name))
    );

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
        theme::strong(plan.entries.len())
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
                theme::dim(connector),
                theme::dim(format!("{name}/"))
            );
        } else {
            println!("  {}{}{}", indent, theme::dim(connector), name);
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
pub fn print_success(
    selection: &Selection,
    registry: &Registry,
    report: &Report,
    git: crate::cli::git::InitOutcome,
    format: Format,
) {
    if format == Format::Json {
        emit_json_ok(json!({
            "project": selection.project_name,
            "network": selection.network.to_string(),
            "nix": selection.nix,
            "generated": true,
            "components": components_json(selection, registry),
            "git": git.as_str(),
            "dependencies": report,
        }));
        return;
    }

    // The result is the same selection panel as the pre-generation summary,
    // now carrying a CREATED badge in its header — so the project name and the
    // per-component list aren't repeated in a second block.
    println!();
    print_panel(
        &selection.project_name,
        Some(theme::badge_ok("CREATED")),
        &summary_rows(selection, registry),
    );

    // Reiterate the experimental caveat after generation (a one-shot user may
    // not have seen the pre-generation summary warning scroll past).
    print_experimental_warning(selection, registry);

    // The git repo is initialized silently: opening the project shows it's a
    // git repo, so a "git initialized" note would just be noise. The outcome is
    // still reported in the JSON output's `git` field for agents.

    // Check-and-advise: surface any missing required deps before "Next steps".
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

// ---------------------------------------------------------------------------
// Update (add / remove) presenters
// ---------------------------------------------------------------------------

/// Bucket an [`UpdatePlan`]'s slot ops into (create, replace, rerender, remove)
/// directory lists, plus the shared removals, as owned strings.
fn update_change_buckets(
    plan: &UpdatePlan,
) -> (Vec<String>, Vec<String>, Vec<String>, Vec<String>) {
    let (mut create, mut replace, mut rerender, mut remove) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for op in &plan.slot_ops {
        let dir = op.dir().to_string_lossy().into_owned();
        match op {
            SlotOp::Create(_) => create.push(dir),
            SlotOp::Replace(_) => replace.push(dir),
            SlotOp::RerenderInfra(_) => rerender.push(dir),
            SlotOp::Remove(_) => remove.push(dir),
        }
    }
    (create, replace, rerender, remove)
}

fn changes_json(plan: &UpdatePlan) -> serde_json::Value {
    let (create, replace, rerender, remove) = update_change_buckets(plan);
    let shared_removed: Vec<String> = plan
        .shared_removals
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    json!({
        "create": create,
        "replace": replace,
        "rerender": rerender,
        "remove": remove,
        "shared_removed": shared_removed,
    })
}

/// Map a slot's tool ids to their display names (falling back to the id when a
/// tool isn't in the registry), so the change panel names the tool, not just the
/// directory.
fn tool_names(ids: &[String], registry: &Registry) -> Vec<String> {
    ids.iter()
        .map(|id| {
            registry
                .get(id)
                .map_or_else(|| id.clone(), |t| t.name.clone())
        })
        .collect()
}

/// Describe an infra re-render as the provider-set delta (`+Ogmios`, `-Kupo`),
/// so a mixed add/remove of providers reads at a glance.
fn infra_detail(from: &[String], to: &[String]) -> String {
    let mut parts = Vec::new();
    for t in to.iter().filter(|t| !from.contains(t)) {
        parts.push(format!("+{t}"));
    }
    for f in from.iter().filter(|f| !to.contains(f)) {
        parts.push(format!("-{f}"));
    }
    parts.join(", ")
}

/// The styled `+ dir/ add <tool>` rows for the change panel, naming the tool that
/// changed in each slot. `applied` swaps "would X" (preview) for "X" (result).
fn update_change_rows(
    old: &Selection,
    new: &Selection,
    plan: &UpdatePlan,
    registry: &Registry,
    applied: bool,
) -> Vec<String> {
    let old_tools = update::slot_tools(old, registry);
    let new_tools = update::slot_tools(new, registry);
    let verb = |s: &str| {
        if applied {
            s.to_string()
        } else {
            format!("would {s}")
        }
    };
    let names = |map: &std::collections::BTreeMap<String, Vec<String>>, dir: &str| {
        map.get(dir)
            .map(|ids| tool_names(ids, registry))
            .unwrap_or_default()
    };

    let mut rows = Vec::new();
    for op in &plan.slot_ops {
        let dir = op.dir().to_string_lossy().into_owned();
        let from = names(&old_tools, &dir);
        let to = names(&new_tools, &dir);
        let (mark, detail) = match op {
            SlotOp::Create(_) => (
                theme::good("+"),
                format!("{} {}", verb("add"), to.join(", ")),
            ),
            SlotOp::Replace(_) => (
                theme::warn("~"),
                format!(
                    "{} {} → {}",
                    verb("replace"),
                    from.join(", "),
                    to.join(", ")
                ),
            ),
            SlotOp::RerenderInfra(_) => (
                theme::warn("~"),
                format!("{} {}", verb("update"), infra_detail(&from, &to)),
            ),
            SlotOp::Remove(_) => (
                theme::warn_strong("-"),
                format!("{} {}", verb("remove"), from.join(", ")),
            ),
        };
        rows.push(format!("{}  {}/  {}", mark, dir, theme::dim(detail)));
    }
    for f in &plan.shared_removals {
        rows.push(format!(
            "{}  {}  {}",
            theme::warn_strong("-"),
            f.to_string_lossy(),
            theme::dim(verb("remove"))
        ));
    }
    if rows.is_empty() {
        rows.push(theme::dim("no component changes").to_string());
    }
    rows.push(theme::dim("· top-level files re-wired to match").to_string());
    rows
}

/// Preview an update (`--dry-run`, or the confirm on a forced dirty tree): the
/// change set, nothing written.
pub fn print_update_plan(
    old: &Selection,
    new: &Selection,
    plan: &UpdatePlan,
    registry: &Registry,
    format: Format,
) {
    if format == Format::Json {
        emit_json_ok(json!({
            "project": new.project_name,
            "applied": false,
            "dry_run": true,
            "changes": changes_json(plan),
        }));
        return;
    }

    println!();
    print_panel(
        &new.project_name,
        Some(theme::badge_warn("PLAN")),
        &update_change_rows(old, new, plan, registry, false),
    );
    println!();
    println!(
        "  {}",
        theme::dim("Re-run without --dry-run to apply; review the result with `git diff`.")
    );
    println!();
}

/// Report a completed update, with the dependency check-and-advise for the new
/// selection (a newly-added tool may pull in new deps).
pub fn print_update_success(
    old: &Selection,
    selection: &Selection,
    registry: &Registry,
    plan: &UpdatePlan,
    report: &Report,
    format: Format,
) {
    if format == Format::Json {
        emit_json_ok(json!({
            "project": selection.project_name,
            "applied": true,
            "network": selection.network.to_string(),
            "nix": selection.nix,
            "components": components_json(selection, registry),
            "changes": changes_json(plan),
            "dependencies": report,
        }));
        return;
    }

    println!();
    print_panel(
        &selection.project_name,
        Some(theme::badge_ok("UPDATED")),
        &update_change_rows(old, selection, plan, registry, true),
    );
    print_experimental_warning(selection, registry);
    print_dep_advice(report);
    println!();
    print_rule("Next steps");
    println!(
        "    {} {}",
        theme::accent("→"),
        theme::dim("review the change with `git diff`")
    );
    println!(
        "    {} {}",
        theme::accent("→"),
        theme::command("just build")
    );
    println!();
}

/// Print the dependency advice block (missing required deps + install plans).
/// Silent when everything required is already present.
fn print_dep_advice(report: &Report) {
    if report.all_required_present {
        println!();
        println!(
            "  {}  all required dependencies present",
            theme::badge_ok("DEPS OK")
        );
        return;
    }

    println!();
    println!(
        "  {}  install the required dependencies below",
        theme::badge_warn("DEPS MISSING")
    );
    for dep in report.missing_required() {
        print_missing_dep(dep);
    }
}

/// Indent for the per-dependency detail (install methods + docs), one level
/// deeper than the `✘ <dep>` header.
const DEP_INDENT: &str = "      ";

/// Render one missing dependency: a header that names it and announces the
/// method list, then an aligned `command → status` table (the recommended path,
/// any other ready methods, then those that first need an installer this host
/// lacks), and the docs fallback.
fn print_missing_dep(dep: &crate::doctor::DepStatus) {
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
        // command → status, aligned into two columns. `Disabled` arrangement
        // keeps long commands (e.g. curl one-liners) on one line.
        let mut table = borderless_table();
        table.set_content_arrangement(comfy_table::ContentArrangement::Disabled);

        // The recommended path — a bootstrap chain is joined with `&&` so the
        // whole thing is one copy-pasteable line.
        if !dep.plan.is_empty() {
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

    // The dependency binaries already present on PATH. Surfaced as the panel's
    // `tooling` row (a missing one shows up in "Missing dependencies", not here)
    // and reused below to mark each component's readiness.
    let present: std::collections::HashSet<&str> = report
        .deps
        .iter()
        .filter(|d| d.present)
        .map(|d| d.id.as_str())
        .collect();

    // Environment panel: os, detected installers, and detected tooling.
    println!();
    let installer_list = if installers.is_empty() {
        "none detected".to_string()
    } else {
        installers.join(", ")
    };
    let mut present_list: Vec<&str> = present.iter().copied().collect();
    present_list.sort_unstable();
    let tooling = if present_list.is_empty() {
        "none detected".to_string()
    } else {
        present_list.join(", ")
    };
    let env_label_w = "installers".len();
    let env_rows = vec![
        format!(
            "{:<env_label_w$}  {}",
            "os",
            theme::accent(format!("{:?}", env.os))
        ),
        format!(
            "{:<env_label_w$}  {}",
            "installers",
            theme::accent(&installer_list)
        ),
        format!("{:<env_label_w$}  {}", "tooling", theme::accent(&tooling)),
    ];
    print_panel("environment", None, &env_rows);

    // Detected components. The mark reflects whether the tool's required
    // dependencies are present, not merely that the component was detected.

    println!();
    if scan.components.is_empty() && scan.unrecognized.is_empty() {
        println!(
            "  {}",
            theme::dim("No generated components detected in this directory.")
        );
    } else {
        let mut table = borderless_table();
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

    // Missing-dependency advice (present tooling is shown in the panel above).
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
    print_rule("Roles");
    println!();
    let mut roles_table = borderless_table();
    roles_table.set_header(vec![
        theme::dim("ROLE").to_string(),
        theme::dim("DESCRIPTION").to_string(),
        theme::dim("DIR").to_string(),
        theme::dim("").to_string(),
    ]);
    for role in view::role_views() {
        let multi = if role.multiple { "(multiple)" } else { "" };
        roles_table.add_row(vec![
            theme::accent(role.id).to_string(),
            role.display.to_string(),
            theme::dim(role.dir).to_string(),
            theme::dim(multi).to_string(),
        ]);
    }
    print_table(roles_table, theme::PAD);

    println!();
    print_rule("Tools");
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

    use crate::registry::types::{Network, RoleAssignment};

    fn selection_with(assignments: Vec<RoleAssignment>) -> Selection {
        Selection {
            project_name: "demo".to_string(),
            assignments,
            network: Network::Preview,
            nix: false,
        }
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
        assert_eq!(aiken["experimental"], serde_json::json!(false));
        assert_eq!(blaster["experimental"], serde_json::json!(true));
    }

    #[test]
    fn selected_experimental_tools_dedups_and_filters() {
        let reg = Registry::load().unwrap();
        // No experimental tool selected → empty.
        let none = selection_with(vec![RoleAssignment {
            role: Role::OnChain,
            tool_id: "aiken".to_string(),
        }]);
        assert!(selected_experimental_tools(&none, &reg).is_empty());

        // Blaster selected → reported once.
        let some = selection_with(vec![RoleAssignment {
            role: Role::FormalMethods,
            tool_id: "blaster".to_string(),
        }]);
        let experimental = selected_experimental_tools(&some, &reg);
        assert_eq!(experimental.len(), 1);
        assert_eq!(experimental[0].id, "blaster");
    }
}
