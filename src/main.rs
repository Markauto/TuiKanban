mod cli;
mod completion;
mod model;
mod recent;
mod store;
mod tui;

use clap::Parser;
fn main() {
    if let Err(error) = cli::run(cli::Cli::parse()) {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}
