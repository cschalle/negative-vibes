mod probe;
mod render;
mod scan;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "nv-cli", about = "Negative Vibes headless scanner tool")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List all SANE devices and their capabilities.
    Probe(probe::ProbeArgs),
    /// Scan a single frame and write raw PNM to disk.
    Scan(scan::ScanArgs),
    /// Run a raw PNM file through the nv-process pipeline and save a rendered PPM.
    Render(render::RenderArgs),
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Cmd::Probe(args)  => probe::run(args),
        Cmd::Scan(args)   => scan::run(args),
        Cmd::Render(args) => render::run(args),
    };
    if let Err(e) = result {
        eprintln!("nv-cli: {e}");
        std::process::exit(1);
    }
}
