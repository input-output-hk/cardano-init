mod cli;
mod contract;
mod doctor;
mod registry;
mod scaffold;

fn main() {
    reset_sigpipe();
    std::process::exit(cli::run());
}

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
