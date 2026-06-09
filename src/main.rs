mod cli;
mod contract;
mod registry;
mod scaffold;
mod web;

fn main() {
    if let Err(e) = cli::run() {
        let code = e.exit_code();
        // code 0 = interactive abort by choice: exit cleanly, no error message.
        if code != 0 {
            eprintln!("{}: {}", console::style("error").red().bold(), e);
        }
        std::process::exit(code);
    }
}
