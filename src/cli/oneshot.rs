use super::CliError;
use crate::registry::loader::Registry;
use crate::registry::mcp;
use crate::registry::types::{Network, Role, RoleAssignment, Selection};

/// What the one-shot flags said about MCP servers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpChoice {
    /// Neither `--mcp` nor `--no-mcp`: every applicable server with
    /// `default = true` (the same set the wizard pre-selects).
    Default,
    /// `--no-mcp`: no `.mcp.json`.
    None,
    /// `--mcp <id>…`: exactly these servers; each must exist and apply.
    Explicit(Vec<String>),
}

/// Build a `Selection` from one-shot CLI flags.
///
/// Assumes `args.name` is `Some` (caller verified before calling this).
// One positional per role flag mirrors the CLI surface; grouping them into a
// struct would just shift the argument list elsewhere with no real gain.
#[allow(clippy::too_many_arguments)]
pub fn build_selection(
    name: &str,
    on_chain: Option<&str>,
    off_chain: Option<&str>,
    fullstack: Option<&str>,
    infra: &[String],
    devnet: Option<&str>,
    formal_methods: Option<&str>,
    nix: bool,
    spec: bool,
    mcp: McpChoice,
    registry: &Registry,
) -> Result<Selection, CliError> {
    validate_project_name(name)?;

    let mut assignments = Vec::new();

    // `--fullstack X` fills both on-chain and off-chain with one tool; the same
    // tool on both roles collapses into a single `protocol/` component at planning
    // time (TECH_SPEC §3.4). The tool must declare a `[fullstack]` template.
    // (The caller guarantees fullstack is not combined with --on-chain/--off-chain.)
    if let Some(tool_id) = fullstack {
        validate_fullstack_tool(tool_id, registry)?;
        assignments.push(RoleAssignment {
            role: Role::OnChain,
            tool_id: tool_id.to_string(),
        });
        assignments.push(RoleAssignment {
            role: Role::OffChain,
            tool_id: tool_id.to_string(),
        });
    }

    if let Some(tool_id) = on_chain {
        validate_tool_for_role(tool_id, Role::OnChain, registry)?;
        assignments.push(RoleAssignment {
            role: Role::OnChain,
            tool_id: tool_id.to_string(),
        });
    }

    if let Some(tool_id) = off_chain {
        validate_tool_for_role(tool_id, Role::OffChain, registry)?;
        assignments.push(RoleAssignment {
            role: Role::OffChain,
            tool_id: tool_id.to_string(),
        });
    }

    // Infrastructure is the only repeatable role. De-duplicate keep-first so a
    // repeated `--infra X --infra X` can't emit `infra/X/` twice (§3.4).
    let mut seen_infra = Vec::new();
    for tool_id in infra {
        if seen_infra.contains(tool_id) {
            continue;
        }
        validate_tool_for_role(tool_id, Role::Infrastructure, registry)?;
        seen_infra.push(tool_id.clone());
        assignments.push(RoleAssignment {
            role: Role::Infrastructure,
            tool_id: tool_id.clone(),
        });
    }

    if let Some(tool_id) = devnet {
        validate_tool_for_role(tool_id, Role::Devnet, registry)?;
        assignments.push(RoleAssignment {
            role: Role::Devnet,
            tool_id: tool_id.to_string(),
        });
    }

    if let Some(tool_id) = formal_methods {
        validate_tool_for_role(tool_id, Role::FormalMethods, registry)?;
        assignments.push(RoleAssignment {
            role: Role::FormalMethods,
            tool_id: tool_id.to_string(),
        });
    }

    if assignments.is_empty() {
        return Err(CliError::NoRolesSelected);
    }

    let mcp = resolve_mcp_choice(mcp, &assignments, registry)?;

    // Always preview; switch networks by editing CARDANO_NETWORK in the generated .env.
    Ok(Selection {
        project_name: name.to_string(),
        assignments,
        network: Network::Preview,
        nix,
        spec,
        mcp,
    })
}

/// Turn the `--mcp` / `--no-mcp` flags into the selection's server ids.
/// Explicit ids are validated: unknown → `unknown_mcp` (listing the registry
/// ids); known but not applicable to these assignments → `mcp_requires_tool`.
/// Duplicates are deduped keep-first; the result is id-sorted (§11).
fn resolve_mcp_choice(
    choice: McpChoice,
    assignments: &[RoleAssignment],
    registry: &Registry,
) -> Result<Vec<String>, CliError> {
    match choice {
        McpChoice::Default => Ok(mcp::default_ids(registry, assignments)),
        McpChoice::None => Ok(Vec::new()),
        McpChoice::Explicit(ids) => {
            let mut out: Vec<String> = Vec::new();
            for id in ids {
                if out.contains(&id) {
                    continue;
                }
                let def = registry.mcp(&id).ok_or_else(|| CliError::UnknownMcp {
                    mcp_id: id.clone(),
                    valid_mcp: registry
                        .mcp_servers()
                        .iter()
                        .map(|m| m.id.clone())
                        .collect(),
                })?;
                if !def.applies_to(assignments) {
                    let req = def
                        .requires
                        .as_ref()
                        .expect("a server that does not apply must have a requirement");
                    return Err(CliError::McpRequiresTool {
                        mcp_id: id,
                        role: req.role.as_kebab().to_string(),
                        tool: req.tool.clone(),
                    });
                }
                out.push(id);
            }
            out.sort();
            Ok(out)
        }
    }
}

/// Validate a project name: non-empty, no path separators, no leading dots,
/// only alphanumeric + hyphens + underscores.
pub fn validate_project_name(name: &str) -> Result<(), CliError> {
    if name.is_empty() {
        return Err(CliError::InvalidProjectName {
            name: name.to_string(),
            reason: "must not be empty".to_string(),
        });
    }
    if name.starts_with('.') {
        return Err(CliError::InvalidProjectName {
            name: name.to_string(),
            reason: "must not start with a dot".to_string(),
        });
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(CliError::InvalidProjectName {
            name: name.to_string(),
            reason: "may only contain letters, digits, hyphens, and underscores".to_string(),
        });
    }
    Ok(())
}

/// Validate that a `--fullstack <tool>` target exists and declares a `[fullstack]`
/// template. On failure lists the fullstack-capable tools so the user can pick a
/// valid one. A tool with a `[fullstack]` template is guaranteed (at registry
/// load) to fill both on-chain and off-chain, so this is the only check needed.
pub(crate) fn validate_fullstack_tool(tool_id: &str, registry: &Registry) -> Result<(), CliError> {
    let is_fullstack = registry.get(tool_id).is_some_and(|t| t.fullstack.is_some());
    if is_fullstack {
        return Ok(());
    }
    let mut valid_tools: Vec<String> = registry
        .all_tools()
        .iter()
        .filter(|t| t.fullstack.is_some())
        .map(|t| t.id.clone())
        .collect();
    valid_tools.sort();
    Err(CliError::FullstackUnsupported {
        tool_id: tool_id.to_string(),
        valid_tools,
    })
}

pub(crate) fn validate_tool_for_role(
    tool_id: &str,
    role: Role,
    registry: &Registry,
) -> Result<(), CliError> {
    let tool = registry.get(tool_id).ok_or_else(|| {
        let mut valid_tools: Vec<String> = registry
            .tools_for_role(role)
            .iter()
            .map(|t| t.id.clone())
            .collect();
        valid_tools.sort();
        CliError::UnknownTool {
            tool_id: tool_id.to_string(),
            role: role.to_string(),
            valid_tools,
        }
    })?;
    if !tool.roles.contains_key(&role) {
        let mut valid_roles: Vec<String> = tool
            .roles
            .keys()
            .map(|r| r.as_kebab().to_string())
            .collect();
        valid_roles.sort();
        return Err(CliError::ToolRoleMismatch {
            tool_id: tool_id.to_string(),
            role: role.to_string(),
            valid_roles,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::load().expect("registry should load")
    }

    #[test]
    fn valid_single_role() {
        let sel = build_selection(
            "my-project",
            Some("aiken"),
            None,
            None,
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        )
        .unwrap();

        assert_eq!(sel.project_name, "my-project");
        assert_eq!(sel.assignments.len(), 1);
        assert_eq!(sel.assignments[0].tool_id, "aiken");
        assert_eq!(sel.assignments[0].role, Role::OnChain);
    }

    #[test]
    fn valid_multiple_roles() {
        let sel = build_selection(
            "test-proj",
            Some("aiken"),
            Some("meshjs"),
            None,
            &[],
            Some("yaci"),
            None,
            true,
            true,
            McpChoice::Default,
            &registry(),
        )
        .unwrap();

        assert_eq!(sel.assignments.len(), 3);
        assert!(sel.nix);
        assert!(sel.spec);
        // Network is fixed at scaffold time; switching is a `.env` edit.
        assert_eq!(sel.network.to_string(), "preview");
    }

    #[test]
    fn valid_formal_methods() {
        let sel = build_selection(
            "my-project",
            None,
            None,
            None,
            &[],
            None,
            Some("blaster"),
            false,
            true,
            McpChoice::Default,
            &registry(),
        )
        .unwrap();

        assert_eq!(sel.assignments.len(), 1);
        assert_eq!(sel.assignments[0].tool_id, "blaster");
        assert_eq!(sel.assignments[0].role, Role::FormalMethods);
    }

    #[test]
    fn unknown_tool_errors() {
        let result = build_selection(
            "test",
            Some("nonexistent"),
            None,
            None,
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::UnknownTool { .. })));
    }

    #[test]
    fn tool_role_mismatch_errors() {
        // Aiken doesn't support off-chain
        let result = build_selection(
            "test",
            None,
            Some("aiken"),
            None,
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::ToolRoleMismatch { .. })));
    }

    #[test]
    fn duplicate_infra_is_deduplicated() {
        let sel = build_selection(
            "test",
            None,
            None,
            None,
            &["kupo".to_string(), "kupo".to_string()],
            None,
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        )
        .unwrap();

        let infra_count = sel
            .assignments
            .iter()
            .filter(|a| a.role == Role::Infrastructure)
            .count();
        assert_eq!(infra_count, 1);
    }

    #[test]
    fn fullstack_flag_pushes_both_roles() {
        // `--fullstack scalus` expands to on-chain + off-chain for scalus.
        let sel = build_selection(
            "my-project",
            None,
            None,
            Some("scalus"),
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        )
        .unwrap();

        let roles: Vec<Role> = sel.assignments.iter().map(|a| a.role).collect();
        assert!(roles.contains(&Role::OnChain));
        assert!(roles.contains(&Role::OffChain));
        assert!(sel.assignments.iter().all(|a| a.tool_id == "scalus"));
        assert_eq!(sel.assignments.len(), 2);
    }

    #[test]
    fn fullstack_on_unsupported_tool_errors() {
        // meshjs has no [fullstack] template → FullstackUnsupported, listing the
        // fullstack-capable tools.
        let result = build_selection(
            "test",
            None,
            None,
            Some("meshjs"),
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        );
        match result {
            Err(CliError::FullstackUnsupported {
                tool_id,
                valid_tools,
            }) => {
                assert_eq!(tool_id, "meshjs");
                assert!(valid_tools.contains(&"scalus".to_string()));
            }
            other => panic!("expected FullstackUnsupported, got {other:?}"),
        }
    }

    #[test]
    fn no_roles_errors() {
        let result = build_selection(
            "test",
            None,
            None,
            None,
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::NoRolesSelected)));
    }

    #[test]
    fn mcp_default_picks_applicable_defaults() {
        let sel = build_selection(
            "p",
            Some("aiken"),
            None,
            None,
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        )
        .unwrap();
        assert_eq!(sel.mcp, vec!["cardano-debug"]);

        let sel = build_selection(
            "p",
            Some("aiken"),
            None,
            None,
            &[],
            Some("yaci"),
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        )
        .unwrap();
        // yaci-devkit applies here but is opt-in (`default = false`), so the
        // default set is unchanged; it must be named explicitly.
        assert_eq!(sel.mcp, vec!["cardano-debug"]);
    }

    #[test]
    fn mcp_none_is_empty() {
        let sel = build_selection(
            "p",
            Some("aiken"),
            None,
            None,
            &[],
            Some("yaci"),
            None,
            false,
            true,
            McpChoice::None,
            &registry(),
        )
        .unwrap();
        assert!(sel.mcp.is_empty());
    }

    #[test]
    fn mcp_explicit_is_validated_deduped_and_sorted() {
        let sel = build_selection(
            "p",
            Some("aiken"),
            None,
            None,
            &[],
            Some("yaci"),
            None,
            false,
            true,
            McpChoice::Explicit(vec![
                "yaci-devkit".into(),
                "cardano-debug".into(),
                "yaci-devkit".into(),
            ]),
            &registry(),
        )
        .unwrap();
        assert_eq!(sel.mcp, vec!["cardano-debug", "yaci-devkit"]);

        let err = build_selection(
            "p",
            Some("aiken"),
            None,
            None,
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Explicit(vec!["bogus".into()]),
            &registry(),
        )
        .unwrap_err();
        match err {
            CliError::UnknownMcp { mcp_id, valid_mcp } => {
                assert_eq!(mcp_id, "bogus");
                assert_eq!(valid_mcp, vec!["cardano-debug", "yaci-devkit"]);
            }
            other => panic!("expected UnknownMcp, got {other:?}"),
        }

        // yaci-devkit without a yaci devnet is a usage error naming the gap.
        let err = build_selection(
            "p",
            Some("aiken"),
            None,
            None,
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Explicit(vec!["yaci-devkit".into()]),
            &registry(),
        )
        .unwrap_err();
        match err {
            CliError::McpRequiresTool { mcp_id, role, tool } => {
                assert_eq!(mcp_id, "yaci-devkit");
                assert_eq!(role, "devnet");
                assert_eq!(tool, "yaci");
            }
            other => panic!("expected McpRequiresTool, got {other:?}"),
        }
    }

    #[test]
    fn invalid_project_name_empty() {
        let result = build_selection(
            "",
            Some("aiken"),
            None,
            None,
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::InvalidProjectName { .. })));
    }

    #[test]
    fn invalid_project_name_dot() {
        let result = build_selection(
            ".hidden",
            Some("aiken"),
            None,
            None,
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::InvalidProjectName { .. })));
    }

    #[test]
    fn invalid_project_name_slash() {
        let result = build_selection(
            "bad/name",
            Some("aiken"),
            None,
            None,
            &[],
            None,
            None,
            false,
            true,
            McpChoice::Default,
            &registry(),
        );
        assert!(matches!(result, Err(CliError::InvalidProjectName { .. })));
    }
}
