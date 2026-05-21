//! Entry-point for the kaleidoscope example compiler

// We use pliron-llvm in this test, which is not supported in wasm.
#![cfg(not(target_family = "wasm"))]

mod ast;
mod dialect;
mod from_ast;
mod jit;
mod to_llvm;

use std::{path::PathBuf, process::ExitCode};

use clap::Parser;

#[derive(Parser)]
#[command(version, about = "Kaleidoscope JIT example", long_about = None)]
struct Cli {
    /// Input Kaleidoscope source file
    #[arg(long = "input", value_name = "FILE")]
    input: PathBuf,

    /// Function name to execute from the source module
    #[arg(long = "fn", default_value = "main")]
    function: String,

    /// Integer argument passed to the JIT function
    #[arg(long = "arg", short = 'a')]
    arg: i64,
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let src = std::fs::read_to_string(&cli.input)?;
    let result = jit::exec_fn(&src, &cli.function, cli.arg)?;
    println!("JIT result ({}({})): {}", cli.function, cli.arg, result);
    Ok(())
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
