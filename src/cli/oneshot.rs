use super::CliError;
use crate::registry::loader::Registry;
use crate::registry::types::{Network, Role, RoleAssignment, Selection};

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

    // Always preview; switch networks by editing CARDANO_NETWORK in the generated .env.
    Ok(Selection {
        project_name: name.to_string(),
        assignments,
        network: Network::Preview,
        nix,
        spec: true,
    })
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
            &registry(),
        )
        .unwrap();

        assert_eq!(sel.assignments.len(), 3);
        assert!(sel.nix);
        // The spec folder is on by default; `--no-spec` is applied by the caller.
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
            &registry(),
        );
        assert!(matches!(result, Err(CliError::NoRolesSelected)));
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
            &registry(),
        );
        assert!(matches!(result, Err(CliError::InvalidProjectName { .. })));
    }
}
