use std::{env, path::PathBuf, process, thread, time::Duration};

#[cfg(windows)]
use noctrail_pty::ShellSource;
use noctrail_pty::{PtySession, PtySize, ResolvedShell};
use noctrail_render::{RenderBackend, RenderPlan, RenderRect};
use noctrail_runtime::{PaneId, PaneRuntimeRegistry, RuntimeCommand, RuntimeEvent};
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
    run_single_shell_pty_smoke()?;
    run_runtime_registry_pty_smoke()?;
    println!("pty smoke ok");
    Ok(())
}

fn run_single_shell_pty_smoke() -> Result<(), String> {
    let probe = pty_resize_smoke_probe()?;
    let mut session = PtySession::spawn_shell(PtySize::new(80, 24))
        .map_err(|error| format!("failed to spawn PTY shell: {error}"))?;
    session
        .write(&probe.initial_input)
        .map_err(|error| format!("failed to write initial PTY smoke input: {error}"))?;
    let initial_output = read_until_fragments(&mut session, &probe.initial_expected_fragments)
        .map_err(|error| format!("failed to read initial PTY smoke output: {error}"))?;
    session
        .resize(probe.resized_size)
        .map_err(|error| format!("failed to resize PTY smoke session: {error}"))?;
    session
        .write(&probe.resized_input)
        .map_err(|error| format!("failed to write resized PTY smoke input: {error}"))?;
    let resized_output = read_all_output(&mut session)
        .map_err(|error| format!("failed to read PTY smoke output: {error}"))?;
    let _ = session.close();

    let initial_haystack = String::from_utf8_lossy(&initial_output);
    let resized_haystack = String::from_utf8_lossy(&resized_output);

    for expected in &probe.initial_expected_fragments {
        if !initial_haystack.contains(expected) {
            return Err(format!(
                "initial PTY smoke output missing {:?}; output was {:?}",
                expected, initial_haystack
            ));
        }
    }

    for expected in &probe.resized_expected_fragments {
        if !resized_haystack.contains(expected) {
            return Err(format!(
                "resized PTY smoke output missing {:?}; output was {:?}",
                expected, resized_haystack
            ));
        }
    }

    Ok(())
}

struct PtyResizeSmokeProbe {
    initial_input: Vec<u8>,
    resized_input: Vec<u8>,
    resized_size: PtySize,
    initial_expected_fragments: Vec<String>,
    resized_expected_fragments: Vec<String>,
}

struct PtySmokeProbe {
    marker: String,
    input: Vec<u8>,
    expected_fragments: Vec<String>,
}

fn pty_resize_smoke_probe() -> Result<PtyResizeSmokeProbe, String> {
    let cwd = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let resized_size = PtySize::new(100, 30);

    #[cfg(windows)]
    {
        let shell = ResolvedShell::detect();
        let cwd = cwd.display().to_string();

        match shell.source() {
            ShellSource::PathPwsh | ShellSource::PathPowerShell => Ok(PtyResizeSmokeProbe {
                initial_input: "Write-Output 'NOCTRAIL_PTY_SMOKE_PRE'; (Get-Location).Path; Write-Output \"$($Host.UI.RawUI.WindowSize.Height) $($Host.UI.RawUI.WindowSize.Width)\"\r".to_string().into_bytes(),
                resized_input: "Write-Output 'NOCTRAIL_PTY_SMOKE_POST'; Write-Output \"$($Host.UI.RawUI.WindowSize.Height) $($Host.UI.RawUI.WindowSize.Width)\"; exit\r".to_string().into_bytes(),
                resized_size,
                initial_expected_fragments: vec![
                    "NOCTRAIL_PTY_SMOKE_PRE".to_string(),
                    cwd,
                    "24 80".to_string(),
                ],
                resized_expected_fragments: vec![
                    "NOCTRAIL_PTY_SMOKE_POST".to_string(),
                    "30 100".to_string(),
                ],
            }),
            ShellSource::PathWsl => Ok(PtyResizeSmokeProbe {
                initial_input: b"printf 'NOCTRAIL_PTY_SMOKE_PRE\\n'; pwd; stty size\r".to_vec(),
                resized_input: b"printf 'NOCTRAIL_PTY_SMOKE_POST\\n'; stty size; exit\r".to_vec(),
                resized_size,
                initial_expected_fragments: vec![
                    "NOCTRAIL_PTY_SMOKE_PRE".to_string(),
                    cwd,
                    "24 80".to_string(),
                ],
                resized_expected_fragments: vec![
                    "NOCTRAIL_PTY_SMOKE_POST".to_string(),
                    "30 100".to_string(),
                ],
            }),
            ShellSource::EnvComSpec | ShellSource::FallbackCmd => Ok(PtyResizeSmokeProbe {
                initial_input: b"echo NOCTRAIL_PTY_SMOKE_PRE\r\ncd\r\nmode con\r\n".to_vec(),
                resized_input: b"echo NOCTRAIL_PTY_SMOKE_POST\r\nmode con\r\nexit\r\n".to_vec(),
                resized_size,
                initial_expected_fragments: vec![
                    "NOCTRAIL_PTY_SMOKE_PRE".to_string(),
                    cwd,
                    "80".to_string(),
                    "24".to_string(),
                ],
                resized_expected_fragments: vec![
                    "NOCTRAIL_PTY_SMOKE_POST".to_string(),
                    "100".to_string(),
                    "30".to_string(),
                ],
            }),
            ShellSource::EnvShell | ShellSource::FallbackSh => Err(
                "unexpected Unix shell source while building Windows PTY resize probe"
                    .to_string(),
            ),
        }
    }

    #[cfg(not(windows))]
    {
        Ok(PtyResizeSmokeProbe {
            initial_input: b"printf 'NOCTRAIL_PTY_SMOKE_PRE\\n'; pwd; stty size\r".to_vec(),
            resized_input: b"printf 'NOCTRAIL_PTY_SMOKE_POST\\n'; stty size; exit\r".to_vec(),
            resized_size,
            initial_expected_fragments: vec![
                "NOCTRAIL_PTY_SMOKE_PRE".to_string(),
                cwd.display().to_string(),
                "24 80".to_string(),
            ],
            resized_expected_fragments: vec![
                "NOCTRAIL_PTY_SMOKE_POST".to_string(),
                "30 100".to_string(),
            ],
        })
    }
}

fn pty_smoke_probe(marker: &str) -> Result<PtySmokeProbe, String> {
    let cwd = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;

    #[cfg(windows)]
    {
        let shell = ResolvedShell::detect();
        let cwd = cwd.display().to_string();

        match shell.source() {
            ShellSource::PathPwsh | ShellSource::PathPowerShell => Ok(PtySmokeProbe {
                marker: marker.to_string(),
                input: format!(
                    "Write-Output '{marker}'; (Get-Location).Path; Write-Output \"$($Host.UI.RawUI.WindowSize.Height) $($Host.UI.RawUI.WindowSize.Width)\"; exit\r"
                )
                .into_bytes(),
                expected_fragments: vec![
                    marker.to_string(),
                    cwd,
                    "24 80".to_string(),
                ],
            }),
            ShellSource::PathWsl => Ok(PtySmokeProbe {
                marker: marker.to_string(),
                input: format!("printf '{marker}\\n'; pwd; stty size; exit\r").into_bytes(),
                expected_fragments: vec![
                    marker.to_string(),
                    cwd,
                    "24 80".to_string(),
                ],
            }),
            ShellSource::EnvComSpec | ShellSource::FallbackCmd => Ok(PtySmokeProbe {
                marker: marker.to_string(),
                input: format!("echo {marker}\r\ncd\r\nmode con\r\nexit\r\n").into_bytes(),
                expected_fragments: vec![
                    marker.to_string(),
                    cwd,
                    "Columns:".to_string(),
                    "Lines:".to_string(),
                ],
            }),
            ShellSource::EnvShell | ShellSource::FallbackSh => Err(
                "unexpected Unix shell source while building Windows PTY smoke probe".to_string(),
            ),
        }
    }

    #[cfg(not(windows))]
    {
        Ok(PtySmokeProbe {
            marker: marker.to_string(),
            input: format!("printf '{marker}\\n'; pwd; stty size; exit\r").into_bytes(),
            expected_fragments: vec![
                marker.to_string(),
                cwd.display().to_string(),
                "24 80".to_string(),
            ],
        })
    }
}

fn run_runtime_registry_pty_smoke() -> Result<(), String> {
    let markers = [
        "NOCTRAIL_PTY_SMOKE_1",
        "NOCTRAIL_PTY_SMOKE_2",
        "NOCTRAIL_PTY_SMOKE_3",
        "NOCTRAIL_PTY_SMOKE_4",
    ];
    let mut registry = PaneRuntimeRegistry::new();
    let mut panes = Vec::new();

    for marker in markers {
        let pane_id = registry
            .spawn_shell(PtySize::new(80, 24))
            .map_err(|error| format!("failed to spawn runtime pane {marker}: {error}"))?;
        let probe = pty_smoke_probe(marker)?;
        let write_result = registry
            .apply_command(RuntimeCommand::Write {
                pane_id,
                bytes: probe.input.clone(),
            })
            .map_err(|error| format!("failed to write runtime pane {pane_id:?}: {error}"))?;
        if write_result.is_some() {
            return Err(format!(
                "runtime pane {pane_id:?} write command unexpectedly emitted an event"
            ));
        }
        panes.push((pane_id, probe));
    }

    for (pane_id, probe) in panes {
        let (output, exit_seen) = collect_runtime_events(&mut registry, pane_id)
            .map_err(|error| format!("failed to read runtime pane {pane_id:?}: {error}"))?;
        let haystack = String::from_utf8_lossy(&output);

        for expected in &probe.expected_fragments {
            if !haystack.contains(expected) {
                return Err(format!(
                    "runtime pane {pane_id:?} output missing {:?}; output was {:?}",
                    expected, haystack
                ));
            }
        }

        for marker in markers {
            if marker != probe.marker && haystack.contains(marker) {
                return Err(format!(
                    "runtime pane {pane_id:?} leaked marker {:?}; output was {:?}",
                    marker, haystack
                ));
            }
        }

        if !exit_seen {
            return Err(format!(
                "runtime pane {pane_id:?} did not emit an exit event before EOF"
            ));
        }
        if registry.contains(pane_id) {
            return Err(format!(
                "runtime pane {pane_id:?} should have been removed after its exit event"
            ));
        }
    }

    Ok(())
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

fn read_until_fragments(
    session: &mut PtySession,
    expected_fragments: &[String],
) -> Result<Vec<u8>, noctrail_pty::PtyError> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        let count = session.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        output.extend_from_slice(&chunk[..count]);

        let haystack = String::from_utf8_lossy(&output);
        if expected_fragments
            .iter()
            .all(|expected| haystack.contains(expected))
        {
            break;
        }
    }

    Ok(output)
}

fn collect_runtime_events(
    registry: &mut PaneRuntimeRegistry,
    pane_id: PaneId,
) -> Result<(Vec<u8>, bool), String> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 1024];
    let mut exit_seen = false;
    let mut idle_polls = 0_u32;

    loop {
        match registry
            .read_output_event(pane_id, &mut chunk)
            .map_err(|error| error.to_string())?
        {
            Some(RuntimeEvent::Output { bytes, .. }) => output.extend_from_slice(&bytes),
            Some(RuntimeEvent::Exited { .. }) => {
                exit_seen = true;
                break;
            }
            Some(RuntimeEvent::Error { error, .. }) => return Err(error.to_string()),
            None => {
                if !registry.contains(pane_id) {
                    break;
                }

                idle_polls += 1;
                if idle_polls >= 100 {
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
    }

    Ok((output, exit_seen))
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
    fn pty_smoke_probe_contains_sentinel() {
        let probe = pty_smoke_probe("NOCTRAIL_PTY_SMOKE").expect("pty smoke probe should build");
        let script = String::from_utf8(probe.input).expect("probe input should be utf-8");
        assert!(script.contains("NOCTRAIL_PTY_SMOKE"));
    }
}
