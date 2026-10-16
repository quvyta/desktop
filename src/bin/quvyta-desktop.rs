//! The `quvyta-desktop` command, the long name of `qdesk`: `--help`, `--version`, `--list`, `wallpaper` and `serve` answer and exit; no argument, or `--new`, opens the desktop.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = qdesk::cli::run(&args, &mut std::io::stdout(), &mut std::io::stderr()) {
        return ExitCode::from(code);
    }
    match qdesk::run_with(qdesk::cli::choice(&args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
