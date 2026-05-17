use std::{env, path::PathBuf, process};

use noctrail_term::recording::replay_recording_file;

const HELP: &str = "\
Noctrail development CLI

Usage:
  noctrail [command]

Commands:
  doctor      Print basic environment diagnostics
  replay      Replay one or more terminal recording fixtures
  help        Print this help text

Options:
  -h, --help     Print this help text
  -V, --version  Print version information
";

fn main() {
    let mut args = env::args().skip(1);

    match args.next().as_deref() {
        None | Some("help" | "-h" | "--help") => print!("{HELP}"),
        Some("-V" | "--version") => println!("noctrail {}", env!("CARGO_PKG_VERSION")),
        Some("doctor") => print_doctor(),
        Some("replay") => {
            let patterns: Vec<String> = args.collect();
            if patterns.is_empty() {
                eprintln!("replay requires at least one fixture path or glob");
                process::exit(2);
            }
            if let Err(error) = replay_fixtures(&patterns) {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        Some(command) => {
            eprintln!("unknown command: {command}");
            eprintln!("run `noctrail help` for usage");
            process::exit(2);
        }
    }
}

fn print_doctor() {
    println!("noctrail {}", env!("CARGO_PKG_VERSION"));
    println!("target: {}", env::consts::OS);
    println!("arch: {}", env::consts::ARCH);
}

fn replay_fixtures(patterns: &[String]) -> Result<(), String> {
    let mut paths = Vec::new();
    for pattern in patterns {
        if contains_glob_meta(pattern) {
            let entries = glob::glob(pattern)
                .map_err(|error| format!("failed to parse glob pattern {pattern:?}: {error}"))?;
            for entry in entries {
                let path = entry.map_err(|error| format!("failed to read glob entry: {error}"))?;
                paths.push(path);
            }
        } else {
            paths.push(PathBuf::from(pattern));
        }
    }

    if paths.is_empty() {
        return Err("no fixtures matched the provided patterns".to_string());
    }

    paths.sort();
    paths.dedup();

    for path in paths {
        replay_recording_file(&path).map_err(|error| error.to_string())?;
        println!("replayed {}", path.display());
    }

    Ok(())
}

fn contains_glob_meta(pattern: &str) -> bool {
    pattern.chars().any(|ch| matches!(ch, '*' | '?' | '['))
}
