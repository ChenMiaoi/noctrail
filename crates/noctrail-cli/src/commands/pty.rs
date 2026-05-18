#[cfg(not(windows))]
use std::env;
use std::{thread, time::Duration};

#[cfg(windows)]
use noctrail_pty::ShellSource;
use noctrail_pty::{PtySession, PtySize, ResolvedShell};
#[cfg(not(windows))]
use noctrail_runtime::PaneRuntime;
use noctrail_runtime::{PaneId, PaneRuntimeRegistry, RuntimeCommand, RuntimeEvent};

pub(crate) fn run_pty_smoke() -> Result<(), String> {
    #[cfg(not(windows))]
    {
        println!("pty-smoke.single-shell=starting");
        run_single_shell_pty_smoke()?;
        println!("pty-smoke.single-shell=ok");
    }
    #[cfg(windows)]
    {
        println!("pty-smoke.single-shell=skipped");
    }
    println!("pty-smoke.runtime-registry=starting");
    run_runtime_registry_pty_smoke()?;
    println!("pty-smoke.runtime-registry=ok");
    println!("pty smoke ok");
    Ok(())
}

#[cfg(not(windows))]
fn run_single_shell_pty_smoke() -> Result<(), String> {
    let probe = pty_resize_smoke_probe()?;
    println!("pty-smoke.single-shell=spawn");
    let mut runtime = PaneRuntime::spawn_shell(PtySize::new(80, 24))
        .map_err(|error| format!("failed to spawn PTY shell: {error}"))?;
    println!("pty-smoke.single-shell=write-initial");
    runtime
        .write(&probe.initial_input)
        .map_err(|error| format!("failed to write initial PTY smoke input: {error}"))?;
    println!("pty-smoke.single-shell=read-initial");
    let initial_output =
        read_runtime_until_fragments(&mut runtime, &probe.initial_expected_fragments)
            .map_err(|error| format!("failed to read initial PTY smoke output: {error}"))?;
    println!("pty-smoke.single-shell=resize");
    runtime
        .resize(probe.resized_size)
        .map_err(|error| format!("failed to resize PTY smoke session: {error}"))?;
    println!("pty-smoke.single-shell=write-resized");
    runtime
        .write(&probe.resized_input)
        .map_err(|error| format!("failed to write resized PTY smoke input: {error}"))?;
    println!("pty-smoke.single-shell=read-resized");
    let resized_output =
        read_runtime_until_fragments(&mut runtime, &probe.resized_expected_fragments)
            .map_err(|error| format!("failed to read PTY smoke output: {error}"))?;
    println!("pty-smoke.single-shell=close");
    let _ = runtime.close();
    println!("pty-smoke.single-shell=validate");

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

#[cfg(not(windows))]
struct PtyResizeSmokeProbe {
    initial_input: Vec<u8>,
    resized_input: Vec<u8>,
    resized_size: PtySize,
    initial_expected_fragments: Vec<String>,
    resized_expected_fragments: Vec<String>,
}

pub(crate) struct PtySmokeProbe {
    pub(crate) marker: String,
    pub(crate) input: Vec<u8>,
    pub(crate) expected_fragments: Vec<String>,
}

#[cfg(not(windows))]
fn pty_resize_smoke_probe() -> Result<PtyResizeSmokeProbe, String> {
    #[cfg(not(windows))]
    let cwd = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let resized_size = PtySize::new(100, 30);

    #[cfg(windows)]
    {
        let shell = ResolvedShell::detect();

        match shell.source() {
            ShellSource::PathPwsh | ShellSource::PathPowerShell => Ok(PtyResizeSmokeProbe {
                initial_input: "Write-Output 'NOCTRAIL_PTY_SMOKE_PRE'\r"
                    .to_string()
                    .into_bytes(),
                resized_input: "Write-Output 'NOCTRAIL_PTY_SMOKE_POST'; exit\r"
                    .to_string()
                    .into_bytes(),
                resized_size,
                initial_expected_fragments: vec!["NOCTRAIL_PTY_SMOKE_PRE".to_string()],
                resized_expected_fragments: vec!["NOCTRAIL_PTY_SMOKE_POST".to_string()],
            }),
            ShellSource::PathWsl => Ok(PtyResizeSmokeProbe {
                initial_input: b"printf 'NOCTRAIL_PTY_SMOKE_PRE\\n'\r".to_vec(),
                resized_input: b"printf 'NOCTRAIL_PTY_SMOKE_POST\\n'; exit\r".to_vec(),
                resized_size,
                initial_expected_fragments: vec!["NOCTRAIL_PTY_SMOKE_PRE".to_string()],
                resized_expected_fragments: vec!["NOCTRAIL_PTY_SMOKE_POST".to_string()],
            }),
            ShellSource::EnvComSpec | ShellSource::FallbackCmd => Ok(PtyResizeSmokeProbe {
                initial_input: b"echo NOCTRAIL_PTY_SMOKE_PRE\r\n".to_vec(),
                resized_input: b"echo NOCTRAIL_PTY_SMOKE_POST\r\nexit\r\n".to_vec(),
                resized_size,
                initial_expected_fragments: vec!["NOCTRAIL_PTY_SMOKE_PRE".to_string()],
                resized_expected_fragments: vec!["NOCTRAIL_PTY_SMOKE_POST".to_string()],
            }),
            ShellSource::EnvShell | ShellSource::FallbackSh => Err(
                "unexpected Unix shell source while building Windows PTY resize probe".to_string(),
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

pub(crate) fn pty_smoke_probe(marker: &str) -> Result<PtySmokeProbe, String> {
    #[cfg(not(windows))]
    let cwd = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;

    #[cfg(windows)]
    {
        let shell = ResolvedShell::detect();

        match shell.source() {
            ShellSource::PathPwsh | ShellSource::PathPowerShell => Ok(PtySmokeProbe {
                marker: marker.to_string(),
                input: format!("Write-Output '{marker}'; exit\r").into_bytes(),
                expected_fragments: vec![marker.to_string()],
            }),
            ShellSource::PathWsl => Ok(PtySmokeProbe {
                marker: marker.to_string(),
                input: format!("printf '{marker}\\n'; exit\r").into_bytes(),
                expected_fragments: vec![marker.to_string()],
            }),
            ShellSource::EnvComSpec | ShellSource::FallbackCmd => Ok(PtySmokeProbe {
                marker: marker.to_string(),
                input: format!("echo {marker}\r\nexit\r\n").into_bytes(),
                expected_fragments: vec![marker.to_string()],
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
        let (output, mut exit_seen) = collect_runtime_events(&mut registry, pane_id)
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

        if registry.contains(pane_id) {
            let closed = registry
                .close(pane_id)
                .map_err(|error| format!("failed to close runtime pane {pane_id:?}: {error}"))?;
            exit_seen |= closed.is_some();
        }

        if !exit_seen {
            return Err(format!(
                "runtime pane {pane_id:?} did not terminate after its smoke probe"
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

pub(crate) fn read_all_output(session: &mut PtySession) -> Result<Vec<u8>, noctrail_pty::PtyError> {
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

#[cfg(not(windows))]
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

#[cfg(not(windows))]
fn read_runtime_until_fragments(
    runtime: &mut PaneRuntime,
    expected_fragments: &[String],
) -> Result<Vec<u8>, noctrail_pty::PtyError> {
    let mut output = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        let count = runtime.read_output(&mut chunk)?;
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

pub(crate) fn collect_runtime_events(
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
            Some(RuntimeEvent::Output { bytes, .. }) => {
                reply_terminal_queries(registry, pane_id, &bytes)?;
                output.extend_from_slice(&bytes);
            }
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

fn reply_terminal_queries(
    registry: &mut PaneRuntimeRegistry,
    pane_id: PaneId,
    bytes: &[u8],
) -> Result<(), String> {
    let query_count = bytes
        .windows(4)
        .filter(|window| *window == b"\x1b[6n")
        .count();

    for _ in 0..query_count {
        registry
            .write_input(pane_id, b"\x1b[1;1R")
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}
