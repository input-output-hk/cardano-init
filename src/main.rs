mod cli;
mod contract;
mod doctor;
mod registry;
mod scaffold;
mod web;

fn main() {
    std::process::exit(cli::run());
}
