//! The guided interactive wizard (dialoguer): one prompt per role, then the
//! project options and a final confirm.

use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, MultiSelect, Select};

use super::oneshot::validate_project_name;
use super::{CliError, output, theme};
use crate::registry::loader::Registry;
use crate::registry::types::{Network, Role, RoleAssignment, Selection, ToolDef};

/// Run the interactive wizard, returning a confirmed `Selection`.
///
/// Unless `allow_experimental`, keeping an experimental tool needs an explicit
/// confirm (the counterpart of the one-shot gate). `ignore_warning` makes
/// incompatible devnets selectable.
///
/// # Errors
/// [`CliError::NoRolesSelected`] if every role is skipped,
/// [`CliError::Aborted`] if the final confirm is declined, or a prompt error.
pub fn run_interactive(
    registry: &Registry,
    allow_experimental: bool,
    ignore_warning: bool,
) -> Result<Selection, CliError> {
    let theme = ColorfulTheme::default();

    output::print_welcome();

    let mut assignments = select_tools(&theme, registry, ignore_warning)?;
    if assignments.is_empty() {
        return Err(CliError::NoRolesSelected);
    }

    if !allow_experimental {
        assignments = confirm_experimental(&theme, assignments, registry)?;
        if assignments.is_empty() {
            return Err(CliError::NoRolesSelected);
        }
    }

    let project_name = prompt_project_name(&theme)?;
    let nix = Confirm::with_theme(&theme)
        .with_prompt("Set up Nix for dependency management?")
        .default(false)
        .interact()?;
    let spec = Confirm::with_theme(&theme)
        .with_prompt("Add a spec/ folder for the protocol specification?")
        .default(true)
        .interact()?;
    // Always preview; switch networks by editing CARDANO_NETWORK in the generated .env.
    let selection = Selection {
        project_name,
        assignments,
        network: Network::Preview,
        nix,
        spec,
    };

    output::print_summary(&selection, registry);

    let confirmed = Confirm::with_theme(&theme)
        .with_prompt("Generate project?")
        .default(true)
        .interact()?;
    if !confirmed {
        return Err(CliError::Aborted);
    }

    Ok(selection)
}

/// Prompt a tool for every role in canonical order; skipping a role is a valid
/// answer. Picking the same fullstack-capable tool for on-chain and off-chain
/// collapses into `protocol/` downstream (tech spec §3.4).
fn select_tools(
    theme: &ColorfulTheme,
    registry: &Registry,
    ignore_warning: bool,
) -> Result<Vec<RoleAssignment>, CliError> {
    let mut assignments = Vec::new();

    for &role in Role::ALL {
        let tools = registry.tools_for_role(role);
        if tools.is_empty() {
            continue;
        }

        if role == Role::Infrastructure {
            for tool in select_infra(theme, &tools)? {
                assignments.push(RoleAssignment {
                    role,
                    tool_id: tool.id.clone(),
                });
            }
        } else if let Some(tool) =
            select_single(theme, role, &tools, &assignments, registry, ignore_warning)?
        {
            assignments.push(RoleAssignment {
                role,
                tool_id: tool.id.clone(),
            });
        }
    }

    Ok(assignments)
}

/// Infrastructure accepts several tools: a multi-select where none means skip.
fn select_infra<'a>(
    theme: &ColorfulTheme,
    tools: &[&'a ToolDef],
) -> Result<Vec<&'a ToolDef>, CliError> {
    let items: Vec<String> = tools.iter().map(|t| prompt_label(t)).collect();
    let picked = MultiSelect::with_theme(theme)
        .with_prompt(format!(
            "Choose tools for {} (space to select, enter to confirm — none to skip):",
            Role::Infrastructure
        ))
        .items(&items)
        .interact()?;
    Ok(picked.into_iter().map(|i| tools[i]).collect())
}

/// A single-tool role: a select whose default leading "(skip)" entry leaves the
/// role empty. Tools incompatible with earlier picks are shown with the reason
/// but re-prompt when chosen.
fn select_single<'a>(
    theme: &ColorfulTheme,
    role: Role,
    tools: &[&'a ToolDef],
    chosen: &[RoleAssignment],
    registry: &Registry,
    ignore_warning: bool,
) -> Result<Option<&'a ToolDef>, CliError> {
    let disabled: Vec<Option<String>> = tools
        .iter()
        .map(|t| disabled_reason(role, t, chosen, registry, ignore_warning))
        .collect();

    let mut items = vec!["(skip)".to_string()];
    items.extend(tools.iter().zip(&disabled).map(|(t, reason)| {
        let label = prompt_label(t);
        match reason {
            Some(reason) => format!("{label}  ✗ unavailable — {reason}"),
            None => label,
        }
    }));

    loop {
        let idx = Select::with_theme(theme)
            .with_prompt(format!("Choose a tool for {role}:"))
            .items(&items)
            .default(0)
            .interact()?;
        let Some(tool_idx) = idx.checked_sub(1) else {
            return Ok(None);
        };
        match &disabled[tool_idx] {
            Some(reason) => println!(
                "  {}",
                theme::warn(format!(
                    "{} can't be selected here — {reason}",
                    tools[tool_idx].name
                ))
            ),
            None => return Ok(Some(tools[tool_idx])),
        }
    }
}

/// `Name [experimental] — first sentence of the description`.
fn prompt_label(tool: &ToolDef) -> String {
    format!(
        "{}{} — {}",
        tool.name,
        output::experimental_tag(tool),
        output::first_sentence(&tool.description)
    )
}

/// Why `candidate` can't be picked for `role` given the tools already chosen,
/// or `None` if it can. Only devnets are filtered: infra is a multi-select
/// with union semantics, so disabling a single pick isn't sound and the
/// run-level compat gate covers it instead.
fn disabled_reason(
    role: Role,
    candidate: &ToolDef,
    chosen: &[RoleAssignment],
    registry: &Registry,
    ignore_warning: bool,
) -> Option<String> {
    if ignore_warning || role != Role::Devnet {
        return None;
    }
    let mut hypothetical = chosen.to_vec();
    hypothetical.push(RoleAssignment {
        role,
        tool_id: candidate.id.clone(),
    });
    crate::registry::compat::check(&hypothetical, registry).map(|inc| inc.reason)
}

/// Confirm (default No) before keeping experimental tools. Declining drops
/// just those assignments; the rest pass through.
fn confirm_experimental(
    theme: &ColorfulTheme,
    assignments: Vec<RoleAssignment>,
    registry: &Registry,
) -> Result<Vec<RoleAssignment>, CliError> {
    let mut experimental_names: Vec<&str> = Vec::new();
    for tool in assignments.iter().filter_map(|a| registry.get(&a.tool_id)) {
        if tool.experimental && !experimental_names.contains(&tool.name.as_str()) {
            experimental_names.push(&tool.name);
        }
    }

    if experimental_names.is_empty() {
        return Ok(assignments);
    }

    println!();
    println!(
        "  {} {} {} {}",
        theme::warn_mark(),
        theme::warn_strong("Experimental:"),
        theme::warn_strong(experimental_names.join(", ")),
        theme::warn("— may be unstable or incomplete (rough edges, breaking changes).")
    );

    let include = Confirm::with_theme(theme)
        .with_prompt("Include this experimental tool anyway?")
        .default(false)
        .interact()?;
    if include {
        return Ok(assignments);
    }

    println!(
        "  {}",
        theme::dim(format!(
            "Skipping experimental: {}",
            experimental_names.join(", ")
        ))
    );
    Ok(assignments
        .into_iter()
        .filter(|a| !registry.get(&a.tool_id).is_some_and(|t| t.experimental))
        .collect())
}

fn prompt_project_name(theme: &ColorfulTheme) -> Result<String, CliError> {
    let name: String = Input::with_theme(theme)
        .with_prompt("Project name")
        .default("my-protocol".to_string())
        .validate_with(|input: &String| validate_project_name(input).map_err(|e| e.to_string()))
        .interact_text()?;
    Ok(name)
}
