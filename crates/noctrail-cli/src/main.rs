use std::{env, path::PathBuf, process};

use noctrail_pty::{PtyCommand, PtySession, PtySize, ResolvedShell};
use noctrail_render::{RenderBackend, RenderPlan, RenderRect};
use noctrail_term::recording::replay_recording_file;
use noctrail_term::{Cell, Color, Cursor, ScreenRowSnapshot, Style, TerminalSnapshot};

const HELP: &str = "\
Noctrail development CLI

Usage:
  noctrail [command]

Commands:
  doctor      Print basic environment diagnostics
  doctor shell  Print shell resolution diagnostics
  replay      Replay one or more terminal recording fixtures
  render-smoke Run the render smoke check
  pty-smoke   Run the PTY smoke check
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
        Some("doctor") => {
            let topic = args.next();
            match topic.as_deref() {
                None => print_doctor(),
                Some("shell") => print_doctor_shell(),
                Some(other) => {
                    eprintln!("unknown doctor topic: {other}");
                    process::exit(2);
                }
            }
        }
        Some("render-smoke") => {
            if let Err(error) = run_render_smoke() {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        Some("pty-smoke") => {
            if let Err(error) = run_pty_smoke() {
                eprintln!("{error}");
                process::exit(1);
            }
        }
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
    println!("hint: run `noctrail doctor shell` for shell diagnostics");
}

fn print_doctor_shell() {
    let shell = ResolvedShell::detect();

    println!(
        "shell.program={}",
        shell.command().program().to_string_lossy()
    );
    if shell.command().argv().is_empty() {
        println!("shell.argv=(none)");
    } else {
        let args = shell
            .command()
            .argv()
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        println!("shell.argv={args}");
    }

    match shell.cwd() {
        Some(cwd) => println!("shell.cwd={}", cwd.display()),
        None => println!("shell.cwd=(unavailable)"),
    }

    println!("shell.source={}", shell.source().label());
    println!(
        "shell.env_mode={}",
        if shell.inherits_env() {
            "inherit"
        } else {
            "clear"
        }
    );

    if shell.env_overrides().is_empty() {
        println!("shell.env_overrides=(none)");
    } else {
        for (key, value) in shell.env_overrides() {
            println!(
                "shell.env_override.{}={}",
                key.to_string_lossy(),
                value.to_string_lossy()
            );
        }
    }
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

fn run_render_smoke() -> Result<(), String> {
    let snapshot = TerminalSnapshot {
        rows: vec![ScreenRowSnapshot {
            cells: vec![
                Cell {
                    text: "A".to_string(),
                    style: Style {
                        foreground: Color::Indexed(2),
                        background: Color::Default,
                        bold: true,
                        italic: false,
                        underline: false,
                    },
                    wide_continuation: false,
                },
                Cell {
                    text: "界".to_string(),
                    style: Style::default(),
                    wide_continuation: false,
                },
                Cell::wide_continuation(Style::default()),
            ],
            wrapped: false,
        }],
        cursor: Cursor { row: 0, col: 2 },
        ..TerminalSnapshot::default()
    };

    let plan = RenderPlan::from_terminal(
        RenderRect::new(0, 0, 96, 32),
        RenderBackend::Software,
        &snapshot,
    );

    if plan.rows.len() != 1 {
        return Err(format!(
            "render smoke expected 1 row, got {}",
            plan.rows.len()
        ));
    }

    let glyphs = &plan.rows[0].glyphs;
    if glyphs.len() != 3 {
        return Err(format!(
            "render smoke expected 3 glyph entries, got {}",
            glyphs.len()
        ));
    }

    if glyphs[0].text != "A" || !glyphs[0].style.bold {
        return Err("render smoke did not preserve ASCII glyph style".to_string());
    }

    if glyphs[1].text != "界" || glyphs[1].span != 2 {
        return Err("render smoke did not preserve wide glyph metadata".to_string());
    }

    if !glyphs[2].wide_continuation || glyphs[2].span != 0 {
        return Err("render smoke did not preserve wide continuation cell".to_string());
    }

    println!("render smoke ok");
    Ok(())
}

fn run_pty_smoke() -> Result<(), String> {
    let command = pty_smoke_command();
    let mut session = PtySession::spawn(command, PtySize::new(80, 24))
        .map_err(|error| format!("failed to spawn PTY smoke command: {error}"))?;
    let output = read_all_output(&mut session)
        .map_err(|error| format!("failed to read PTY smoke output: {error}"))?;
    let _ = session.close();

    let haystack = String::from_utf8_lossy(&output);
    if !haystack.contains("NOCTRAIL_PTY_SMOKE") {
        return Err(format!(
            "PTY smoke output did not contain sentinel; output was {:?}",
            haystack
        ));
    }

    println!("pty smoke ok");
    Ok(())
}

fn pty_smoke_command() -> PtyCommand {
    #[cfg(windows)]
    {
        let program = env::var_os("COMSPEC").unwrap_or_else(|| std::ffi::OsString::from("cmd.exe"));
        let mut command = PtyCommand::new(program);
        command.args(["/C", "echo", "NOCTRAIL_PTY_SMOKE"]);
        command
    }

    #[cfg(not(windows))]
    {
        let mut command = PtyCommand::new("sh");
        command.args(["-lc", "printf 'NOCTRAIL_PTY_SMOKE'"]);
        command
    }
}

fn read_all_output(session: &mut PtySession) -> Result<Vec<u8>, noctrail_pty::PtyError> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        let count = session.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        output.extend_from_slice(&chunk[..count]);
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_detection_matches_shell_like_patterns() {
        assert!(contains_glob_meta("tests/fixtures/*.json"));
        assert!(contains_glob_meta("tests/fixtures/[ab].json"));
        assert!(!contains_glob_meta("tests/fixtures/core.ntrec"));
    }

    #[test]
    fn render_smoke_succeeds() {
        run_render_smoke().expect("render smoke should pass");
    }

    #[test]
    fn pty_smoke_command_has_a_program() {
        let command = pty_smoke_command();
        assert!(!command.program().is_empty());
    }
}
