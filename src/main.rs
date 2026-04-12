#![allow(dead_code)]

mod cli;
mod diff;
mod output;
mod parse;
mod vcs;

use anyhow::Result;
use clap::Parser;

fn main() -> Result<()> {
    let _cli = cli::Cli::parse();
    Ok(())
}
