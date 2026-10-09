use dialoguer::theme::ColorfulTheme;
use dialoguer::{Confirm, Input, MultiSelect, Select};

use super::oneshot::validate_project_name;
use super::{CliError, output, theme};
use crate::registry::loader::Registry;
use crate::registry::types::{Network, Role, RoleAssignment, Selection};

/// Run the full interactive wizard, returning a validated Selection.
///
/// `allow_experimental` pre-acknowledges experimental tools (the CLI's
/// `--allow-experimental`): when `false`, choosing an experimental tool triggers
/// an explicit confirm before it's kept (the interactive counterpart of the
/// one-shot gate).
pub fn run_interactive(
    registry: &Registry,
    allow_experimental: bool,
    ignore_warning: bool,
) -> Result<Selection, CliError> {
    let theme = ColorfulTheme::default();

    // Step 1: Welcome
    output::print_welcome();

    // Step 2: Tool selection — prompt one tool per role (default "(skip)"). No
    // separate role-picking step: skipping a role is choosing "(skip)" for it.
    // Choosing the same fullstack-capable tool for both on-chain and off-chain
    // collapses into a single `protocol/` component automatically (planner).
    let mut assignments = select_tools(&theme, registry, ignore_warning)?;
    if assignments.is_empty() {
        return Err(CliError::NoRolesSelected);
    }

    // Experimental gate: unless pre-acknowledged with --allow-experimental,
    // require an explicit confirm before keeping any experimental tool. Declining
    // drops those assignments rather than aborting the whole wizard.
    if !allow_experimental {
        assignments = confirm_experimental(&theme, assignments, registry)?;
        if assignments.is_empty() {
            return Err(CliError::NoRolesSelected);
        }
    }

    // Step 3: Options
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

    // Step 4: Summary + confirmation
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

/// Prompt a tool for every role in canonical order. Non-infra roles use a
/// single-choice `Select` whose default is a leading **"(skip)"** entry, so a
/// role is left empty just by pressing enter; infrastructure (multi-tool) uses a
/// `MultiSelect` where selecting nothing means skip. There is no separate
/// role-picking step, and no fullstack prompt — assigning the same
/// fullstack-capable tool to on-chain and off-chain collapses into `protocol/`
/// downstream (TECH_SPEC §3.4).
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
            let items: Vec<String> = tools
                .iter()
                .map(|t| {
                    format!(
                        "{}{} — {}",
                        t.name,
                        output::experimental_tag(t),
                        output::first_sentence(&t.description)
                    )
                })
                .collect();
            let selections = MultiSelect::with_theme(theme)
                .with_prompt(format!(
                    "Choose tools for {} (space to select, enter to confirm — none to skip):",
                    role
                ))
                .items(&items)
                .interact()?;

            for &idx in &selections {
                assignments.push(RoleAssignment {
                    role,
                    tool_id: tools[idx].id.clone(),
                });
            }
        } else {
            // A tool may be unavailable given earlier picks (e.g. a devnet that
            // can't talk to the already-chosen off-chain tool). Compute the
            // reason per tool so we can show it as clearly non-selectable —
            // unless --ignore-warning lets the user pick it anyway.
            let disabled: Vec<Option<String>> = tools
                .iter()
                .map(|t| disabled_reason(role, t, &assignments, registry, ignore_warning))
                .collect();

            // Leading "(skip)" is the default; real tools follow at index 1..
            let mut items = vec!["(skip)".to_string()];
            items.extend(tools.iter().enumerate().map(|(i, t)| {
                let base = format!(
                    "{}{} — {}",
                    t.name,
                    output::experimental_tag(t),
                    output::first_sentence(&t.description)
                );
                match &disabled[i] {
                    Some(reason) => format!("{base}  ✗ unavailable — {reason}"),
                    None => base,
                }
            }));

            // Re-prompt if a disabled tool is chosen: an unavailable option is
            // shown (with its reason) but not selectable.
            loop {
                let idx = Select::with_theme(theme)
                    .with_prompt(format!("Choose a tool for {}:", role))
                    .items(&items)
                    .default(0)
                    .interact()?;

                if idx == 0 {
                    break; // (skip)
                }
                if let Some(reason) = &disabled[idx - 1] {
                    println!(
                        "  {}",
                        theme::warn(format!(
                            "{} can't be selected here — {reason}",
                            tools[idx - 1].name
                        ))
                    );
                    continue;
                }
                assignments.push(RoleAssignment {
                    role,
                    tool_id: tools[idx - 1].id.clone(),
                });
                break;
            }
        }
    }

    Ok(assignments)
}

/// Why `candidate` can't be selected for `role` given the tools already chosen,
/// or `None` if it's selectable. The cross-role constraint is off-chain <->
/// provider seam compatibility; we apply it to the devnet prompt (infra is
/// multi-select with union semantics — an individual pick isn't sound to
/// disable, so the run-level gate covers it). `--ignore-warning` clears it (the
/// option stays selectable, and the run-level gate downgrades to a warning).
fn disabled_reason(
    role: Role,
    candidate: &crate::registry::types::ToolDef,
    chosen: &[RoleAssignment],
    registry: &Registry,
    ignore_warning: bool,
) -> Option<String> {
    if ignore_warning || role != Role::Devnet {
        return None;
    }
    // Would choosing this devnet (on top of the off-chain + infra already
    // picked) make the selection incompatible?
    let mut hypothetical = chosen.to_vec();
    hypothetical.push(RoleAssignment {
        role,
        tool_id: candidate.id.clone(),
    });
    crate::registry::compat::check(&hypothetical, registry).map(|inc| inc.reason)
}

/// Gate experimental tools behind an explicit confirm (default No). Returns the
/// assignments to keep: if the user declines, the experimental ones are dropped
/// (with a note) and the rest proceed. Non-experimental selections pass through
/// untouched. This mirrors the one-shot `--allow-experimental` requirement.
fn confirm_experimental(
    theme: &ColorfulTheme,
    assignments: Vec<RoleAssignment>,
    registry: &Registry,
) -> Result<Vec<RoleAssignment>, CliError> {
    let experimental_names: Vec<&str> = assignments
        .iter()
        .filter_map(|a| registry.get(&a.tool_id))
        .filter(|t| t.experimental)
        .map(|t| t.name.as_str())
        .collect();

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

    // Declined: drop every experimental assignment, keep the rest.
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
        .validate_with(|input: &String| -> Result<(), String> {
            validate_project_name(input).map_err(|e| e.to_string())
        })
        .interact_text()?;
    Ok(name)
}
