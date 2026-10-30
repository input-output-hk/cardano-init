//! The pre-generation "newer version available" banner (tech spec §10).

use crate::cli::theme;
use crate::cli::version_check::{RELEASES_URL, UpdateNotice};

/// Print the update banner: a yellow box with the update command for this
/// install (or the release link when the install method is unknown).
pub fn print_update_notice(notice: &UpdateNotice) {
    let width = usize::from(console::Term::stdout().size().1);
    println!();
    for line in update_notice_lines(notice, width) {
        println!("{line}");
    }
}

/// The banner lines for a terminal `term_width` columns wide. When the box
/// would wrap, the rows are printed unframed so the command stays
/// copy-pasteable.
fn update_notice_lines(notice: &UpdateNotice, term_width: usize) -> Vec<String> {
    let mut rows = vec![
        format!(
            "{} {}",
            theme::warn_strong("A new version of cardano-init is available!"),
            theme::dim(format!("({} → {})", notice.current, notice.latest)),
        ),
        String::new(),
    ];
    if let Some(cmd) = notice.command() {
        rows.push("To update to the latest version, run:".to_string());
        rows.push(format!("  {}", theme::command(cmd)));
    } else {
        rows.push("Download the latest version from:".to_string());
        rows.push(format!("  {}", theme::link(RELEASES_URL)));
    }

    let interior = rows
        .iter()
        .map(|r| console::measure_text_width(r))
        .max()
        .unwrap_or(0);
    if theme::PAD.len() + interior + 4 > term_width {
        return rows
            .into_iter()
            .map(|r| format!("{}{}", theme::PAD, r))
            .collect();
    }

    let rule = "─".repeat(interior + 2);
    let mut out = vec![format!(
        "{}{}",
        theme::PAD,
        theme::warn(format!("┌{rule}┐"))
    )];
    for row in &rows {
        let gap = interior - console::measure_text_width(row);
        out.push(format!(
            "{}{} {}{} {}",
            theme::PAD,
            theme::warn("│"),
            row,
            " ".repeat(gap),
            theme::warn("│"),
        ));
    }
    out.push(format!(
        "{}{}",
        theme::PAD,
        theme::warn(format!("└{rule}┘"))
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::version_check::InstallMethod;

    fn notice(install: InstallMethod) -> UpdateNotice {
        UpdateNotice {
            current: "0.2.1".into(),
            latest: "0.3.0".into(),
            install,
        }
    }

    #[test]
    fn update_notice_box_lines_align() {
        let lines = update_notice_lines(&notice(InstallMethod::Npm), 200);
        let widths: Vec<usize> = lines
            .iter()
            .map(|l| console::measure_text_width(l))
            .collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "{widths:?}");
        let text = console::strip_ansi_codes(&lines.join("\n")).into_owned();
        assert!(text.contains("0.2.1 → 0.3.0"));
        assert!(text.contains("npm install -g cardano-init@latest"));
        assert!(text.starts_with("  ┌") && text.ends_with('┘'));
    }

    #[test]
    fn update_notice_falls_back_to_release_link() {
        let lines = update_notice_lines(&notice(InstallMethod::Unknown), 200);
        let text = console::strip_ansi_codes(&lines.join("\n")).into_owned();
        assert!(text.contains(RELEASES_URL));
    }

    #[test]
    fn update_notice_unframed_when_too_narrow() {
        let lines = update_notice_lines(&notice(InstallMethod::ShellInstaller), 80);
        let text = console::strip_ansi_codes(&lines.join("\n")).into_owned();
        assert!(!text.contains('┌'));
        assert!(text.contains("cardano-init-installer.sh | sh"));
    }
}
