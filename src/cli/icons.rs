//! Whether to show tool icons this run. Icons are Nerd Font glyphs, and no
//! terminal can be asked whether it draws them, so `auto` only switches them on
//! in terminals that ship Nerd Font symbols themselves. The last `--icons`
//! value given is remembered in a small state file, so a choice sticks without
//! a config file. Failures to read or write that file are ignored.

use std::path::PathBuf;

/// The `--icons` modes. Plain `//` comments on the variants: doc comments
/// would become per-value help, which switches clap's whole `--help` to its
/// long, multi-line layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum IconsMode {
    // On in terminals known to ship Nerd Font symbols (Ghostty, WezTerm,
    // kitty), when output goes to the terminal.
    Auto,
    // Always on, even when output is piped.
    Always,
    // Always off.
    Never,
}

impl IconsMode {
    /// Parse a mode from the env var or the state file. Besides the mode
    /// names, accepts the usual boolean spellings (`1`/`true`/`yes`/`on` →
    /// `always`, `0`/`false`/`no`/`off` → `never`).
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Self::Auto),
            "always" | "1" | "true" | "yes" | "on" => Some(Self::Always),
            "never" | "0" | "false" | "no" | "off" => Some(Self::Never),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        }
    }
}

/// The env var that sets the mode when no flag is given.
pub const ENV_VAR: &str = "CARDANO_INIT_ICONS";

/// Decide whether icons show this run, and remember an explicit `--icons`
/// choice for later runs.
pub fn init(flag: Option<IconsMode>, json: bool) -> bool {
    let saved = read_saved();
    if let Some(mode) = flag
        && saved != Some(mode)
    {
        save(mode);
    }
    let env = std::env::var(ENV_VAR).ok();
    let mode = resolve(flag, env.as_deref(), saved);
    let is_tty = console::Term::stdout().is_term();
    enabled(mode, json, is_tty, |k| std::env::var(k).ok())
}

/// Which mode applies: the flag, then the env var, then the remembered choice,
/// then `auto`. An unrecognized env value is ignored.
fn resolve(flag: Option<IconsMode>, env: Option<&str>, saved: Option<IconsMode>) -> IconsMode {
    flag.or_else(|| env.and_then(IconsMode::parse))
        .or(saved)
        .unwrap_or(IconsMode::Auto)
}

/// Whether `mode` shows icons. JSON never carries them.
fn enabled(
    mode: IconsMode,
    json: bool,
    is_tty: bool,
    env: impl Fn(&str) -> Option<String>,
) -> bool {
    if json {
        return false;
    }
    match mode {
        IconsMode::Always => true,
        IconsMode::Never => false,
        IconsMode::Auto => is_tty && terminal_ships_nerd_symbols(env),
    }
}

/// Ghostty, WezTerm and kitty (0.36+) draw Nerd Font symbols without a patched
/// font. Multiplexers (tmux, zellij) replace `TERM_PROGRAM`, so inside one the
/// terminal can't be recognized and this is `false`.
fn terminal_ships_nerd_symbols(env: impl Fn(&str) -> Option<String>) -> bool {
    let program = env("TERM_PROGRAM").unwrap_or_default();
    let term = env("TERM").unwrap_or_default();
    program.eq_ignore_ascii_case("ghostty")
        || program.eq_ignore_ascii_case("wezterm")
        || term == "xterm-ghostty"
        || term == "xterm-kitty"
        || env("KITTY_WINDOW_ID").is_some()
}

fn read_saved() -> Option<IconsMode> {
    let text = std::fs::read_to_string(path()?).ok()?;
    IconsMode::parse(&text)
}

fn save(mode: IconsMode) {
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, mode.as_str());
}

/// `<state dir>/cardano-init/icons`: `$XDG_STATE_HOME|~/.local/state` on Linux;
/// the local data dir elsewhere (`~/Library/Application Support` on macOS,
/// `%LOCALAPPDATA%` on Windows).
fn path() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|d| d.join("cardano-init").join("icons"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use IconsMode::*;

    fn env_of<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |k| {
            vars.iter()
                .find(|(name, _)| *name == k)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn parse_accepts_modes_and_booleans() {
        assert_eq!(IconsMode::parse("auto"), Some(Auto));
        assert_eq!(IconsMode::parse(" Always\n"), Some(Always));
        assert_eq!(IconsMode::parse("1"), Some(Always));
        assert_eq!(IconsMode::parse("off"), Some(Never));
        assert_eq!(IconsMode::parse("maybe"), None);
    }

    #[test]
    fn resolve_prefers_flag_then_env_then_saved() {
        assert_eq!(resolve(Some(Never), Some("1"), Some(Always)), Never);
        assert_eq!(resolve(None, Some("1"), Some(Never)), Always);
        assert_eq!(resolve(None, Some("garbage"), Some(Never)), Never);
        assert_eq!(resolve(None, None, Some(Always)), Always);
        assert_eq!(resolve(None, None, None), Auto);
    }

    #[test]
    fn auto_needs_a_known_terminal_and_a_tty() {
        let ghostty = [("TERM_PROGRAM", "ghostty")];
        assert!(enabled(Auto, false, true, env_of(&ghostty)));
        assert!(!enabled(Auto, false, false, env_of(&ghostty)));
        assert!(enabled(
            Auto,
            false,
            true,
            env_of(&[("TERM_PROGRAM", "WezTerm")])
        ));
        assert!(enabled(
            Auto,
            false,
            true,
            env_of(&[("TERM", "xterm-kitty")])
        ));
        assert!(!enabled(
            Auto,
            false,
            true,
            env_of(&[("TERM_PROGRAM", "tmux")])
        ));
        assert!(!enabled(Auto, false, true, env_of(&[])));
    }

    #[test]
    fn always_and_never_ignore_the_terminal_but_json_wins() {
        assert!(enabled(Always, false, false, env_of(&[])));
        assert!(!enabled(
            Never,
            false,
            true,
            env_of(&[("TERM_PROGRAM", "ghostty")])
        ));
        assert!(!enabled(Always, true, true, env_of(&[])));
    }
}
