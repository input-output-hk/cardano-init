//! The CLI's visual vocabulary: status glyphs, semantic text styles, badges and
//! layout constants. Presenters style through these helpers, never through
//! `console::style` directly, so commands can't drift apart.
//!
//! `console` already drops ANSI off-TTY and under `NO_COLOR`/`CLICOLOR`/
//! `TERM=dumb`, and [`console::Emoji`] falls back to ASCII where unicode can't
//! render, so no TTY logic is needed here.

use console::{Emoji, StyledObject, style};

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
