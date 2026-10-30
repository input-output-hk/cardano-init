//! Layout primitives shared by the presenters: framed panels, section rules,
//! aligned tables and word wrapping. Widths are measured with ANSI stripped, so
//! styled content still lines up.

use comfy_table::{ContentArrangement, Table, presets};

use crate::cli::theme;

/// A borderless table with content-fit columns, for aligned sections. Cells
/// carry their own styling (`console`), which comfy-table measures correctly.
pub(super) fn borderless_table() -> Table {
    let mut table = Table::new();
    table.load_preset(presets::NOTHING);
    table.set_content_arrangement(ContentArrangement::Dynamic);
    table
}

/// Print `table` with each row prefixed by `indent`. Left cell padding is
/// zeroed so the first column sits flush at the indent.
pub(super) fn print_table(mut table: Table, indent: &str) {
    for column in table.column_iter_mut() {
        column.set_padding((0, 2));
    }
    for line in table.to_string().lines() {
        println!("{}{}", indent, line.trim_end());
    }
}

/// Print a rounded, cyan-framed panel (see [`panel_lines`]) to stdout.
pub(super) fn print_panel(
    title: &str,
    badge: Option<console::StyledObject<String>>,
    rows: &[String],
) {
    for line in panel_lines(title, badge, rows) {
        println!("{line}");
    }
}

/// The lines of a rounded, cyan-framed panel wrapping `rows`, with `title`
/// centered in the top border and an optional `badge` on its own line above
/// the frame. Returned rather than printed so errors can go to stderr.
pub(super) fn panel_lines(
    title: &str,
    badge: Option<console::StyledObject<String>>,
    rows: &[String],
) -> Vec<String> {
    let title_w = console::measure_text_width(title);
    let interior = rows
        .iter()
        .map(|r| console::measure_text_width(r))
        .chain(std::iter::once(title_w))
        .max()
        .unwrap_or(0);
    // Run length between the corner glyphs: 2 left margin + interior + 1 right.
    let span = interior + 3;

    let mut out = Vec::new();
    if let Some(badge) = badge {
        out.push(format!("{}{}", theme::PAD, badge));
    }

    // Dashes on both sides of " title ".
    let dashes = span - 2 - title_w;
    let left = dashes / 2;
    let right = dashes - left;
    out.push(format!(
        "{}{}{} {} {}{}",
        theme::PAD,
        theme::accent("╭"),
        theme::accent("─".repeat(left)),
        theme::accent_strong(title),
        theme::accent("─".repeat(right)),
        theme::accent("╮"),
    ));
    for row in rows {
        let gap = interior - console::measure_text_width(row);
        out.push(format!(
            "{}{}  {}{} {}",
            theme::PAD,
            theme::accent("│"),
            row,
            " ".repeat(gap),
            theme::accent("│"),
        ));
    }
    out.push(format!(
        "{}{}",
        theme::PAD,
        theme::accent(format!("╰{}╯", "─".repeat(span))),
    ));
    out
}

/// Print a bold section label followed by a dim horizontal rule.
pub(super) fn print_rule(label: &str) {
    const WIDTH: usize = 44;
    let dashes = WIDTH.saturating_sub(console::measure_text_width(label) + 1);
    println!(
        "{}{} {}",
        theme::PAD,
        theme::strong(label),
        theme::dim("─".repeat(dashes)),
    );
}

/// Wrap `text` to `width` columns on word boundaries.
pub(super) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_on_word_boundaries() {
        assert_eq!(wrap("aa bb cc", 5), vec!["aa bb", "cc"]);
        assert!(wrap("", 10).is_empty());
    }
}
