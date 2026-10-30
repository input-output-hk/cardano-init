use std::collections::HashMap;

use rust_embed::RustEmbed;
use serde::Deserialize;

use super::types::{
    CompatConfig, DetectSignature, InfraConfig, Link, Role, RoleConfig, Seam, ToolDef,
    UnknownRoleError, UnknownSeamError,
};

#[derive(RustEmbed)]
#[folder = "registry/"]
struct RegistryAssets;

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("failed to parse tool file '{file}': {source}")]
    Parse {
        file: String,
        source: toml::de::Error,
    },

    #[error("unknown role '{role}' in tool file '{file}': {source}")]
    UnknownRole {
        file: String,
        role: String,
        source: UnknownRoleError,
    },

    #[error("unknown seam '{seam}' in tool file '{file}': {source}")]
    UnknownSeam {
        file: String,
        seam: String,
        source: UnknownSeamError,
    },

    #[error("duplicate tool id '{id}'")]
    DuplicateId { id: String },

    #[error(
        "tool '{file}' fills the infrastructure role but is missing the required [infra] table"
    )]
    InfraConfigMissing { file: String },

    #[error(
        "tool '{file}' declares a [fullstack] template but must also declare both \
         [roles.on-chain] and [roles.off-chain]"
    )]
    FullstackRolesMissing { file: String },

    #[error("no tool definitions found in registry")]
    Empty,
}

/// On-disk shape of `registry/tools/<tool>.toml`.
#[derive(Deserialize)]
struct ToolFileToml {
    tool: ToolMetaToml,
    #[serde(default)]
    roles: HashMap<String, RoleConfig>,
    infra: Option<InfraConfig>,
    fullstack: Option<RoleConfig>,
    #[serde(default)]
    compat: CompatToml,
}

#[derive(Deserialize)]
struct ToolMetaToml {
    id: String,
    name: String,
    description: String,
    website: String,
    #[serde(default)]
    community: Vec<Link>,
    languages: Vec<String>,
    #[serde(default)]
    system_deps: Vec<String>,
    #[serde(default)]
    nix_packages: Vec<String>,
    #[serde(default)]
    nix_self_contained: bool,
    #[serde(default)]
    detect: Vec<DetectToml>,
    #[serde(default)]
    experimental: bool,
}

#[derive(Deserialize, Default)]
struct CompatToml {
    #[serde(default)]
    consumes: Vec<String>,
    #[serde(default)]
    serves: Vec<String>,
    #[serde(default)]
    self_contained_devnet: bool,
}

/// A `detect` entry is either a bare path (existence check) or a table with an
/// optional `contains` substring (content check).
#[derive(Deserialize)]
#[serde(untagged)]
enum DetectToml {
    Simple(String),
    Detailed {
        file: String,
        #[serde(default)]
        contains: Option<String>,
    },
}

impl From<DetectToml> for DetectSignature {
    fn from(raw: DetectToml) -> Self {
        match raw {
            DetectToml::Simple(file) => DetectSignature {
                file,
                contains: None,
            },
            DetectToml::Detailed { file, contains } => DetectSignature { file, contains },
        }
    }
}

fn parse_tool_file(file_name: &str) -> Result<ToolDef, RegistryError> {
    let asset = RegistryAssets::get(file_name).expect("asset listed by iter() must exist");
    let text = std::str::from_utf8(&asset.data).expect("tool TOML files must be valid UTF-8");
    let raw = toml::from_str(text).map_err(|source| RegistryError::Parse {
        file: file_name.to_string(),
        source,
    })?;
    to_tool_def(file_name, raw)
}

/// Convert and validate a parsed tool file.
fn to_tool_def(file_name: &str, raw: ToolFileToml) -> Result<ToolDef, RegistryError> {
    let roles = parse_roles(file_name, raw.roles)?;

    if roles.contains_key(&Role::Infrastructure) && raw.infra.is_none() {
        return Err(RegistryError::InfraConfigMissing {
            file: file_name.to_string(),
        });
    }

    // `[fullstack]` fuses on-chain + off-chain, so the tool must fill both.
    if raw.fullstack.is_some()
        && !(roles.contains_key(&Role::OnChain) && roles.contains_key(&Role::OffChain))
    {
        return Err(RegistryError::FullstackRolesMissing {
            file: file_name.to_string(),
        });
    }

    let compat = CompatConfig {
        consumes: parse_seams(file_name, &raw.compat.consumes)?,
        serves: parse_seams(file_name, &raw.compat.serves)?,
        self_contained_devnet: raw.compat.self_contained_devnet,
    };

    let ToolMetaToml {
        id,
        name,
        description,
        website,
        community,
        languages,
        system_deps,
        nix_packages,
        nix_self_contained,
        detect,
        experimental,
    } = raw.tool;

    Ok(ToolDef {
        id,
        name,
        description,
        website,
        community,
        languages,
        system_deps,
        nix_packages,
        nix_self_contained,
        detect: detect.into_iter().map(DetectSignature::from).collect(),
        roles,
        infra: raw.infra,
        experimental,
        fullstack: raw.fullstack,
        compat,
    })
}

fn parse_roles(
    file_name: &str,
    raw: HashMap<String, RoleConfig>,
) -> Result<HashMap<Role, RoleConfig>, RegistryError> {
    raw.into_iter()
        .map(|(role, cfg)| match Role::from_kebab(&role) {
            Ok(parsed) => Ok((parsed, cfg)),
            Err(source) => Err(RegistryError::UnknownRole {
                file: file_name.to_string(),
                role,
                source,
            }),
        })
        .collect()
}

fn parse_seams(file_name: &str, raw: &[String]) -> Result<Vec<Seam>, RegistryError> {
    raw.iter()
        .map(|s| {
            Seam::from_kebab(s).map_err(|source| RegistryError::UnknownSeam {
                file: file_name.to_string(),
                seam: s.clone(),
                source,
            })
        })
        .collect()
}

/// Immutable registry of all tool definitions, loaded from embedded TOML files.
pub struct Registry {
    tools: Vec<ToolDef>,
    by_id: HashMap<String, usize>,
    by_role: HashMap<Role, Vec<usize>>,
}

impl Registry {
    /// Load all tool definitions from the embedded `registry/tools/` directory.
    ///
    /// Fails on malformed TOML, unknown roles/seams, inconsistent tables,
    /// duplicate ids, or an empty registry.
    pub fn load() -> Result<Self, RegistryError> {
        let mut registry = Registry {
            tools: Vec::new(),
            by_id: HashMap::new(),
            by_role: HashMap::new(),
        };

        let tool_files = RegistryAssets::iter()
            .filter(|name| name.starts_with("tools/") && name.ends_with(".toml"));
        for file_name in tool_files {
            registry.insert(parse_tool_file(&file_name)?)?;
        }

        if registry.tools.is_empty() {
            return Err(RegistryError::Empty);
        }
        Ok(registry)
    }

    fn insert(&mut self, tool: ToolDef) -> Result<(), RegistryError> {
        if self.by_id.contains_key(&tool.id) {
            return Err(RegistryError::DuplicateId { id: tool.id });
        }

        let idx = self.tools.len();
        self.by_id.insert(tool.id.clone(), idx);
        for &role in tool.roles.keys() {
            self.by_role.entry(role).or_default().push(idx);
        }
        self.tools.push(tool);
        Ok(())
    }

    /// Look up a tool by its id.
    pub fn get(&self, id: &str) -> Option<&ToolDef> {
        self.by_id.get(id).map(|&idx| &self.tools[idx])
    }

    /// All tools that can fill the given role.
    pub fn tools_for_role(&self, role: Role) -> Vec<&ToolDef> {
        self.by_role
            .get(&role)
            .map(|indices| indices.iter().map(|&idx| &self.tools[idx]).collect())
            .unwrap_or_default()
    }

    /// All loaded tool definitions.
    pub fn all_tools(&self) -> &[ToolDef] {
        &self.tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> Registry {
        Registry::load().expect("registry should load successfully")
    }

    fn sorted_ids_for_role(reg: &Registry, role: Role) -> Vec<&str> {
        let mut ids: Vec<&str> = reg
            .tools_for_role(role)
            .iter()
            .map(|t| t.id.as_str())
            .collect();
        ids.sort_unstable();
        ids
    }

    /// Parse a demo tool file: a fixed `[tool]` header followed by `tables`.
    fn demo_tool(tables: &str) -> Result<ToolDef, RegistryError> {
        let toml = format!(
            r#"
            [tool]
            id = "demo"
            name = "Demo"
            description = "d"
            website = "https://x"
            languages = ["scala"]
            {tables}
            "#
        );
        let raw: ToolFileToml = toml::from_str(&toml).unwrap();
        to_tool_def("demo.toml", raw)
    }

    #[test]
    fn load_succeeds() {
        let _ = registry();
    }

    #[test]
    fn load_tool_count() {
        // aiken, plinth, scalus, meshjs, evolution, tx3, yaci, blaster + infra:
        // kupo, ogmios, dolos, tx-submit-api, cardano-node, cardano-node-api, dingo
        assert_eq!(registry().all_tools().len(), 15);
    }

    #[test]
    fn get_by_id() {
        let reg = registry();
        let aiken = reg.get("aiken").expect("aiken should exist");
        assert_eq!(aiken.name, "Aiken");
        assert_eq!(aiken.id, "aiken");
    }

    #[test]
    fn get_unknown_returns_none() {
        assert!(registry().get("nonexistent").is_none());
    }

    #[test]
    fn tools_for_role_on_chain() {
        assert_eq!(
            sorted_ids_for_role(&registry(), Role::OnChain),
            vec!["aiken", "plinth", "scalus"]
        );
    }

    #[test]
    fn tools_for_role_off_chain() {
        assert_eq!(
            sorted_ids_for_role(&registry(), Role::OffChain),
            vec!["evolution", "meshjs", "scalus", "tx3"]
        );
    }

    #[test]
    fn tools_for_role_devnet() {
        // Yaci DevKit is a local devnet, so it fills the devnet role (not infra).
        assert_eq!(sorted_ids_for_role(&registry(), Role::Devnet), vec!["yaci"]);
    }

    #[test]
    fn infrastructure_tools_present() {
        assert_eq!(
            sorted_ids_for_role(&registry(), Role::Infrastructure),
            vec![
                "cardano-node",
                "cardano-node-api",
                "dingo",
                "dolos",
                "kupo",
                "ogmios",
                "tx-submit-api",
            ]
        );
    }

    #[test]
    fn infra_tools_declare_infra_config() {
        let reg = registry();
        for tool in reg.tools_for_role(Role::Infrastructure) {
            let infra = tool
                .infra
                .as_ref()
                .unwrap_or_else(|| panic!("infra tool '{}' must declare [infra]", tool.id));
            assert!(
                !infra.cardano_up_package.is_empty(),
                "infra tool '{}' needs a cardano_up_package",
                tool.id
            );
            // All infra tools share the single aggregated driver template.
            assert_eq!(
                tool.roles.get(&Role::Infrastructure).unwrap().template,
                "_infra/cardano-up"
            );
        }
    }

    #[test]
    fn experimental_flag_defaults_false_and_marks_blaster() {
        let reg = registry();
        // Blaster is the current formal-methods experimental tool (work in progress).
        assert!(
            reg.get("blaster")
                .expect("blaster should exist")
                .experimental,
            "blaster should be marked experimental"
        );
        // Released tools default to non-experimental (no flag needed in their TOML).
        for id in ["aiken", "meshjs", "scalus", "yaci", "kupo"] {
            assert!(
                !reg.get(id).unwrap().experimental,
                "{id} should not be experimental by default"
            );
        }
    }

    #[test]
    fn experimental_defaults_false_when_flag_absent() {
        let tables = r#"
            [roles.on-chain]
            template = "demo/on-chain"
        "#;
        let def = demo_tool(tables).unwrap();
        assert!(!def.experimental);
    }

    #[test]
    fn scalus_multi_role() {
        let reg = registry();
        let scalus = reg.get("scalus").expect("scalus should exist");
        assert!(scalus.roles.contains_key(&Role::OnChain));
        assert!(scalus.roles.contains_key(&Role::OffChain));
        assert_eq!(scalus.roles.len(), 2);
    }

    #[test]
    fn scalus_declares_fullstack_template() {
        let reg = registry();
        let scalus = reg.get("scalus").expect("scalus should exist");
        let fs = scalus
            .fullstack
            .as_ref()
            .expect("scalus should declare a [fullstack] template");
        assert_eq!(fs.template, "scalus/fullstack");
        // Aiken (on-chain only) has no fullstack capability.
        assert!(reg.get("aiken").unwrap().fullstack.is_none());
    }

    #[test]
    fn fullstack_with_both_roles_loads() {
        let tables = r#"
            [roles.on-chain]
            template = "demo/on-chain"
            [roles.off-chain]
            template = "demo/off-chain"
            [fullstack]
            template = "demo/fullstack"
        "#;
        let def = demo_tool(tables).expect("valid fullstack tool");
        assert_eq!(def.fullstack.unwrap().template, "demo/fullstack");
    }

    #[test]
    fn fullstack_without_both_roles_is_rejected() {
        // [fullstack] but only on-chain declared → error (must fill both roles).
        let tables = r#"
            [roles.on-chain]
            template = "demo/on-chain"
            [fullstack]
            template = "demo/fullstack"
        "#;
        assert!(matches!(
            demo_tool(tables),
            Err(RegistryError::FullstackRolesMissing { .. })
        ));
    }

    #[test]
    fn all_fields_populated() {
        let reg = registry();
        for tool in reg.all_tools() {
            assert!(!tool.id.is_empty(), "id should not be empty");
            assert!(!tool.name.is_empty(), "name should not be empty");
            assert!(
                !tool.description.is_empty(),
                "description should not be empty"
            );
            assert!(!tool.website.is_empty(), "website should not be empty");
            for link in &tool.community {
                assert!(!link.name.is_empty(), "{}: unnamed community link", tool.id);
                assert!(
                    link.url.starts_with("https://"),
                    "{}: community link '{}' is not https",
                    tool.id,
                    link.name
                );
            }
            // Infra providers are cardano-up packages, not authored in a
            // language, so only they may leave `languages` empty.
            if !tool.roles.contains_key(&Role::Infrastructure) {
                assert!(!tool.languages.is_empty(), "languages should not be empty");
            }
            assert!(!tool.roles.is_empty(), "roles should not be empty");

            for (role, cfg) in &tool.roles {
                assert!(
                    !cfg.template.is_empty(),
                    "template for {} in role {} should not be empty",
                    tool.id,
                    role
                );
            }
        }
    }
}
