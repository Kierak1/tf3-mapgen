use anyhow::Result;
use clap::Parser;

use tf3_mapgen::cli::Cli;
use tf3_mapgen::{generate, summary_text, userdata, StderrProgress};

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    if cli.print_userdata {
        match userdata::find_userdata() {
            Some(path) => {
                println!("{}", path.display());
                return Ok(());
            }
            None => {
                eprintln!("could not detect the Transport Fever 3 userdata folder");
                std::process::exit(2);
            }
        }
    }
    let options = cli.into_options();
    let summary = generate(&options, &StderrProgress)?;
    println!();
    print!("{}", summary_text(&summary));
    Ok(())
}
