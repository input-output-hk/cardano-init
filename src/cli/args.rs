//! Command-line arguments (clap). The `///` docs on fields are the `--help` text.

use clap::{Parser, Subcommand};

use super::oneshot::RoleFlags;

/// Scaffold a new Cardano protocol project.
#[derive(Parser, Debug)]
#[command(name = "cardano-init", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Print machine-readable JSON instead of human-readable text (global)
    #[arg(long, global = true)]
    pub json: bool,

    #[command(flatten)]
    pub init: InitArgs,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Check that the dependencies this project needs are installed, and
    /// advise how to install any that are missing
    Doctor,

    /// List the available roles and tools (use --json for agents)
    List(ListArgs),

    /// Add or swap a tool/role in the project in the current directory
    Add(AddArgs),

    /// Remove a role (or an infrastructure provider) from the project in the
    /// current directory
    Remove(RemoveArgs),
}

/// Arguments for `list`.
#[derive(clap::Args, Debug, Default)]
pub struct ListArgs {
    /// Show a compact matrix of tools by role (human output only; `--json` is
    /// unaffected)
    #[arg(long)]
    pub table: bool,
}

/// Flags shared by the update commands (add / remove).
#[derive(clap::Args, Debug, Default, Clone)]
pub struct UpdateFlags {
    /// Show the change set without writing anything
    #[arg(long, visible_alias = "dryrun")]
    pub dry_run: bool,

    /// Update even when the git working tree has uncommitted changes (there is
    /// then no `git diff` to review/revert against)
    #[arg(long)]
    pub force: bool,

    /// Proceed even if the resulting off-chain ↔ provider selection is flagged
    /// incompatible (downgrades the stop to a warning)
    #[arg(long, visible_alias = "ignore-warnings")]
    pub ignore_warning: bool,

    /// Allow experimental tools in the resulting selection
    #[arg(long, short = 'e', visible_alias = "experimental")]
    pub allow_experimental: bool,
}

/// Arguments for `add`: the same role flags as init (presence assigns/replaces).
#[derive(clap::Args, Debug)]
pub struct AddArgs {
    /// On-chain tool (replaces the current on-chain tool if any)
    #[arg(long, visible_aliases = ["on", "onchain"], value_name = "TOOL_ID")]
    pub on_chain: Option<String>,

    /// Off-chain tool (replaces the current off-chain tool if any)
    #[arg(long, visible_aliases = ["off", "offchain"], value_name = "TOOL_ID")]
    pub off_chain: Option<String>,

    /// Fullstack tool filling both on-chain and off-chain as one `protocol/`
    /// component. Not combinable with --on-chain/--off-chain.
    #[arg(
        long,
        short = 'f',
        visible_short_alias = 'p',
        visible_alias = "protocol",
        value_name = "TOOL_ID"
    )]
    pub fullstack: Option<String>,

    /// Infrastructure provider to add (repeatable)
    #[arg(
        long,
        short = 'i',
        visible_alias = "infrastructure",
        value_name = "TOOL_ID"
    )]
    pub infra: Vec<String>,

    /// Devnet tool (replaces the current devnet tool if any)
    #[arg(long, short = 'd', value_name = "TOOL_ID")]
    pub devnet: Option<String>,

    /// Formal-methods tool (replaces the current one if any)
    #[arg(long, visible_aliases = ["formal", "formalmethods"], value_name = "TOOL_ID")]
    pub formal_methods: Option<String>,

    #[command(flatten)]
    pub flags: UpdateFlags,
}

impl AddArgs {
    /// The role flags, borrowed for validation.
    pub fn role_flags(&self) -> RoleFlags<'_> {
        RoleFlags {
            on_chain: self.on_chain.as_deref(),
            off_chain: self.off_chain.as_deref(),
            fullstack: self.fullstack.as_deref(),
            infra: &self.infra,
            devnet: self.devnet.as_deref(),
            formal_methods: self.formal_methods.as_deref(),
        }
    }
}

/// Arguments for `remove`: bare role flags drop that role; `--infra <id>` drops
/// one provider.
#[derive(clap::Args, Debug)]
pub struct RemoveArgs {
    /// Remove the on-chain component
    #[arg(long, visible_aliases = ["on", "onchain"])]
    pub on_chain: bool,

    /// Remove the off-chain component
    #[arg(long, visible_aliases = ["off", "offchain"])]
    pub off_chain: bool,

    /// Remove an infrastructure provider by id (repeatable)
    #[arg(
        long,
        short = 'i',
        visible_alias = "infrastructure",
        value_name = "TOOL_ID"
    )]
    pub infra: Vec<String>,

    /// Remove the devnet component
    #[arg(long, short = 'd')]
    pub devnet: bool,

    /// Remove the formal-methods component
    #[arg(long, visible_aliases = ["formal", "formalmethods"])]
    pub formal_methods: bool,

    #[command(flatten)]
    pub flags: UpdateFlags,
}

/// Arguments for the default init mode (interactive or one-shot).
#[derive(clap::Args, Debug, Default)]
pub struct InitArgs {
    /// Project name (required in one-shot mode)
    #[arg(long, short = 'n')]
    pub name: Option<String>,

    /// On-chain tool (e.g., aiken, scalus)
    #[arg(long, visible_aliases = ["on", "onchain"], value_name = "TOOL_ID")]
    pub on_chain: Option<String>,

    /// Off-chain tool (e.g., meshjs, scalus)
    #[arg(long, visible_aliases = ["off", "offchain"], value_name = "TOOL_ID")]
    pub off_chain: Option<String>,

    /// Fullstack tool for both on-chain and off-chain, as one `protocol/`
    /// component. Not combinable with --on-chain/--off-chain
    #[arg(
        long,
        short = 'f',
        visible_short_alias = 'p',
        visible_alias = "protocol",
        value_name = "TOOL_ID"
    )]
    pub fullstack: Option<String>,

    /// Infrastructure tool (repeatable: --infra kupo --infra ogmios)
    #[arg(
        long,
        short = 'i',
        visible_alias = "infrastructure",
        value_name = "TOOL_ID"
    )]
    pub infra: Vec<String>,

    /// Devnet tool (e.g., yaci)
    #[arg(long, short = 'd', value_name = "TOOL_ID")]
    pub devnet: Option<String>,

    /// Formal methods tool (e.g., blaster)
    #[arg(long, visible_aliases = ["formal", "formalmethods"], value_name = "TOOL_ID")]
    pub formal_methods: Option<String>,

    /// Generate Nix flake for dependency management
    #[arg(long)]
    pub nix: bool,

    /// Skip the `spec/` protocol-specification folder (generated by default)
    #[arg(long)]
    pub no_spec: bool,

    /// Opt in to experimental tools
    #[arg(long, short = 'e', visible_alias = "experimental")]
    pub allow_experimental: bool,

    /// Show what would be generated without writing to disk
    #[arg(long, visible_alias = "dryrun")]
    pub dry_run: bool,

    /// Scaffold a combination the compatibility check flags as incompatible
    #[arg(long, visible_alias = "ignore-warnings")]
    pub ignore_warning: bool,
}

impl InitArgs {
    /// Whether any flag that only makes sense in one-shot mode was given.
    pub fn has_oneshot_flags(&self) -> bool {
        self.on_chain.is_some()
            || self.off_chain.is_some()
            || self.fullstack.is_some()
            || !self.infra.is_empty()
            || self.devnet.is_some()
            || self.formal_methods.is_some()
            || self.nix
            || self.no_spec
            || self.dry_run
    }

    /// The role flags, borrowed for validation.
    pub fn role_flags(&self) -> RoleFlags<'_> {
        RoleFlags {
            on_chain: self.on_chain.as_deref(),
            off_chain: self.off_chain.as_deref(),
            fullstack: self.fullstack.as_deref(),
            infra: &self.infra,
            devnet: self.devnet.as_deref(),
            formal_methods: self.formal_methods.as_deref(),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("cardano-init").chain(args.iter().copied()))
            .unwrap_or_else(|e| panic!("{args:?} should parse: {e}"))
    }

    #[test]
    fn cli_definition_has_no_flag_conflicts() {
        Cli::command().debug_assert();
    }

    #[test]
    fn role_flag_aliases_parse() {
        for (on, off) in [("--on", "--off"), ("--onchain", "--offchain")] {
            let cli = parse(&[on, "aiken", off, "meshjs"]);
            assert_eq!(cli.init.on_chain.as_deref(), Some("aiken"));
            assert_eq!(cli.init.off_chain.as_deref(), Some("meshjs"));
        }
        for formal in ["--formal", "--formalmethods"] {
            let cli = parse(&[formal, "blaster"]);
            assert_eq!(cli.init.formal_methods.as_deref(), Some("blaster"));
        }
    }

    #[test]
    fn fullstack_short_flags_and_alias_parse() {
        for flag in ["-f", "-p", "--protocol", "--fullstack"] {
            let cli = parse(&[flag, "scalus"]);
            assert_eq!(cli.init.fullstack.as_deref(), Some("scalus"), "{flag}");
        }
    }

    #[test]
    fn init_short_flags_parse() {
        let cli = parse(&[
            "-n", "demo", "-i", "kupo", "-i", "ogmios", "-d", "yaci", "-e",
        ]);
        assert_eq!(cli.init.name.as_deref(), Some("demo"));
        assert_eq!(cli.init.infra, vec!["kupo", "ogmios"]);
        assert_eq!(cli.init.devnet.as_deref(), Some("yaci"));
        assert!(cli.init.allow_experimental);
    }

    #[test]
    fn add_and_remove_accept_aliases() {
        let Some(Command::Add(add)) = parse(&["add", "-p", "scalus", "--dryrun"]).command else {
            panic!("expected add");
        };
        assert_eq!(add.fullstack.as_deref(), Some("scalus"));
        assert!(add.flags.dry_run);

        let Some(Command::Remove(rm)) = parse(&["remove", "--on", "--formal"]).command else {
            panic!("expected remove");
        };
        assert!(rm.on_chain && rm.formal_methods);
        assert!(!rm.off_chain);
    }

    #[test]
    fn no_spec_flag_parses_and_counts_as_oneshot() {
        let cli = parse(&["--no-spec"]);
        assert!(cli.init.no_spec);
        // Like --nix, it only makes sense alongside --name.
        assert!(cli.init.has_oneshot_flags());

        assert!(!parse(&[]).init.no_spec);
    }
}
