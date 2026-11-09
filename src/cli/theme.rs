//! The CLI's visual vocabulary: status glyphs, semantic text styles, badges and
//! layout constants. Presenters style through these helpers, never through
//! `console::style` directly, so commands can't drift apart.
//!
//! `console` already drops ANSI off-TTY and under `NO_COLOR`/`CLICOLOR`/
//! `TERM=dumb`, and [`console::Emoji`] falls back to ASCII where unicode can't
//! render, so no TTY logic is needed here.

use std::sync::atomic::{AtomicBool, Ordering};

use console::{Emoji, StyledObject, style};

use crate::registry::types::{Role, ToolDef};

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// The standard two-space indent every block of human output is offset by.
pub const PAD: &str = "  ";

// ---------------------------------------------------------------------------
// Status glyphs (unicode with an ASCII fallback for dumb terminals)
// ---------------------------------------------------------------------------

const OK: Emoji<'static, 'static> = Emoji("✔", "√");
const FAIL: Emoji<'static, 'static> = Emoji("✘", "x");
const WARNING: Emoji<'static, 'static> = Emoji("⚠", "!");
const UNKNOWN_MARK: Emoji<'static, 'static> = Emoji("?", "?");

/// Green ✔ — a satisfied check (created, scaffolded, dependency present).
pub fn ok() -> StyledObject<Emoji<'static, 'static>> {
    style(OK).green().bold()
}

/// Yellow ✘ — a needs-attention check (an absent required dependency). Yellow,
/// not red: a missing dep is an actionable warning, not a failure.
pub fn caution() -> StyledObject<Emoji<'static, 'static>> {
    style(FAIL).yellow().bold()
}

/// Yellow ⚠ — a warning marker (experimental / incompatible-but-forced).
pub fn warn_mark() -> StyledObject<Emoji<'static, 'static>> {
    style(WARNING).yellow().bold()
}

/// Yellow ? — an unrecognized / indeterminate component.
pub fn unknown() -> StyledObject<Emoji<'static, 'static>> {
    style(UNKNOWN_MARK).yellow().bold()
}

// ---------------------------------------------------------------------------
// Text roles
// ---------------------------------------------------------------------------

/// Emphasized plain text (labels like `Next steps:`, counts).
pub fn strong<D>(val: D) -> StyledObject<D> {
    style(val).bold()
}

/// A path, tool name, or other salient value.
pub fn accent<D>(val: D) -> StyledObject<D> {
    style(val).cyan()
}

/// A salient value that also carries emphasis (welcome role labels, the
/// created project name).
pub fn accent_strong<D>(val: D) -> StyledObject<D> {
    style(val).cyan().bold()
}

/// A shell command the user can copy and run.
pub fn command<D>(val: D) -> StyledObject<D> {
    style(val).cyan()
}

/// A positive value (`yes`, "all present").
pub fn good<D>(val: D) -> StyledObject<D> {
    style(val).green()
}

/// Warning-level body text.
pub fn warn<D>(val: D) -> StyledObject<D> {
    style(val).yellow()
}

/// A warning heading / emphasized warning fragment.
pub fn warn_strong<D>(val: D) -> StyledObject<D> {
    style(val).yellow().bold()
}

/// Secondary / de-emphasized text (field labels, hints, tree connectors).
pub fn dim<D>(val: D) -> StyledObject<D> {
    style(val).dim()
}

/// An underlined link (docs URLs).
pub fn link<D>(val: D) -> StyledObject<D> {
    style(val).underlined()
}

// ---------------------------------------------------------------------------
// Badges (filled pills — a single splash of background, used sparingly)
// ---------------------------------------------------------------------------

/// A success badge: `text` on a solid green fill (e.g. ` CREATED `). Without
/// color the padded label stays legible.
pub fn badge_ok(text: &str) -> StyledObject<String> {
    style(format!(" {text} ")).black().on_green().bold()
}

/// A caution badge: `text` on a solid yellow fill (e.g. ` DEPS MISSING `).
pub fn badge_warn(text: &str) -> StyledObject<String> {
    style(format!(" {text} ")).black().on_yellow().bold()
}

/// An error badge: `text` on a solid red fill (e.g. ` ERR (unknown_tool) `).
pub fn badge_err(text: &str) -> StyledObject<String> {
    style(format!(" {text} ")).black().on_red().bold()
}

// ---------------------------------------------------------------------------
// Tool icons (opt-in Nerd Font glyphs; they need a Nerd Font in the terminal,
// which can't be detected, so they show only when asked for)
// ---------------------------------------------------------------------------

static ICONS: AtomicBool = AtomicBool::new(false);

/// Turn tool icons on or off for this run (set once, from `--icons`).
pub fn set_icons(on: bool) {
    ICONS.store(on, Ordering::Relaxed);
}

/// Whether tool icons are on for this run.
pub fn icons_enabled() -> bool {
    ICONS.load(Ordering::Relaxed)
}

/// `icon_cell(tool)` when icons are on, else `""`: the prefix to put before a
/// tool's name wherever one is shown.
pub fn icon_prefix(tool: &ToolDef) -> String {
    if icons_enabled() {
        icon_cell(tool)
    } else {
        String::new()
    }
}

/// [`icon_prefix`] for a tool that may not be in the registry: unknown tools
/// get blanks, so names still line up.
pub fn icon_prefix_or_blank(tool: Option<&ToolDef>) -> String {
    match tool {
        Some(tool) => icon_prefix(tool),
        None if icons_enabled() => "  ".to_string(),
        None => String::new(),
    }
}

/// `"<icon> "`, or two spaces when the tool has none, so names stay aligned.
pub fn icon_cell(tool: &ToolDef) -> String {
    tool_icon(tool).map_or_else(|| "  ".to_string(), |icon| format!("{icon} "))
}

/// A tool's icon: a terminal for tools you run (those that only fill the
/// infrastructure or devnet roles), else its primary language's icon.
pub fn tool_icon(tool: &ToolDef) -> Option<StyledObject<&'static str>> {
    let runs = tool
        .roles
        .keys()
        .all(|r| matches!(r, Role::Infrastructure | Role::Devnet));
    if runs {
        return Some(terminal_icon());
    }
    tool.languages.first().and_then(|l| language_icon(l))
}

/// `nf-fa-terminal` in green: a tool you run rather than write code against.
pub fn terminal_icon() -> StyledObject<&'static str> {
    style("\u{f120}").green()
}

/// The icon for a registry language id in its brand color (nearest 256-color
/// match), if there is one. Aiken uses the closest glyph to its connected-dots
/// logo, and Pebble an outlined oval for a pebble. Tx3 and Lean have no Nerd Font glyph, so they borrow a character
/// from their logos that any font can draw: Tx3's pink `3`, and Lean's `∀`
/// (from its `L∃∀N` wordmark) for Blaster's Lean-based specs.
pub fn language_icon(language: &str) -> Option<StyledObject<&'static str>> {
    let (glyph, color) = match language {
        "aiken" => ("\u{f1e0}", 141),     // nf-fa-share_nodes, light purple
        "tx3" => ("3", 205),              // Tx3 wordmark, pink
        "haskell" => ("\u{e61f}", 97),    // nf-seti-haskell, purple
        "scala" => ("\u{e68e}", 160),     // nf-seti-scala, red
        "typescript" => ("\u{e628}", 32), // nf-seti-typescript, blue
        "rust" => ("\u{e68b}", 173),      // nf-seti-rust, rust orange
        "go" => ("\u{e627}", 38),         // nf-seti-go, gopher cyan
        "python" => ("\u{e606}", 220),    // nf-seti-python, yellow
        "pebble" => ("\u{f0ea1}", 248),   // nf-md-ellipse_outline, stone gray
        "blaster-spec" => ("∀", 75),      // Lean, light blue
        _ => return None,
    };
    Some(style(glyph).color256(color))
}
