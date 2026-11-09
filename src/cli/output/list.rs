//! The `list` and `table` presenters: the role/tool catalog as text, a role
//! matrix, or JSON.

use std::fmt::Write;

use serde_json::json;

use super::emit_json_ok;
use super::layout::{print_rule, wrap};
use crate::cli::theme;
use crate::doctor::catalog::DepCatalog;
use crate::registry::loader::Registry;
use crate::registry::types::{Link, Role, ToolDef};
use crate::registry::view;

/// Print the registry for `cardano-init list`: the tech spec §8 payload with
/// `json`, else the full catalog.
pub fn print_list(registry: &Registry, catalog: &DepCatalog, json: bool) {
    if json {
        emit_list_json(registry, catalog);
        return;
    }

    // One section per role, in `Role::ALL` order, tools name-sorted. A tool
    // filling several roles is described under the first and pointed to from
    // the rest.
    let mut seen: Vec<&str> = Vec::new();
    for &role in Role::ALL {
        println!();
        print_role_rule(role);
        let mut tools = registry.tools_for_role(role);
        tools.sort_by(|a, b| a.name.cmp(&b.name));
        let mut block = String::new();
        for tool in tools {
            block.push('\n');
            if seen.contains(&tool.id.as_str()) {
                format_tool_reference(&mut block, tool);
            } else {
                format_tool(&mut block, tool, &tool_support_links(tool, catalog));
                seen.push(&tool.id);
            }
        }
        print!("{block}");
    }
    println!();
}

/// Print `cardano-init table`: a role × tool matrix, or with `json` the same
/// payload as `list --json` (it already carries the matrix's data).
pub fn print_table(registry: &Registry, catalog: &DepCatalog, json: bool) {
    if json {
        emit_list_json(registry, catalog);
    } else {
        print_tool_table(registry);
    }
}

/// The tech spec §8 `list` payload: roles, and tools with their support links.
fn emit_list_json(registry: &Registry, catalog: &DepCatalog) {
    #[derive(serde::Serialize)]
    struct ListTool {
        #[serde(flatten)]
        view: view::ToolView,
        support: Vec<Link>,
    }
    let tools: Vec<ListTool> = view::tool_views(registry)
        .into_iter()
        .map(|v| {
            let support = registry
                .get(&v.id)
                .map_or_else(Vec::new, |t| tool_support_links(t, catalog));
            ListTool { view: v, support }
        })
        .collect();
    emit_json_ok(&json!({
        "roles": view::role_views(),
        "tools": tools,
    }));
}

/// A role's section heading: `On-chain ────── on-chain/`, noting when the role
/// takes several tools.
fn print_role_rule(role: Role) {
    const WIDTH: usize = 60;
    let label = role.to_string();
    let note = if role.multiple() {
        format!("{}/ · pick any number", role.dir())
    } else {
        format!("{}/", role.dir())
    };
    let used = console::measure_text_width(&label) + console::measure_text_width(&note) + 2;
    println!(
        "{}{} {} {}",
        theme::PAD,
        theme::strong(&label),
        theme::dim("─".repeat(WIDTH.saturating_sub(used))),
        theme::dim(note),
    );
}

/// Support links for a tool's system dependencies, de-duplicated by URL.
fn tool_support_links(tool: &ToolDef, catalog: &DepCatalog) -> Vec<Link> {
    let mut links: Vec<Link> = Vec::new();
    for dep in &tool.system_deps {
        for link in catalog.get(dep).map_or(&[][..], |r| &r.support) {
            if !links.iter().any(|l| l.url == link.url) {
                links.push(link.clone());
            }
        }
    }
    links
}

/// Width of the field labels (`Community`, `Tooling`, …) in a tool entry.
const LABEL_W: usize = 10;

/// Append one tool's catalog entry to `out`: a header line, the description
/// wrapped to 72 columns, then aligned fields and links. Empty fields are left
/// out.
fn format_tool(out: &mut String, tool: &ToolDef, support: &[Link]) {
    format_tool_header(out, tool);
    for line in wrap(&tool.description, 72) {
        let _ = writeln!(out, "    {line}");
    }
    out.push('\n');

    if tool.roles.len() > 1 {
        let roles: Vec<&str> = Role::ALL
            .iter()
            .filter(|r| tool.roles.contains_key(r))
            .map(|r| r.as_kebab())
            .collect();
        write_field(out, "Roles", &roles.join(", "));
    }
    if !tool.languages.is_empty() {
        write_field(out, "Languages", &tool.languages.join(", "));
    }
    write_field(out, "Website", &tool.website);

    // Link names line up across both groups.
    let name_w = tool
        .community
        .iter()
        .chain(support)
        .map(|l| console::measure_text_width(&l.name))
        .max()
        .unwrap_or(0);
    write_links(out, "Community", &tool.community, name_w);
    write_links(out, "Tooling", support, name_w);
}

/// A tool already described under an earlier role: its header and a pointer.
fn format_tool_reference(out: &mut String, tool: &ToolDef) {
    format_tool_header(out, tool);
    let first = Role::ALL
        .iter()
        .find(|r| tool.roles.contains_key(r))
        .map_or("", |r| r.as_kebab());
    let _ = writeln!(out, "    {}", theme::dim(format!("See {first} above.")));
}

/// `[icon] Name  id  [experimental · needs --allow-experimental]`.
fn format_tool_header(out: &mut String, tool: &ToolDef) {
    let tag = if tool.experimental {
        format!(
            "  {}",
            theme::warn("experimental · needs --allow-experimental")
        )
    } else {
        String::new()
    };
    let _ = writeln!(
        out,
        "  {}{}  {}{tag}",
        theme::icon_prefix(tool),
        theme::accent_strong(&tool.name),
        theme::dim(&tool.id),
    );
}

/// `    Label      value`.
fn write_field(out: &mut String, label: &str, value: &str) {
    let _ = writeln!(
        out,
        "    {}  {value}",
        theme::dim(format!("{label:<LABEL_W$}"))
    );
}

/// `    Label      name  url` rows, with `label` on the first row only.
fn write_links(out: &mut String, label: &str, links: &[Link], name_w: usize) {
    for (i, link) in links.iter().enumerate() {
        let label = if i == 0 { label } else { "" };
        let _ = writeln!(
            out,
            "    {}  {:<name_w$}  {}",
            theme::dim(format!("{label:<LABEL_W$}")),
            link.name,
            link.url
        );
    }
}

/// Per-role columns for the `table` matrix, in `Role::ALL` order: each role's
/// tools, name-sorted, as `Name (id)` (the id is what the role flags take),
/// with experimental (`🧪`) and fullstack (`🧩`) tools tagged and, with
/// `icons`, prefixed by the tool's icon.
fn table_columns(registry: &Registry, icons: bool) -> Vec<(String, Vec<String>)> {
    Role::ALL
        .iter()
        .map(|&role| {
            let mut tools = registry.tools_for_role(role);
            tools.sort_by(|a, b| a.name.cmp(&b.name));
            let cells = tools
                .iter()
                .map(|t| {
                    let mut cell = format!("{} {}", t.name, theme::dim(format!("({})", t.id)));
                    if icons {
                        cell = format!("{}{cell}", theme::icon_cell(t));
                    }
                    if t.experimental {
                        cell.push_str(" 🧪");
                    }
                    if t.fullstack.is_some() {
                        cell.push_str(" 🧩");
                    }
                    cell
                })
                .collect();
            (role.to_string(), cells)
        })
        .collect()
}

/// The usage example under the `table` matrix.
const TABLE_EXAMPLE: &str =
    "cardano-init --name my-app --on-chain aiken --off-chain meshjs --devnet yaci";

/// `table`: one framed column per role, tools stacked down each column.
fn print_tool_table(registry: &Registry) {
    let columns = table_columns(registry, theme::icons_enabled());
    let depth = columns.iter().map(|(_, c)| c.len()).max().unwrap_or(0);

    let mut table = comfy_table::Table::new();
    table
        .load_preset(comfy_table::presets::UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS)
        .set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
    // Centred two-line headers: the role in bold over the flag that fills it,
    // noting when the flag can be repeated.
    table.set_header(
        Role::ALL
            .iter()
            .zip(&columns)
            .map(|(&role, (header, _))| {
                let flag = crate::cli::role_flag(role);
                let flag = if role.multiple() {
                    format!("{flag} (repeatable)")
                } else {
                    flag.to_string()
                };
                comfy_table::Cell::new(format!(
                    "{}\n{}",
                    theme::accent_strong(header),
                    theme::dim(flag)
                ))
                .set_alignment(comfy_table::CellAlignment::Center)
            })
            .collect::<Vec<_>>(),
    );
    for i in 0..depth {
        table.add_row(
            columns
                .iter()
                .map(|(_, cells)| cells.get(i).cloned().unwrap_or_default())
                .collect::<Vec<_>>(),
        );
    }

    println!();
    print_rule("Tools by role");
    println!();
    for line in table.to_string().lines() {
        println!("{}{}", theme::PAD, line);
    }
    println!();
    for note in [
        "🧪 experimental — needs --allow-experimental",
        "🧩 fullstack — --fullstack fills on-chain and off-chain as one protocol/ component",
    ] {
        println!("{}{}", theme::PAD, theme::dim(note));
    }
    println!();
    println!("{}{} {}", theme::PAD, theme::dim("e.g."), TABLE_EXAMPLE);
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_tool_lists_community_links() {
        let registry = Registry::load().unwrap();
        for tool in registry.all_tools() {
            let mut out = String::new();
            format_tool(&mut out, tool, &[]);
            for link in &tool.community {
                assert!(
                    out.lines()
                        .any(|l| l.contains(&link.name) && l.contains(&link.url)),
                    "{}: community link '{}' missing from list output",
                    tool.id,
                    link.name
                );
            }
            assert_eq!(out.contains("Community"), !tool.community.is_empty());
        }
    }

    #[test]
    fn format_tool_lists_tooling_from_system_deps() {
        let registry = Registry::load().unwrap();
        let catalog = DepCatalog::load().unwrap();
        let aiken = registry.get("aiken").unwrap();
        let support = tool_support_links(aiken, &catalog);
        assert!(!support.is_empty());
        let mut out = String::new();
        format_tool(&mut out, aiken, &support);
        assert!(out.contains("Tooling"));
        for link in &support {
            assert!(
                out.lines()
                    .any(|l| l.contains(&link.name) && l.contains(&link.url))
            );
        }

        let meshjs = registry.get("meshjs").unwrap();
        assert!(tool_support_links(meshjs, &catalog).is_empty());
    }

    /// Drop terminal styling so cells compare as plain text.
    fn plain(cols: Vec<(String, Vec<String>)>) -> Vec<(String, Vec<String>)> {
        cols.into_iter()
            .map(|(h, cells)| {
                let cells = cells
                    .iter()
                    .map(|c| console::strip_ansi_codes(c).into_owned())
                    .collect();
                (h, cells)
            })
            .collect()
    }

    #[test]
    fn table_example_uses_real_tool_ids() {
        let reg = Registry::load().unwrap();
        let words: Vec<&str> = TABLE_EXAMPLE.split_whitespace().collect();
        for pair in words.windows(2) {
            if Role::ALL
                .iter()
                .any(|&r| crate::cli::role_flag(r) == pair[0])
            {
                assert!(reg.get(pair[1]).is_some(), "unknown tool id '{}'", pair[1]);
            }
        }
    }

    #[test]
    fn table_columns_group_tools_by_role_and_tag_experimental() {
        let reg = Registry::load().unwrap();
        let cols = plain(table_columns(&reg, false));

        let headers: Vec<&str> = cols.iter().map(|(h, _)| h.as_str()).collect();
        assert_eq!(
            headers,
            vec![
                "On-chain",
                "Off-chain",
                "Infrastructure",
                "Devnet",
                "Formal methods"
            ]
        );

        let col = |name: &str| -> &Vec<String> { &cols.iter().find(|(h, _)| h == name).unwrap().1 };

        // A tool filling two roles appears in both columns (e.g. Scalus).
        // It is also tagged as fullstack.
        assert!(col("On-chain").iter().any(|t| t == "Scalus (scalus) 🧩"));
        assert!(col("Off-chain").iter().any(|t| t == "Scalus (scalus) 🧩"));

        // Experimental tools are tagged; stable ones are not.
        assert!(col("Off-chain").iter().any(|t| t == "Tx3 (tx3) 🧪"));
        assert!(
            col("Formal methods")
                .iter()
                .any(|t| t == "Blaster (blaster) 🧪")
        );
        assert!(col("On-chain").iter().any(|t| t == "Aiken (aiken)"));

        // Cells are name-sorted within a column.
        let infra = col("Infrastructure");
        let mut sorted = infra.clone();
        sorted.sort();
        assert_eq!(infra, &sorted);
    }

    #[test]
    fn table_columns_prefix_icons_when_asked() {
        let reg = Registry::load().unwrap();
        let cols = plain(table_columns(&reg, true));
        let col = |name: &str| -> &Vec<String> { &cols.iter().find(|(h, _)| h == name).unwrap().1 };
        let haskell = theme::language_icon("haskell").unwrap();
        let term = theme::terminal_icon().to_string();

        // Code tools get their primary language's icon.
        assert!(col("On-chain").contains(&format!("{haskell} Plinth (plinth)")));
        assert!(col("Formal methods").contains(&"∀ Blaster (blaster) 🧪".to_string()));
        // Tools you run get a terminal, whatever their language.
        assert!(col("Infrastructure").iter().all(|t| t.starts_with(&term)));
        assert!(col("Devnet").contains(&format!("{term} Yaci DevKit (yaci)")));
        assert!(col("Off-chain").contains(&"3 Tx3 (tx3) 🧪".to_string()));
    }
}
