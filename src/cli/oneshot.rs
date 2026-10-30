//! Flag → `Selection` for non-interactive runs: validates role flags against the
//! registry, producing the machine-readable errors agents rely on.

use super::CliError;
use crate::registry::loader::Registry;
use crate::registry::types::{Network, Role, RoleAssignment, Selection};

/// The role flags shared by one-shot `init` and `add`, borrowed from either.
#[derive(Debug, Default, Clone, Copy)]
pub struct RoleFlags<'a> {
    pub on_chain: Option<&'a str>,
    pub off_chain: Option<&'a str>,
    pub fullstack: Option<&'a str>,
    pub infra: &'a [String],
    pub devnet: Option<&'a str>,
    pub formal_methods: Option<&'a str>,
}

impl RoleFlags<'_> {
    /// `--fullstack X` is sugar for `--on-chain X --off-chain X`, so combining it
    /// with either is ambiguous (tech spec §2.2).
    pub fn has_fullstack_conflict(&self) -> bool {
        self.fullstack.is_some() && (self.on_chain.is_some() || self.off_chain.is_some())
    }

    /// Validate every flag and expand it into role assignments, in flag order.
    ///
    /// A fullstack tool is assigned to both on-chain and off-chain (collapsed
    /// into one `protocol/` component at planning time, tech spec §3.4).
    /// Repeated infra ids are de-duplicated keep-first.
    ///
    /// # Errors
    /// [`CliError::UnknownTool`], [`CliError::ToolRoleMismatch`] or
    /// [`CliError::FullstackUnsupported`] for the first invalid flag.
    pub fn assignments(&self, registry: &Registry) -> Result<Vec<RoleAssignment>, CliError> {
        let mut assignments = Vec::new();
        let mut assign = |role, tool_id: &str| {
            assignments.push(RoleAssignment {
                role,
                tool_id: tool_id.to_string(),
            });
        };

        if let Some(tool_id) = self.fullstack {
            validate_fullstack_tool(tool_id, registry)?;
            assign(Role::OnChain, tool_id);
            assign(Role::OffChain, tool_id);
        }
        for (role, tool_id) in [
            (Role::OnChain, self.on_chain),
            (Role::OffChain, self.off_chain),
        ] {
            if let Some(tool_id) = tool_id {
                validate_tool_for_role(tool_id, role, registry)?;
                assign(role, tool_id);
            }
        }
        for (i, tool_id) in self.infra.iter().enumerate() {
            if self.infra[..i].contains(tool_id) {
                continue;
            }
            validate_tool_for_role(tool_id, Role::Infrastructure, registry)?;
            assign(Role::Infrastructure, tool_id);
        }
        for (role, tool_id) in [
            (Role::Devnet, self.devnet),
            (Role::FormalMethods, self.formal_methods),
        ] {
            if let Some(tool_id) = tool_id {
                validate_tool_for_role(tool_id, role, registry)?;
                assign(role, tool_id);
            }
        }
        Ok(assignments)
    }
}

/// Build a `Selection` from one-shot flags.
///
/// # Errors
/// An invalid `name`, any invalid role flag (see [`RoleFlags::assignments`]), or
/// [`CliError::NoRolesSelected`] when no role flag was given.
pub fn build_selection(
    name: &str,
    roles: &RoleFlags<'_>,
    nix: bool,
    spec: bool,
    registry: &Registry,
) -> Result<Selection, CliError> {
    validate_project_name(name)?;

    let assignments = roles.assignments(registry)?;
    if assignments.is_empty() {
        return Err(CliError::NoRolesSelected);
    }

    // Always preview; switch networks by editing CARDANO_NETWORK in the generated .env.
    Ok(Selection {
        project_name: name.to_string(),
        assignments,
        network: Network::Preview,
        nix,
        spec,
    })
}

/// Validate a project name: non-empty, no leading dot, and only letters,
/// digits, hyphens and underscores (so never a path).
pub fn validate_project_name(name: &str) -> Result<(), CliError> {
    let reason = if name.is_empty() {
        "must not be empty"
    } else if name.starts_with('.') {
        "must not start with a dot"
    } else if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        "may only contain letters, digits, hyphens, and underscores"
    } else {
        return Ok(());
    };
    Err(CliError::InvalidProjectName {
        name: name.to_string(),
        reason: reason.to_string(),
    })
}

/// Check that a `--fullstack` target declares a `[fullstack]` template (the
/// registry guarantees such a tool fills both on-chain and off-chain). The error
/// lists the fullstack-capable tools.
fn validate_fullstack_tool(tool_id: &str, registry: &Registry) -> Result<(), CliError> {
    if registry.get(tool_id).is_some_and(|t| t.fullstack.is_some()) {
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

/// Check that `tool_id` exists and fills `role`; the error lists the valid
/// alternatives (tools for the role, or roles for the tool).
fn validate_tool_for_role(tool_id: &str, role: Role, registry: &Registry) -> Result<(), CliError> {
    let Some(tool) = registry.get(tool_id) else {
        let mut valid_tools: Vec<String> = registry
            .tools_for_role(role)
            .iter()
            .map(|t| t.id.clone())
            .collect();
        valid_tools.sort();
        return Err(CliError::UnknownTool {
            tool_id: tool_id.to_string(),
            role: role.to_string(),
            valid_tools,
        });
    };
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

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::load().expect("registry should load")
    }

    fn build(name: &str, roles: &RoleFlags<'_>) -> Result<Selection, CliError> {
        build_selection(name, roles, false, true, &registry())
    }

    #[test]
    fn valid_single_role() {
        let roles = RoleFlags {
            on_chain: Some("aiken"),
            ..RoleFlags::default()
        };
        let sel = build("my-project", &roles).unwrap();

        assert_eq!(sel.project_name, "my-project");
        assert_eq!(sel.assignments.len(), 1);
        assert_eq!(sel.assignments[0].tool_id, "aiken");
        assert_eq!(sel.assignments[0].role, Role::OnChain);
    }

    #[test]
    fn valid_multiple_roles() {
        let roles = RoleFlags {
            on_chain: Some("aiken"),
            off_chain: Some("meshjs"),
            devnet: Some("yaci"),
            ..RoleFlags::default()
        };
        let sel = build_selection("test-proj", &roles, true, true, &registry()).unwrap();

        assert_eq!(sel.assignments.len(), 3);
        assert!(sel.nix);
        assert!(sel.spec);
        // Network is fixed at scaffold time; switching is a `.env` edit.
        assert_eq!(sel.network.to_string(), "preview");
    }

    #[test]
    fn valid_formal_methods() {
        let roles = RoleFlags {
            formal_methods: Some("blaster"),
            ..RoleFlags::default()
        };
        let sel = build("my-project", &roles).unwrap();

        assert_eq!(sel.assignments.len(), 1);
        assert_eq!(sel.assignments[0].tool_id, "blaster");
        assert_eq!(sel.assignments[0].role, Role::FormalMethods);
    }

    #[test]
    fn unknown_tool_errors() {
        let roles = RoleFlags {
            on_chain: Some("nonexistent"),
            ..RoleFlags::default()
        };
        let result = build("test", &roles);
        assert!(matches!(result, Err(CliError::UnknownTool { .. })));
    }

    #[test]
    fn tool_role_mismatch_errors() {
        // Aiken doesn't support off-chain
        let roles = RoleFlags {
            off_chain: Some("aiken"),
            ..RoleFlags::default()
        };
        let result = build("test", &roles);
        assert!(matches!(result, Err(CliError::ToolRoleMismatch { .. })));
    }

    #[test]
    fn duplicate_infra_is_deduplicated() {
        let infra = ["kupo".to_string(), "kupo".to_string()];
        let roles = RoleFlags {
            infra: &infra,
            ..RoleFlags::default()
        };
        let sel = build("test", &roles).unwrap();

        let infra_count = sel
            .assignments
            .iter()
            .filter(|a| a.role == Role::Infrastructure)
            .count();
        assert_eq!(infra_count, 1);
    }

    #[test]
    fn fullstack_flag_pushes_both_roles() {
        let roles = RoleFlags {
            fullstack: Some("scalus"),
            ..RoleFlags::default()
        };
        let sel = build("my-project", &roles).unwrap();

        let roles: Vec<Role> = sel.assignments.iter().map(|a| a.role).collect();
        assert!(roles.contains(&Role::OnChain));
        assert!(roles.contains(&Role::OffChain));
        assert!(sel.assignments.iter().all(|a| a.tool_id == "scalus"));
        assert_eq!(sel.assignments.len(), 2);
    }

    #[test]
    fn fullstack_on_unsupported_tool_errors() {
        // meshjs has no [fullstack] template.
        let roles = RoleFlags {
            fullstack: Some("meshjs"),
            ..RoleFlags::default()
        };
        match build("test", &roles) {
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
    fn fullstack_conflict_detected() {
        let roles = RoleFlags {
            fullstack: Some("scalus"),
            off_chain: Some("meshjs"),
            ..RoleFlags::default()
        };
        assert!(roles.has_fullstack_conflict());
    }

    #[test]
    fn no_roles_errors() {
        let result = build("test", &RoleFlags::default());
        assert!(matches!(result, Err(CliError::NoRolesSelected)));
    }

    #[test]
    fn invalid_project_names_are_rejected() {
        for name in ["", ".hidden", "bad/name"] {
            let roles = RoleFlags {
                on_chain: Some("aiken"),
                ..RoleFlags::default()
            };
            let result = build(name, &roles);
            assert!(
                matches!(result, Err(CliError::InvalidProjectName { .. })),
                "{name:?}"
            );
        }
    }
}
