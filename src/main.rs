mod cli;
mod contract;
mod doctor;
mod registry;
mod scaffold;

fn main() {
    reset_sigpipe();
    restore_cursor_on_sigint();
    std::process::exit(cli::run());
}

/// Show the terminal cursor again when Ctrl+C kills the process.
///
/// dialoguer prompts and indicatif spinners hide the cursor while they run.
/// On Ctrl+C, console restores the terminal mode and raises SIGINT, whose
/// default action kills the process before they can show it again, leaving
/// the user's shell without a cursor. The handler re-shows it, then re-raises
/// under the default action so the exit status still reports SIGINT.
#[cfg(unix)]
fn restore_cursor_on_sigint() {
    extern "C" fn on_sigint(_: libc::c_int) {
        // Show the cursor, and end the prompt's line so the shell starts clean.
        const SHOW_CURSOR: &[u8] = b"\x1b[?25h\n";
        // SAFETY: write(2), signal(2) and raise(3) are async-signal-safe.
        unsafe {
            libc::write(
                libc::STDERR_FILENO,
                SHOW_CURSOR.as_ptr().cast(),
                SHOW_CURSOR.len(),
            );
            libc::signal(libc::SIGINT, libc::SIG_DFL);
            libc::raise(libc::SIGINT);
        }
    }

    // Only a terminal has a cursor; don't write escapes into a redirected stderr.
    if !console::Term::stderr().is_term() {
        return;
    }
    // SAFETY: installs an async-signal-safe handler once at startup, before any
    // threads are spawned.
    unsafe {
        libc::signal(
            libc::SIGINT,
            on_sigint as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
    }
}

#[cfg(not(unix))]
fn restore_cursor_on_sigint() {}

/// Restore the default SIGPIPE disposition so piping into `head`/`less` exits
/// cleanly instead of panicking on EPIPE (Rust ignores SIGPIPE by default).
#[cfg(unix)]
fn reset_sigpipe() {
    // SAFETY: resetting a signal to SIG_DFL is async-signal-safe and this runs
    // once at startup, before any threads are spawned.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}
