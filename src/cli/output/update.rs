//! Presenters for the update commands (`add` / `remove`): the change set as a
//! preview or as an applied result.

use std::collections::BTreeMap;

use serde_json::json;

use super::doctor::print_dep_advice;
use super::layout::{print_panel, print_rule};
use super::{emit_json_ok, print_experimental_warning, selection_json};
use crate::cli::theme;
use crate::doctor::Report;
use crate::registry::loader::Registry;
use crate::registry::types::Selection;
use crate::scaffold::update::{self, SlotOp, UpdatePlan};

/// Preview an update (`--dry-run`, or before confirming on a dirty tree): the
/// change set, nothing written.
pub fn print_update_plan(
    old: &Selection,
    new: &Selection,
    plan: &UpdatePlan,
    registry: &Registry,
    json: bool,
) {
    if json {
        emit_json_ok(&json!({
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
        &change_rows(old, new, plan, registry, false),
    );
    println!();
    println!(
        "  {}",
        theme::dim("Re-run without --dry-run to apply; review the result with `git diff`.")
    );
    println!();
}

/// Report a completed update, with dependency advice for the new selection (an
/// added tool may need new deps).
pub fn print_update_success(
    old: &Selection,
    selection: &Selection,
    registry: &Registry,
    plan: &UpdatePlan,
    report: &Report,
    json: bool,
) {
    if json {
        emit_json_ok(&selection_json(
            selection,
            registry,
            json!({
                "applied": true,
                "changes": changes_json(plan),
                "dependencies": report,
            }),
        ));
        return;
    }

    println!();
    print_panel(
        &selection.project_name,
        Some(theme::badge_ok("UPDATED")),
        &change_rows(old, selection, plan, registry, true),
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

/// The change set as JSON: affected directories bucketed by operation.
fn changes_json(plan: &UpdatePlan) -> serde_json::Value {
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

/// The styled `+ dir/  add <tool>` rows of the change panel. `applied` selects
/// the past ("add") over the preview ("would add") wording.
fn change_rows(
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
    let names = |map: &BTreeMap<String, Vec<String>>, dir: &str| {
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

/// Display names for tool ids, falling back to the id for unknown tools.
fn tool_names(ids: &[String], registry: &Registry) -> Vec<String> {
    ids.iter()
        .map(|id| {
            registry
                .get(id)
                .map_or_else(|| id.clone(), |t| t.name.clone())
        })
        .collect()
}

/// An infra re-render as the provider-set delta, e.g. `+Ogmios, -Kupo`.
fn infra_detail(from: &[String], to: &[String]) -> String {
    let added = to
        .iter()
        .filter(|t| !from.contains(t))
        .map(|t| format!("+{t}"));
    let removed = from
        .iter()
        .filter(|f| !to.contains(f))
        .map(|f| format!("-{f}"));
    added.chain(removed).collect::<Vec<_>>().join(", ")
}
