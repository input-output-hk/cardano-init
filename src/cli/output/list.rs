//! The `list` presenter: the role/tool catalog as text, a role matrix, or JSON.

use std::fmt::Write;

use serde_json::json;

use super::emit_json_ok;
use super::layout::{borderless_table, print_rule, print_table, wrap};
use crate::cli::theme;
use crate::doctor::catalog::DepCatalog;
use crate::registry::loader::Registry;
use crate::registry::types::{Link, Role, ToolDef};
use crate::registry::view;

/// Print the registry for `cardano-init list`: the tech spec §8 payload with
/// `json`, a role × tool matrix with `table`, else the full catalog.
pub fn print_list(registry: &Registry, catalog: &DepCatalog, json: bool, table: bool) {
    if json {
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
        return;
    }

    if table {
        print_list_table(registry);
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
            role.display,
            theme::dim(role.dir).to_string(),
            theme::dim(multi).to_string(),
        ]);
    }
    print_table(roles_table, theme::PAD);

    println!();
    print_rule("Tools");
    // Sorted by id to match the JSON ordering.
    let mut tools: Vec<&ToolDef> = registry.all_tools().iter().collect();
    tools.sort_by(|a, b| a.id.cmp(&b.id));
    let mut block = String::new();
    for tool in tools {
        block.push('\n');
        format_tool(&mut block, tool, &tool_support_links(tool, catalog));
    }
    print!("{block}");
    println!();
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

/// Append one tool's catalog entry to `out`: header, facts, links, and the
/// description wrapped to 76 columns.
fn format_tool(out: &mut String, tool: &ToolDef, support: &[Link]) {
    let mut roles: Vec<&str> = tool.roles.keys().map(|r| r.as_kebab()).collect();
    roles.sort_unstable();
    let experimental_tag = if tool.experimental {
        "  [experimental]"
    } else {
        ""
    };
    let _ = writeln!(out, "  {} ({}){}", tool.name, tool.id, experimental_tag);
    if tool.experimental {
        let _ = writeln!(
            out,
            "    Status:    experimental — may be unstable or incomplete; needs --allow-experimental"
        );
    }
    let _ = writeln!(out, "    Roles:     {}", roles.join(", "));
    let _ = writeln!(out, "    Languages: {}", tool.languages.join(", "));
    let _ = writeln!(out, "    Website:   {}", tool.website);
    write_links(out, "Community:", &tool.community);
    write_links(out, "Tooling:", support);
    let _ = writeln!(out, "    {}", wrap(&tool.description, 72).join("\n    "));
}

/// `    <label>  name: url` rows, with `label` on the first row only.
fn write_links(out: &mut String, label: &str, links: &[Link]) {
    for (i, link) in links.iter().enumerate() {
        let label = if i == 0 { label } else { "" };
        let _ = writeln!(out, "    {label:<10} {}: {}", link.name, link.url);
    }
}

/// Per-role columns for the `list --table` matrix, in `Role::ALL` order: each
/// role's tool names, name-sorted, with experimental tools tagged.
fn list_matrix_columns(registry: &Registry) -> Vec<(String, Vec<String>)> {
    Role::ALL
        .iter()
        .map(|&role| {
            let mut tools = registry.tools_for_role(role);
            tools.sort_by(|a, b| a.name.cmp(&b.name));
            let cells = tools
                .iter()
                .map(|t| {
                    if t.experimental {
                        format!("{} 🧪", t.name)
                    } else {
                        t.name.clone()
                    }
                })
                .collect();
            (role.to_string(), cells)
        })
        .collect()
}

/// `list --table`: one framed column per role, tools stacked down each column.
fn print_list_table(registry: &Registry) {
    let columns = list_matrix_columns(registry);
    let depth = columns.iter().map(|(_, c)| c.len()).max().unwrap_or(0);

    let mut table = comfy_table::Table::new();
    table
        .load_preset(comfy_table::presets::UTF8_FULL)
        .set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
    table.set_header(
        columns
            .iter()
            .map(|(header, _)| theme::accent(header).to_string())
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
    println!(
        "{}{}",
        theme::PAD,
        theme::dim("🧪 experimental — needs --allow-experimental")
    );
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
                    out.contains(&format!("{}: {}", link.name, link.url)),
                    "{}: community link '{}' missing from list output",
                    tool.id,
                    link.name
                );
            }
            assert_eq!(out.contains("Community:"), !tool.community.is_empty());
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
        assert!(out.contains("Tooling:"));
        for link in &support {
            assert!(out.contains(&format!("{}: {}", link.name, link.url)));
        }

        let meshjs = registry.get("meshjs").unwrap();
        assert!(tool_support_links(meshjs, &catalog).is_empty());
    }

    #[test]
    fn list_matrix_columns_group_tools_by_role_and_tag_experimental() {
        let reg = Registry::load().unwrap();
        let cols = list_matrix_columns(&reg);

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
        assert!(col("On-chain").iter().any(|t| t == "Scalus"));
        assert!(col("Off-chain").iter().any(|t| t == "Scalus"));

        // Experimental tools are tagged; stable ones are not.
        assert!(col("Off-chain").iter().any(|t| t == "Tx3 🧪"));
        assert!(col("Formal methods").iter().any(|t| t == "Blaster 🧪"));
        assert!(col("On-chain").iter().any(|t| t == "Aiken"));

        // Cells are name-sorted within a column.
        let infra = col("Infrastructure");
        let mut sorted = infra.clone();
        sorted.sort();
        assert_eq!(infra, &sorted);
    }
}
