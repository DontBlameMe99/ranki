//! Headless benchmark harness for every Ranki surface.

mod measure;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use measure::BenchError;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("ranki-bench: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), BenchError> {
    let mut args = env::args_os().skip(1);
    let command = args.next().ok_or_else(|| {
        BenchError::Message(
            "usage: ranki-bench measure --out REPORT.json".to_owned(),
        )
    })?;
    if command != "measure" {
        return Err(BenchError::Message(format!(
            "unknown command: {}",
            command.to_string_lossy()
        )));
    }
    let mut output: Option<PathBuf> = None;
    while let Some(argument) = args.next() {
        if argument != "--out" {
            return Err(BenchError::Message(format!(
                "unknown argument: {}",
                argument.to_string_lossy()
            )));
        }
        let value = args.next().ok_or_else(|| {
            BenchError::Message("--out needs a path".to_owned())
        })?;
        output = Some(PathBuf::from(value));
    }
    let path = output
        .ok_or_else(|| BenchError::Message("missing --out".to_owned()))?;
    let report = measure::run()?;
    let text = serde_json::to_string_pretty(&report)?;
    fs::write(&path, format!("{text}\n"))?;
    println!(
        "Wrote {} workloads to {}",
        report.results.len(),
        path.display()
    );
    Ok(())
}
