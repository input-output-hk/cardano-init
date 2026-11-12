//! The guided interactive wizard (dialoguer): one prompt per role, then the
//! project options and a final confirm.

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use dialoguer::theme::{ColorfulTheme, Theme};
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
    let theme = WizardTheme::default();

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
    theme: &WizardTheme,
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
    theme: &WizardTheme,
    tools: &[&'a ToolDef],
) -> Result<Vec<&'a ToolDef>, CliError> {
    let width = item_width();
    let items: Vec<String> = tools
        .iter()
        .map(|t| theme.register(prompt_label(t).fit(width)))
        .collect();
    let picked = MultiSelect::with_theme(theme)
        .with_prompt(format!(
            "Choose tools for {} (space toggles, enter confirms):",
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
    theme: &WizardTheme,
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

    let width = item_width();
    let mut items = vec!["(skip)".to_string()];
    items.extend(tools.iter().zip(&disabled).map(|(t, reason)| {
        let mut label = prompt_label(t);
        if let Some(reason) = reason {
            label.push_styled(format!("  ✗ unavailable — {reason}"), |s| {
                theme::warn(s).to_string()
            });
        }
        theme.register(label.fit(width))
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

/// `Name [experimental] — first sentence of the description`, with the icon in
/// its brand colour and the experimental tag in yellow.
fn prompt_label(tool: &ToolDef) -> Label {
    let mut label = Label::default();
    if theme::icons_enabled() {
        match theme::tool_icon(tool) {
            Some(icon) => {
                let glyph = console::strip_ansi_codes(&icon.to_string()).into_owned();
                label.push_styled(format!("{glyph} "), |_| format!("{icon} "));
            }
            None => label.push("  "),
        }
    }
    label.push(&tool.name);
    let tag = output::experimental_tag(tool);
    if !tag.is_empty() {
        label.push_styled(tag, |s| theme::warn(s).to_string());
    }
    label.push(format!(" — {}", output::first_sentence(&tool.description)));
    label
}

/// A prompt item built from segments. dialoguer only ever sees the plain
/// text (see [`Label::fit`]); [`WizardTheme`] maps it back to the segments to
/// draw the colours.
#[derive(Clone, Default)]
struct Label(Vec<Segment>);

#[derive(Clone)]
struct Segment {
    plain: String,
    /// Pre-rendered colours, or `None` to take the row's (active/inactive)
    /// style. Own-coloured segments are drawn apart from the row style, so
    /// their ANSI reset can't cancel the active-row highlight.
    styled: Option<String>,
}

impl Label {
    fn push(&mut self, text: impl Into<String>) {
        self.0.push(Segment {
            plain: text.into(),
            styled: None,
        });
    }

    fn push_styled(&mut self, text: impl Into<String>, paint: impl FnOnce(&str) -> String) {
        let plain = text.into();
        let styled = paint(&plain);
        self.0.push(Segment {
            plain,
            styled: Some(styled),
        });
    }

    fn plain(&self) -> String {
        self.0.iter().map(|s| s.plain.as_str()).collect()
    }

    /// Truncate (with `…`) so the plain text fits in `max_bytes`.
    ///
    /// dialoguer (0.11 and 0.12) redraws a list by guessing each item's
    /// wrapped height from its *byte* length, not its display width; an item
    /// longer in bytes than the terminal is wide makes it erase extra lines
    /// above the prompt on every keypress. Capping the byte length of the
    /// plain text it sees (which bounds the display width too) keeps every
    /// item on one row and the redraw exact; colours are added by the theme,
    /// outside dialoguer's count.
    fn fit(self, max_bytes: usize) -> Self {
        if self.plain().len() <= max_bytes {
            return self;
        }
        let mut budget = max_bytes.saturating_sub('…'.len_utf8());
        let mut out = Label::default();
        for seg in self.0 {
            if seg.plain.len() <= budget {
                budget -= seg.plain.len();
                out.0.push(seg);
                continue;
            }
            // Cut on a char boundary; a partial segment loses its own colour.
            let end = seg
                .plain
                .char_indices()
                .map(|(i, c)| i + c.len_utf8())
                .take_while(|&end| end <= budget)
                .last()
                .unwrap_or(0);
            out.push(seg.plain[..end].trim_end());
            break;
        }
        out.push("…");
        out
    }
}

/// Columns the theme draws before an item: `❯ ✔ ` in a multi-select (the
/// widest case; a select uses `❯ `).
const ITEM_PREFIX_WIDTH: usize = 4;

/// The byte budget for one select/multi-select item on the current terminal.
fn item_width() -> usize {
    let cols = usize::from(console::Term::stderr().size().1);
    cols.saturating_sub(ITEM_PREFIX_WIDTH).max(20)
}

/// [`ColorfulTheme`] plus a `❯` cursor on the active multi-select row (which
/// the stock theme marks only by text colour) and coloured [`Label`] items.
#[derive(Default)]
struct WizardTheme {
    base: ColorfulTheme,
    /// Plain item text (what dialoguer holds) → its coloured segments.
    labels: RefCell<HashMap<String, Label>>,
}

impl WizardTheme {
    /// Remember `label`'s colours and return the plain text to hand dialoguer.
    fn register(&self, label: Label) -> String {
        let plain = label.plain();
        self.labels.borrow_mut().insert(plain.clone(), label);
        plain
    }

    /// Draw `text` in the row style, swapping in a registered label's own
    /// colours segment by segment.
    fn write_item(&self, f: &mut dyn fmt::Write, text: &str, active: bool) -> fmt::Result {
        let row = if active {
            &self.base.active_item_style
        } else {
            &self.base.inactive_item_style
        };
        let labels = self.labels.borrow();
        let Some(label) = labels.get(text) else {
            return write!(f, "{}", row.apply_to(text));
        };
        for seg in &label.0 {
            match &seg.styled {
                Some(styled) => write!(f, "{styled}")?,
                None => write!(f, "{}", row.apply_to(&seg.plain))?,
            }
        }
        Ok(())
    }
}

impl Theme for WizardTheme {
    fn format_prompt(&self, f: &mut dyn fmt::Write, prompt: &str) -> fmt::Result {
        self.base.format_prompt(f, prompt)
    }

    fn format_error(&self, f: &mut dyn fmt::Write, err: &str) -> fmt::Result {
        self.base.format_error(f, err)
    }

    fn format_confirm_prompt(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        default: Option<bool>,
    ) -> fmt::Result {
        self.base.format_confirm_prompt(f, prompt, default)
    }

    fn format_confirm_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        selection: Option<bool>,
    ) -> fmt::Result {
        self.base
            .format_confirm_prompt_selection(f, prompt, selection)
    }

    fn format_input_prompt(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        default: Option<&str>,
    ) -> fmt::Result {
        self.base.format_input_prompt(f, prompt, default)
    }

    fn format_input_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        sel: &str,
    ) -> fmt::Result {
        self.base.format_input_prompt_selection(f, prompt, sel)
    }

    fn format_select_prompt(&self, f: &mut dyn fmt::Write, prompt: &str) -> fmt::Result {
        self.base.format_select_prompt(f, prompt)
    }

    fn format_select_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        sel: &str,
    ) -> fmt::Result {
        self.base.format_select_prompt_selection(f, prompt, sel)
    }

    fn format_multi_select_prompt(&self, f: &mut dyn fmt::Write, prompt: &str) -> fmt::Result {
        self.base.format_multi_select_prompt(f, prompt)
    }

    fn format_multi_select_prompt_selection(
        &self,
        f: &mut dyn fmt::Write,
        prompt: &str,
        selections: &[&str],
    ) -> fmt::Result {
        self.base
            .format_multi_select_prompt_selection(f, prompt, selections)
    }

    fn format_select_prompt_item(
        &self,
        f: &mut dyn fmt::Write,
        text: &str,
        active: bool,
    ) -> fmt::Result {
        let prefix = if active {
            &self.base.active_item_prefix
        } else {
            &self.base.inactive_item_prefix
        };
        write!(f, "{prefix} ")?;
        self.write_item(f, text, active)
    }

    fn format_multi_select_prompt_item(
        &self,
        f: &mut dyn fmt::Write,
        text: &str,
        checked: bool,
        active: bool,
    ) -> fmt::Result {
        let cursor = if active {
            &self.base.active_item_prefix
        } else {
            &self.base.inactive_item_prefix
        };
        let check = if checked {
            &self.base.checked_item_prefix
        } else {
            &self.base.unchecked_item_prefix
        };
        write!(f, "{cursor} {check} ")?;
        self.write_item(f, text, active)
    }
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
    theme: &WizardTheme,
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

fn prompt_project_name(theme: &WizardTheme) -> Result<String, CliError> {
    let name: String = Input::with_theme(theme)
        .with_prompt("Project name")
        .default("my-protocol".to_string())
        .validate_with(|input: &String| validate_project_name(input).map_err(|e| e.to_string()))
        .interact_text()?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::Label;

    fn label() -> Label {
        let mut l = Label::default();
        l.push_styled("* ", |s| format!("<{s}>"));
        l.push("Pebble");
        l.push_styled(" [experimental]", |s| format!("<{s}>"));
        l.push(" — a long description that will not fit");
        l
    }

    #[test]
    fn fit_keeps_short_labels() {
        let l = label().fit(200);
        assert_eq!(l.plain(), label().plain());
    }

    #[test]
    fn fit_caps_byte_length_on_a_char_boundary() {
        for max in 10..60 {
            let plain = label().fit(max).plain();
            assert!(plain.len() <= max, "{plain:?} is {} bytes", plain.len());
            assert!(plain.ends_with('…'));
        }
    }

    #[test]
    fn fit_keeps_whole_styled_segments() {
        let l = label().fit(30);
        assert_eq!(l.0[2].styled.as_deref(), Some("< [experimental]>"));
    }
}
