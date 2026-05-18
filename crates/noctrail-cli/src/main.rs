mod commands;
mod installer;

use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[cfg(not(windows))]
use commands::hook::render_bash_hook;
use commands::{
    common::{find_executable, make_executable_path, temp_fixture_path},
    doctor::{
        print_doctor, print_doctor_config, print_doctor_font, print_doctor_gpu,
        print_doctor_permissions, print_doctor_pty, print_doctor_shell,
    },
    hook::{print_shell_hook, run_hook_smoke},
    pty::run_pty_smoke,
    render::{replay_fixtures, run_render_fixtures, run_render_smoke},
    shell::run_shell_matrix,
    unicode::run_unicode_matrix,
};
use noctrail_pty::{PtySession, PtySize};
use noctrail_term::{
    Color, MouseTrackingMode, ScreenRowSnapshot, ShellIntegrationEvent, TerminalSnapshot,
    TerminalState,
};

const HELP: &str = "\
Noctrail development CLI

Usage:
  noctrail [command]

Commands:
  doctor      Print basic environment diagnostics
  doctor shell  Print shell resolution diagnostics
  doctor pty  Print PTY spawn diagnostics
  doctor gpu  Print GPU backend diagnostics
  doctor font Print font fallback diagnostics
  doctor config [path] Print config diagnostics
  doctor permissions Print local path permission diagnostics
  installer-smoke Run the packaged installer lifecycle smoke
  shell-hook  Print a shell integration hook script
  hook-smoke  Run the shell hook compatibility smoke matrix
  replay      Replay one or more terminal recording fixtures
  shell-matrix Run the shell compatibility smoke matrix
  prompt-matrix Run the prompt compatibility smoke matrix
  unicode-matrix Run the Unicode and cursor compatibility matrix
  tui-matrix  Run the TUI compatibility smoke matrix
  render-fixtures  Run deterministic render fixtures
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
                Some("pty") => {
                    if let Err(error) = print_doctor_pty() {
                        eprintln!("{error}");
                        process::exit(1);
                    }
                }
                Some("gpu") => {
                    if let Err(error) = print_doctor_gpu() {
                        eprintln!("{error}");
                        process::exit(1);
                    }
                }
                Some("font") => print_doctor_font(),
                Some("config") => {
                    let path = args.next().map(PathBuf::from);
                    if let Err(error) = print_doctor_config(path.as_deref()) {
                        eprintln!("{error}");
                        process::exit(1);
                    }
                }
                Some("permissions") => {
                    if let Err(error) = print_doctor_permissions() {
                        eprintln!("{error}");
                        process::exit(1);
                    }
                }
                Some(other) => {
                    eprintln!("unknown doctor topic: {other}");
                    process::exit(2);
                }
            }
        }
        Some("installer-smoke") => {
            if let Err(error) = installer::run_installer_smoke() {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        Some("shell-hook") => {
            let Some(shell) = args.next() else {
                eprintln!("shell-hook requires a shell target");
                process::exit(2);
            };
            match print_shell_hook(&shell) {
                Ok(()) => {}
                Err(error) => {
                    eprintln!("{error}");
                    process::exit(1);
                }
            }
        }
        Some("hook-smoke") => {
            let targets: Vec<String> = args.collect();
            if let Err(error) = run_hook_smoke(&targets) {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        Some("render-smoke") => {
            if let Err(error) = run_render_smoke() {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        Some("tui-matrix") => {
            let targets: Vec<String> = args.collect();
            if let Err(error) = run_tui_matrix(&targets) {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        Some("shell-matrix") => {
            let targets: Vec<String> = args.collect();
            if let Err(error) = run_shell_matrix(&targets) {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        Some("prompt-matrix") => {
            let targets: Vec<String> = args.collect();
            if let Err(error) = run_prompt_matrix(&targets) {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        Some("unicode-matrix") => {
            let targets: Vec<String> = args.collect();
            if let Err(error) = run_unicode_matrix(&targets) {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        Some("render-fixtures") => {
            let patterns: Vec<String> = args.collect();
            if let Err(error) = run_render_fixtures(&patterns) {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiCapability {
    AltScreen,
    Mouse,
    Resize,
    Color,
}

impl TuiCapability {
    const fn label(self) -> &'static str {
        match self {
            Self::AltScreen => "alt-screen",
            Self::Mouse => "mouse",
            Self::Resize => "resize",
            Self::Color => "color",
        }
    }
}

#[derive(Debug, Clone)]
struct TuiTargetSpec {
    name: &'static str,
    aliases: &'static [&'static str],
    override_env: &'static str,
    program_candidates: &'static [&'static str],
    required: &'static [TuiCapability],
    actual_probe: Option<TuiActualProbe>,
    skip_hint: &'static str,
}

type TuiActualProbe = fn(&Path) -> Result<TuiProbe, String>;

#[derive(Debug, Clone)]
struct TuiProbe {
    target: &'static str,
    source: String,
    command: noctrail_pty::PtyCommand,
    initial_size: PtySize,
    steps: Vec<TuiProbeStep>,
    required: &'static [TuiCapability],
    timeout: Duration,
    cleanup_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
struct TuiProbeStep {
    at: Duration,
    action: TuiProbeAction,
}

#[derive(Debug, Clone)]
enum TuiProbeAction {
    Write(Vec<u8>),
    Resize(PtySize),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ObservedCapabilities {
    alt_screen: bool,
    mouse: bool,
    resize: bool,
    color: bool,
}

impl ObservedCapabilities {
    fn contains(self, capability: TuiCapability) -> bool {
        match capability {
            TuiCapability::AltScreen => self.alt_screen,
            TuiCapability::Mouse => self.mouse,
            TuiCapability::Resize => self.resize,
            TuiCapability::Color => self.color,
        }
    }

    fn labels(self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        for capability in [
            TuiCapability::AltScreen,
            TuiCapability::Mouse,
            TuiCapability::Resize,
            TuiCapability::Color,
        ] {
            if self.contains(capability) {
                labels.push(capability.label());
            }
        }
        labels
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TuiProbeStatus {
    Passed,
    Skipped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct TuiProbeReport {
    target: &'static str,
    source: String,
    observed: ObservedCapabilities,
    required: Vec<&'static str>,
    status: TuiProbeStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptCapability {
    Layout,
    Escape,
    Hook,
}

impl PromptCapability {
    const fn label(self) -> &'static str {
        match self {
            Self::Layout => "layout",
            Self::Escape => "escape",
            Self::Hook => "hook",
        }
    }
}

#[derive(Debug, Clone)]
struct PromptTargetSpec {
    name: &'static str,
    build_probe: Option<PromptProbeBuilder>,
    required: &'static [PromptCapability],
    skip_hint: &'static str,
}

type PromptProbeBuilder = fn() -> Result<PromptProbe, String>;

#[derive(Debug, Clone)]
struct PromptProbe {
    target: &'static str,
    source: String,
    command: noctrail_pty::PtyCommand,
    initial_size: PtySize,
    prompt_lines: Vec<String>,
    bootstrap_bytes: Vec<u8>,
    input_line: String,
    submit_bytes: Vec<u8>,
    input_row: usize,
    expected_result: String,
    expected_cwd: String,
    required: &'static [PromptCapability],
    cleanup_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ObservedPromptCapabilities {
    layout: bool,
    escape: bool,
    hook: bool,
}

impl ObservedPromptCapabilities {
    fn contains(self, capability: PromptCapability) -> bool {
        match capability {
            PromptCapability::Layout => self.layout,
            PromptCapability::Escape => self.escape,
            PromptCapability::Hook => self.hook,
        }
    }

    fn labels(self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        for capability in [
            PromptCapability::Layout,
            PromptCapability::Escape,
            PromptCapability::Hook,
        ] {
            if self.contains(capability) {
                labels.push(capability.label());
            }
        }
        labels
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PromptProbeStatus {
    Passed,
    Skipped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PromptProbeReport {
    target: &'static str,
    source: String,
    observed: ObservedPromptCapabilities,
    required: Vec<&'static str>,
    status: PromptProbeStatus,
}

fn run_tui_matrix(filters: &[String]) -> Result<(), String> {
    let specs = tui_target_specs();
    let selected = select_tui_targets(specs, filters)?;
    let mut ran_any = false;
    let mut failures = Vec::new();

    for spec in selected {
        match run_tui_target(spec) {
            Ok(report) => {
                println!("{}", format_tui_report(&report));
                if matches!(report.status, TuiProbeStatus::Passed) {
                    ran_any = true;
                }
            }
            Err(error) => {
                failures.push(format!("{}: {error}", spec.name));
            }
        }
    }

    if !failures.is_empty() {
        return Err(failures.join("\n"));
    }

    if !ran_any {
        println!("tui-matrix: all selected targets were skipped");
    } else {
        println!("tui matrix ok");
    }
    Ok(())
}

fn run_prompt_matrix(filters: &[String]) -> Result<(), String> {
    let specs = prompt_target_specs();
    let selected = select_prompt_targets(&specs, filters)?;
    let mut ran_any = false;
    let mut failures = Vec::new();

    for spec in selected {
        match run_prompt_target(spec) {
            Ok(report) => {
                println!("{}", format_prompt_report(&report));
                if matches!(report.status, PromptProbeStatus::Passed) {
                    ran_any = true;
                }
            }
            Err(error) => failures.push(format!("{}: {error}", spec.name)),
        }
    }

    if !failures.is_empty() {
        return Err(failures.join("\n"));
    }

    if !ran_any {
        println!("prompt-matrix: all selected targets were skipped");
    } else {
        println!("prompt matrix ok");
    }
    Ok(())
}

fn run_prompt_target(spec: &PromptTargetSpec) -> Result<PromptProbeReport, String> {
    let Some(build_probe) = spec.build_probe else {
        return Ok(PromptProbeReport {
            target: spec.name,
            source: "unavailable".to_string(),
            observed: ObservedPromptCapabilities::default(),
            required: spec.required.iter().map(|cap| cap.label()).collect(),
            status: PromptProbeStatus::Skipped(spec.skip_hint.to_string()),
        });
    };

    let probe = build_probe()?;
    let result = run_prompt_probe(&probe);
    cleanup_prompt_probe(&probe);
    let observed = result?;
    let missing = probe
        .required
        .iter()
        .filter(|capability| !observed.contains(**capability))
        .map(|capability| capability.label())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "missing capabilities [{}] from {}",
            missing.join(", "),
            probe.source
        ));
    }

    Ok(PromptProbeReport {
        target: probe.target,
        source: probe.source,
        observed,
        required: probe.required.iter().map(|cap| cap.label()).collect(),
        status: PromptProbeStatus::Passed,
    })
}

fn run_prompt_probe(probe: &PromptProbe) -> Result<ObservedPromptCapabilities, String> {
    let mut session = PtySession::spawn(probe.command.clone(), probe.initial_size)
        .map_err(|error| format!("failed to spawn {} prompt probe: {error}", probe.target))?;
    let reader = session
        .clone_output_reader()
        .map_err(|error| format!("failed to clone {} prompt reader: {error}", probe.target))?;
    let (tx, rx) = mpsc::channel();
    let reader_handle = thread::spawn(move || pump_tui_output(reader, tx));
    let mut terminal = TerminalState::new(
        usize::from(probe.initial_size.cols),
        usize::from(probe.initial_size.rows),
    );
    let _ = terminal.grid_mut().take_dirty_rows();
    let mut observed = ObservedPromptCapabilities::default();
    let mut input_sent = false;
    let mut exit_requested = false;
    let mut prompt_ready = false;
    let mut exit_seen = false;
    let mut ready_snapshot = None;
    let started_at = Instant::now();
    let timeout = Duration::from_secs(4);

    session
        .write(&probe.bootstrap_bytes)
        .map_err(|error| format!("failed to bootstrap {} prompt probe: {error}", probe.target))?;

    while started_at.elapsed() <= timeout {
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(TuiProbeReaderEvent::Bytes(bytes)) => {
                terminal.advance_bytes(&bytes);
                if !input_sent && prompt_is_ready(&terminal.snapshot(), probe) {
                    prompt_ready = true;
                    ready_snapshot = Some(terminal.snapshot());
                    session.write(&probe.submit_bytes).map_err(|error| {
                        format!("failed to write {} prompt input: {error}", probe.target)
                    })?;
                    input_sent = true;
                }
                if input_sent
                    && !exit_requested
                    && terminal
                        .snapshot()
                        .rows
                        .iter()
                        .map(ScreenRowSnapshot::rendered_text)
                        .any(|row| row.contains(&probe.expected_result))
                {
                    session.write(b"exit\r").map_err(|error| {
                        format!("failed to exit {} prompt probe: {error}", probe.target)
                    })?;
                    exit_requested = true;
                }
            }
            Ok(TuiProbeReaderEvent::Eof) => {
                exit_seen = session.try_wait().ok().flatten().is_some();
                break;
            }
            Ok(TuiProbeReaderEvent::Error(error)) => {
                let _ = session.close();
                let _ = reader_handle.join();
                return Err(format!("{} prompt reader error: {error}", probe.target));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if session.try_wait().ok().flatten().is_some() {
                    exit_seen = true;
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                exit_seen = true;
                break;
            }
        }
    }

    let status = session
        .close()
        .map_err(|error| format!("failed to close {} prompt probe: {error}", probe.target))?;
    let _ = reader_handle.join();
    exit_seen |= status.as_ref().is_some_and(|status| status.success());
    if !exit_seen {
        return Err(format!("{} prompt probe timed out", probe.target));
    }

    let ready_snapshot = ready_snapshot.unwrap_or_else(|| terminal.snapshot());
    let events = terminal.drain_shell_integration_events();
    let ready_rows = ready_snapshot
        .rows
        .iter()
        .map(ScreenRowSnapshot::rendered_text)
        .collect::<Vec<_>>();
    observed.layout = prompt_ready && prompt_rows_match_at_anchor(&ready_rows, probe);
    observed.escape = prompt_escape_is_clean(&ready_snapshot, &ready_rows, probe);
    observed.hook = prompt_hook_events_match(&events, probe);

    Ok(observed)
}

fn prompt_hook_events_match(events: &[ShellIntegrationEvent], probe: &PromptProbe) -> bool {
    events
        .iter()
        .any(|event| matches!(event, ShellIntegrationEvent::Prompt))
        && events.iter().any(|event| {
            matches!(
                event,
                ShellIntegrationEvent::CommandText(text) if text.contains(&probe.input_line)
            )
        })
        && events
            .iter()
            .any(|event| matches!(event, ShellIntegrationEvent::CommandStart))
        && events
            .iter()
            .any(|event| matches!(event, ShellIntegrationEvent::CommandEnd))
        && events.iter().any(|event| {
            matches!(
                event,
                ShellIntegrationEvent::Cwd(cwd) if cwd == &probe.expected_cwd
            )
        })
        && events
            .iter()
            .any(|event| matches!(event, ShellIntegrationEvent::ExitCode(0)))
        && events
            .iter()
            .any(|event| matches!(event, ShellIntegrationEvent::DurationMs(_)))
}

fn prompt_is_ready(snapshot: &TerminalSnapshot, probe: &PromptProbe) -> bool {
    let rendered_rows = snapshot
        .rows
        .iter()
        .map(ScreenRowSnapshot::rendered_text)
        .collect::<Vec<_>>();
    if prompt_anchor(&rendered_rows, &probe.prompt_lines).is_none() {
        return false;
    }
    let Some(current_row) = rendered_rows.get(snapshot.cursor.row) else {
        return false;
    };

    current_row.starts_with(&probe.prompt_lines[probe.input_row])
        && snapshot.cursor.col >= probe.prompt_lines[probe.input_row].len()
}

fn prompt_rows_match_at_anchor(rendered_rows: &[String], probe: &PromptProbe) -> bool {
    let Some(anchor) = prompt_anchor(rendered_rows, &probe.prompt_lines) else {
        return false;
    };

    prompt_rows_match(rendered_rows, probe, anchor)
}

fn prompt_rows_match(rendered_rows: &[String], probe: &PromptProbe, anchor: usize) -> bool {
    probe
        .prompt_lines
        .iter()
        .enumerate()
        .all(|(index, expected)| {
            rendered_rows
                .get(anchor + index)
                .is_some_and(|row| row.starts_with(expected))
        })
}

fn prompt_escape_is_clean(
    snapshot: &TerminalSnapshot,
    rendered_rows: &[String],
    probe: &PromptProbe,
) -> bool {
    if rendered_rows.iter().any(|row| row.contains('\u{1b}')) {
        return false;
    }

    let Some(anchor) = prompt_anchor(rendered_rows, &probe.prompt_lines) else {
        return false;
    };

    prompt_rows_match(rendered_rows, probe, anchor)
        && prompt_has_non_default_style(snapshot, probe, anchor)
}

fn prompt_has_non_default_style(
    snapshot: &TerminalSnapshot,
    probe: &PromptProbe,
    anchor: usize,
) -> bool {
    probe
        .prompt_lines
        .iter()
        .enumerate()
        .all(|(row_index, expected)| {
            let Some(row) = snapshot.rows.get(anchor + row_index) else {
                return false;
            };
            row.cells
                .iter()
                .take(expected.len())
                .filter(|cell| !cell.text.is_empty())
                .any(|cell| !cell.style.is_default())
        })
}

fn prompt_anchor(rendered_rows: &[String], prompt_lines: &[String]) -> Option<usize> {
    prompt_anchors(rendered_rows, prompt_lines)
        .into_iter()
        .next()
}

fn prompt_anchors(rendered_rows: &[String], prompt_lines: &[String]) -> Vec<usize> {
    if prompt_lines.is_empty() || prompt_lines.len() > rendered_rows.len() {
        return Vec::new();
    }

    (0..=rendered_rows.len() - prompt_lines.len())
        .rev()
        .filter(|start| {
            prompt_lines.iter().enumerate().all(|(offset, expected)| {
                rendered_rows
                    .get(start + offset)
                    .is_some_and(|row| row.starts_with(expected))
            })
        })
        .collect()
}

fn cleanup_prompt_probe(probe: &PromptProbe) {
    for path in &probe.cleanup_paths {
        let _ = fs::remove_file(path);
    }
}

fn format_prompt_report(report: &PromptProbeReport) -> String {
    let required = if report.required.is_empty() {
        "none".to_string()
    } else {
        report.required.join(",")
    };
    let observed = {
        let labels = report.observed.labels();
        if labels.is_empty() {
            "none".to_string()
        } else {
            labels.join(",")
        }
    };

    match &report.status {
        PromptProbeStatus::Passed => format!(
            "pass {} source={} required={} observed={}",
            report.target, report.source, required, observed
        ),
        PromptProbeStatus::Skipped(reason) => format!(
            "skip {} source={} required={} reason={}",
            report.target, report.source, required, reason
        ),
    }
}

fn select_prompt_targets<'a>(
    specs: &'a [PromptTargetSpec],
    filters: &[String],
) -> Result<Vec<&'a PromptTargetSpec>, String> {
    if filters.is_empty() {
        return Ok(specs.iter().collect());
    }

    let mut selected = Vec::new();
    for filter in filters {
        let Some(spec) = specs.iter().find(|spec| spec.name == filter) else {
            return Err(format!("unknown prompt target: {filter}"));
        };
        if !selected
            .iter()
            .any(|existing: &&PromptTargetSpec| existing.name == spec.name)
        {
            selected.push(spec);
        }
    }

    Ok(selected)
}

#[cfg(not(windows))]
fn prompt_target_specs() -> Vec<PromptTargetSpec> {
    vec![
        PromptTargetSpec {
            name: "starship",
            build_probe: Some(starship_prompt_probe),
            required: &[
                PromptCapability::Layout,
                PromptCapability::Escape,
                PromptCapability::Hook,
            ],
            skip_hint: "prompt emulation is unavailable on this platform",
        },
        PromptTargetSpec {
            name: "oh-my-zsh",
            build_probe: Some(oh_my_zsh_prompt_probe),
            required: &[
                PromptCapability::Layout,
                PromptCapability::Escape,
                PromptCapability::Hook,
            ],
            skip_hint: "prompt emulation is unavailable on this platform",
        },
        PromptTargetSpec {
            name: "powerlevel10k",
            build_probe: Some(powerlevel10k_prompt_probe),
            required: &[
                PromptCapability::Layout,
                PromptCapability::Escape,
                PromptCapability::Hook,
            ],
            skip_hint: "prompt emulation is unavailable on this platform",
        },
    ]
}

#[cfg(windows)]
fn prompt_target_specs() -> Vec<PromptTargetSpec> {
    vec![
        PromptTargetSpec {
            name: "starship",
            build_probe: None,
            required: &[
                PromptCapability::Layout,
                PromptCapability::Escape,
                PromptCapability::Hook,
            ],
            skip_hint: "prompt emulation is unavailable on this platform",
        },
        PromptTargetSpec {
            name: "oh-my-zsh",
            build_probe: None,
            required: &[
                PromptCapability::Layout,
                PromptCapability::Escape,
                PromptCapability::Hook,
            ],
            skip_hint: "prompt emulation is unavailable on this platform",
        },
        PromptTargetSpec {
            name: "powerlevel10k",
            build_probe: None,
            required: &[
                PromptCapability::Layout,
                PromptCapability::Escape,
                PromptCapability::Hook,
            ],
            skip_hint: "prompt emulation is unavailable on this platform",
        },
    ]
}

#[cfg(not(windows))]
fn starship_prompt_probe() -> Result<PromptProbe, String> {
    hooked_bash_prompt_probe(
        "starship",
        r#"PS1=$'\[\e[32m\]STARSHIP\[\e[0m\] \[\e[34m\]PROMPT\[\e[0m\] > '"#,
        vec!["STARSHIP PROMPT > ".to_string()],
        0,
        "printf 'RESULT:status\\n'",
    )
}

#[cfg(not(windows))]
fn oh_my_zsh_prompt_probe() -> Result<PromptProbe, String> {
    hooked_bash_prompt_probe(
        "oh-my-zsh",
        r#"PS1=$'\[\e[35m\]OHMYZSH\[\e[0m\] \[\e[33m\]%\[\e[0m\] '"#,
        vec!["OHMYZSH % ".to_string()],
        0,
        "printf 'RESULT:pwd\\n'",
    )
}

#[cfg(not(windows))]
fn powerlevel10k_prompt_probe() -> Result<PromptProbe, String> {
    hooked_bash_prompt_probe(
        "powerlevel10k",
        r#"PS1=$'\[\e[36m\]P10K-L1\[\e[0m\] \[\e[35m\]P10K>\[\e[0m\] '"#,
        vec!["P10K-L1 P10K> ".to_string()],
        0,
        "printf 'RESULT:build\\n'",
    )
}

#[cfg(not(windows))]
fn hooked_bash_prompt_probe(
    target: &'static str,
    prompt_setup: &str,
    prompt_lines: Vec<String>,
    input_row: usize,
    input_line: &str,
) -> Result<PromptProbe, String> {
    let cwd = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let program_path = find_executable(&["bash"])
        .ok_or_else(|| "install bash to run prompt-matrix".to_string())?;
    let hook_path = temp_fixture_path(&format!("{target}-prompt-hook"), "sh");
    fs::write(&hook_path, render_bash_hook())
        .map_err(|error| format!("failed to write {target} prompt hook: {error}"))?;
    make_executable_path(&hook_path)?;

    let mut command = noctrail_pty::PtyCommand::new(program_path.as_os_str());
    command.args(["--noprofile", "--norc", "-i"]);
    command.cwd_path(&cwd);

    Ok(PromptProbe {
        target,
        source: format!(
            "program:{}+hook:{}",
            program_path.display(),
            hook_path.display()
        ),
        command,
        initial_size: PtySize::new(120, 24),
        prompt_lines,
        bootstrap_bytes: format!(". '{}'\r{prompt_setup}\r", hook_path.display()).into_bytes(),
        input_line: input_line.to_string(),
        submit_bytes: format!("{input_line}\r").into_bytes(),
        input_row,
        expected_result: input_line
            .strip_prefix("printf '")
            .and_then(|text| text.strip_suffix("\\n'"))
            .unwrap_or("RESULT")
            .to_string(),
        expected_cwd: cwd.display().to_string(),
        required: &[
            PromptCapability::Layout,
            PromptCapability::Escape,
            PromptCapability::Hook,
        ],
        cleanup_paths: vec![hook_path],
    })
}

fn run_tui_target(spec: &'static TuiTargetSpec) -> Result<TuiProbeReport, String> {
    let Some(probe) = resolve_tui_probe(spec)? else {
        return Ok(TuiProbeReport {
            target: spec.name,
            source: "unavailable".to_string(),
            observed: ObservedCapabilities::default(),
            required: spec.required.iter().map(|cap| cap.label()).collect(),
            status: TuiProbeStatus::Skipped(spec.skip_hint.to_string()),
        });
    };

    let result = run_tui_probe(&probe);
    cleanup_tui_probe(&probe);

    let observed = result?;
    let missing = probe
        .required
        .iter()
        .filter(|capability| !observed.contains(**capability))
        .map(|capability| capability.label())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "missing capabilities [{}] from {}",
            missing.join(", "),
            probe.source
        ));
    }

    Ok(TuiProbeReport {
        target: probe.target,
        source: probe.source,
        observed,
        required: probe.required.iter().map(|cap| cap.label()).collect(),
        status: TuiProbeStatus::Passed,
    })
}

fn resolve_tui_probe(spec: &'static TuiTargetSpec) -> Result<Option<TuiProbe>, String> {
    if let Some(override_path) = env::var_os(spec.override_env) {
        return Ok(Some(override_tui_probe(
            spec,
            PathBuf::from(override_path),
            spec.override_env,
        )?));
    }

    let Some(actual_probe) = spec.actual_probe else {
        return Ok(None);
    };
    let Some(program_path) = find_executable(spec.program_candidates) else {
        return Ok(None);
    };
    actual_probe(&program_path).map(Some)
}

fn override_tui_probe(
    spec: &'static TuiTargetSpec,
    program_path: PathBuf,
    source_label: &str,
) -> Result<TuiProbe, String> {
    let mut command = noctrail_pty::PtyCommand::new(program_path.as_os_str());
    command.cwd_path(
        env::current_dir()
            .map_err(|error| format!("failed to resolve current working directory: {error}"))?,
    );

    Ok(TuiProbe {
        target: spec.name,
        source: format!("override:{source_label}"),
        command,
        initial_size: PtySize::new(80, 24),
        steps: vec![
            TuiProbeStep {
                at: Duration::from_millis(150),
                action: TuiProbeAction::Write(b"start\r".to_vec()),
            },
            TuiProbeStep {
                at: Duration::from_millis(300),
                action: TuiProbeAction::Resize(PtySize::new(100, 30)),
            },
            TuiProbeStep {
                at: Duration::from_millis(450),
                action: TuiProbeAction::Write(b"resize\r".to_vec()),
            },
            TuiProbeStep {
                at: Duration::from_millis(600),
                action: TuiProbeAction::Write(b"q\r".to_vec()),
            },
        ],
        required: spec.required,
        timeout: Duration::from_secs(4),
        cleanup_paths: Vec::new(),
    })
}

fn less_tui_probe(program_path: &Path) -> Result<TuiProbe, String> {
    let fixture_path = temp_fixture_path("less", "ansi.txt");
    let wrapper_path = temp_fixture_path("less-wrapper", "sh");
    fs::write(
        &fixture_path,
        "\u{1b}[31mNOCTRAIL_LESS_RED\u{1b}[0m\nNOCTRAIL_LESS_BODY\n",
    )
    .map_err(|error| format!("failed to write less fixture: {error}"))?;
    fs::write(
        &wrapper_path,
        "#!/bin/sh\nprintf '\\033[?1049h'\n\"$1\" -R \"$2\"\nstatus=$?\nprintf '\\033[?1049l'\nexit \"$status\"\n",
    )
    .map_err(|error| format!("failed to write less wrapper: {error}"))?;
    make_executable_path(&wrapper_path)?;

    let mut command = noctrail_pty::PtyCommand::new(wrapper_path.as_os_str());
    command.arg(program_path.as_os_str()).arg(&fixture_path);
    command.cwd_path(
        env::current_dir()
            .map_err(|error| format!("failed to resolve current working directory: {error}"))?,
    );

    Ok(TuiProbe {
        target: "less",
        source: format!("program:{}", program_path.display()),
        command,
        initial_size: PtySize::new(80, 24),
        steps: vec![
            TuiProbeStep {
                at: Duration::from_millis(200),
                action: TuiProbeAction::Resize(PtySize::new(100, 30)),
            },
            TuiProbeStep {
                at: Duration::from_millis(350),
                action: TuiProbeAction::Write(b"r".to_vec()),
            },
            TuiProbeStep {
                at: Duration::from_millis(500),
                action: TuiProbeAction::Write(b"q".to_vec()),
            },
        ],
        required: &[
            TuiCapability::AltScreen,
            TuiCapability::Resize,
            TuiCapability::Color,
        ],
        timeout: Duration::from_secs(4),
        cleanup_paths: vec![fixture_path, wrapper_path],
    })
}

fn run_tui_probe(probe: &TuiProbe) -> Result<ObservedCapabilities, String> {
    let mut session = PtySession::spawn(probe.command.clone(), probe.initial_size)
        .map_err(|error| format!("failed to spawn {} probe: {error}", probe.target))?;
    let reader = session
        .clone_output_reader()
        .map_err(|error| format!("failed to clone {} probe reader: {error}", probe.target))?;
    let (tx, rx) = mpsc::channel();
    let reader_handle = thread::spawn(move || pump_tui_output(reader, tx));
    let mut terminal = TerminalState::new(
        usize::from(probe.initial_size.cols),
        usize::from(probe.initial_size.rows),
    );
    let _ = terminal.grid_mut().take_dirty_rows();
    let mut observed = ObservedCapabilities::default();
    let started_at = Instant::now();
    let mut next_step = 0;
    let mut saw_resize = false;
    let mut exit_seen = false;

    while started_at.elapsed() <= probe.timeout {
        while next_step < probe.steps.len() && started_at.elapsed() >= probe.steps[next_step].at {
            match &probe.steps[next_step].action {
                TuiProbeAction::Write(bytes) => {
                    session.write(bytes).map_err(|error| {
                        format!("failed to write {} probe input: {error}", probe.target)
                    })?;
                }
                TuiProbeAction::Resize(size) => {
                    session.resize(*size).map_err(|error| {
                        format!("failed to resize {} probe: {error}", probe.target)
                    })?;
                    terminal.resize(usize::from(size.cols), usize::from(size.rows));
                    saw_resize = true;
                }
            }
            next_step += 1;
        }

        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(TuiProbeReaderEvent::Bytes(bytes)) => {
                let result = terminal.advance_bytes(&bytes);
                if result.alternate_screen_changed || terminal.snapshot().alternate_screen {
                    observed.alt_screen = true;
                }
                if terminal.mouse_tracking_mode() != MouseTrackingMode::Disabled
                    || terminal.sgr_mouse_mode()
                {
                    observed.mouse = true;
                }
                if terminal_snapshot_has_color(&terminal.snapshot()) {
                    observed.color = true;
                }
                if saw_resize {
                    observed.resize = true;
                }
            }
            Ok(TuiProbeReaderEvent::Eof) => {
                exit_seen = session.try_wait().ok().flatten().is_some();
                break;
            }
            Ok(TuiProbeReaderEvent::Error(error)) => {
                let _ = session.close();
                let _ = reader_handle.join();
                return Err(format!("{} probe reader error: {error}", probe.target));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if session.try_wait().ok().flatten().is_some() {
                    exit_seen = true;
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                exit_seen = true;
                break;
            }
        }
    }

    let status = session.close().ok().flatten();
    let _ = reader_handle.join();
    exit_seen |= status.is_some();

    if !exit_seen {
        return Err(format!(
            "{} probe timed out after {:?}",
            probe.target, probe.timeout
        ));
    }

    Ok(observed)
}

#[derive(Debug)]
enum TuiProbeReaderEvent {
    Bytes(Vec<u8>),
    Eof,
    Error(String),
}

fn pump_tui_output(
    mut reader: noctrail_pty::PtyOutputReader,
    tx: mpsc::Sender<TuiProbeReaderEvent>,
) {
    let mut buf = [0_u8; 4096];

    loop {
        match std::io::Read::read(&mut reader, &mut buf) {
            Ok(0) => {
                let _ = tx.send(TuiProbeReaderEvent::Eof);
                break;
            }
            Ok(count) => {
                if tx
                    .send(TuiProbeReaderEvent::Bytes(buf[..count].to_vec()))
                    .is_err()
                {
                    break;
                }
            }
            Err(error) => {
                let _ = tx.send(TuiProbeReaderEvent::Error(error.to_string()));
                break;
            }
        }
    }
}

fn cleanup_tui_probe(probe: &TuiProbe) {
    for path in &probe.cleanup_paths {
        let _ = fs::remove_file(path);
    }
}

fn terminal_snapshot_has_color(snapshot: &TerminalSnapshot) -> bool {
    snapshot.rows.iter().any(row_has_color) || snapshot.scrollback.iter().any(row_has_color)
}

fn row_has_color(row: &ScreenRowSnapshot) -> bool {
    row.cells.iter().any(|cell| {
        cell.style.foreground != Color::Default || cell.style.background != Color::Default
    })
}

fn format_tui_report(report: &TuiProbeReport) -> String {
    let required = if report.required.is_empty() {
        "none".to_string()
    } else {
        report.required.join(",")
    };
    let observed = {
        let labels = report.observed.labels();
        if labels.is_empty() {
            "none".to_string()
        } else {
            labels.join(",")
        }
    };

    match &report.status {
        TuiProbeStatus::Passed => format!(
            "pass {} source={} required={} observed={}",
            report.target, report.source, required, observed
        ),
        TuiProbeStatus::Skipped(reason) => format!(
            "skip {} source={} required={} reason={}",
            report.target, report.source, required, reason
        ),
    }
}

fn select_tui_targets(
    specs: &'static [TuiTargetSpec],
    filters: &[String],
) -> Result<Vec<&'static TuiTargetSpec>, String> {
    if filters.is_empty() {
        return Ok(specs.iter().collect());
    }

    let mut selected = Vec::new();
    for filter in filters {
        let Some(spec) = specs
            .iter()
            .find(|spec| spec.name == filter || spec.aliases.iter().any(|alias| alias == filter))
        else {
            return Err(format!("unknown TUI target: {filter}"));
        };
        if !selected
            .iter()
            .any(|existing: &&TuiTargetSpec| existing.name == spec.name)
        {
            selected.push(spec);
        }
    }

    Ok(selected)
}

fn tui_target_specs() -> &'static [TuiTargetSpec] {
    &[
        TuiTargetSpec {
            name: "nvim",
            aliases: &[],
            override_env: "NOCTRAIL_TUI_NVIM",
            program_candidates: &[],
            required: &[
                TuiCapability::AltScreen,
                TuiCapability::Mouse,
                TuiCapability::Resize,
                TuiCapability::Color,
            ],
            actual_probe: None,
            skip_hint: "set NOCTRAIL_TUI_NVIM to a scripted probe",
        },
        TuiTargetSpec {
            name: "tmux",
            aliases: &[],
            override_env: "NOCTRAIL_TUI_TMUX",
            program_candidates: &[],
            required: &[
                TuiCapability::AltScreen,
                TuiCapability::Mouse,
                TuiCapability::Resize,
                TuiCapability::Color,
            ],
            actual_probe: None,
            skip_hint: "set NOCTRAIL_TUI_TMUX to a scripted probe",
        },
        TuiTargetSpec {
            name: "fzf",
            aliases: &[],
            override_env: "NOCTRAIL_TUI_FZF",
            program_candidates: &[],
            required: &[
                TuiCapability::AltScreen,
                TuiCapability::Mouse,
                TuiCapability::Resize,
                TuiCapability::Color,
            ],
            actual_probe: None,
            skip_hint: "set NOCTRAIL_TUI_FZF to a scripted probe",
        },
        TuiTargetSpec {
            name: "less",
            aliases: &[],
            override_env: "NOCTRAIL_TUI_LESS",
            program_candidates: &["less"],
            required: &[
                TuiCapability::AltScreen,
                TuiCapability::Resize,
                TuiCapability::Color,
            ],
            actual_probe: Some(less_tui_probe),
            skip_hint: "install less or set NOCTRAIL_TUI_LESS to a scripted probe",
        },
        TuiTargetSpec {
            name: "top/htop",
            aliases: &["top", "htop"],
            override_env: "NOCTRAIL_TUI_TOP_HTOP",
            program_candidates: &[],
            required: &[
                TuiCapability::AltScreen,
                TuiCapability::Resize,
                TuiCapability::Color,
            ],
            actual_probe: None,
            skip_hint: "set NOCTRAIL_TUI_TOP_HTOP to a scripted probe",
        },
        TuiTargetSpec {
            name: "ssh",
            aliases: &[],
            override_env: "NOCTRAIL_TUI_SSH",
            program_candidates: &[],
            required: &[TuiCapability::Resize, TuiCapability::Color],
            actual_probe: None,
            skip_hint: "set NOCTRAIL_TUI_SSH to a scripted probe",
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(windows))]
    use std::sync::{Mutex, OnceLock};

    #[test]
    fn glob_detection_matches_shell_like_patterns() {
        assert!(crate::commands::render::contains_glob_meta(
            "tests/fixtures/*.json"
        ));
        assert!(crate::commands::render::contains_glob_meta(
            "tests/fixtures/[ab].json"
        ));
        assert!(!crate::commands::render::contains_glob_meta(
            "tests/fixtures/core.ntrec"
        ));
    }

    #[test]
    fn render_smoke_succeeds() {
        run_render_smoke().expect("render smoke should pass");
    }

    #[test]
    fn render_fixtures_succeed() {
        run_render_fixtures(&[]).expect("render fixtures should pass");
    }

    #[test]
    fn pty_smoke_probe_contains_sentinel() {
        let probe = crate::commands::pty::pty_smoke_probe("NOCTRAIL_PTY_SMOKE")
            .expect("pty smoke probe should build");
        let script = String::from_utf8(probe.input).expect("probe input should be utf-8");
        assert!(script.contains("NOCTRAIL_PTY_SMOKE"));
    }

    #[test]
    fn tui_target_filter_accepts_aliases() {
        let selected = select_tui_targets(
            tui_target_specs(),
            &[String::from("htop"), String::from("ssh")],
        )
        .expect("filters should resolve");

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].name, "top/htop");
        assert_eq!(selected[1].name, "ssh");
    }

    #[test]
    fn prompt_target_filter_accepts_named_targets() {
        let specs = prompt_target_specs();
        let selected = select_prompt_targets(
            &specs,
            &[String::from("starship"), String::from("powerlevel10k")],
        )
        .expect("filters should resolve");

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].name, "starship");
        assert_eq!(selected[1].name, "powerlevel10k");
    }

    #[cfg(not(windows))]
    #[test]
    fn tui_matrix_accepts_override_probe() {
        let _guard = env_test_lock()
            .lock()
            .expect("env test lock should be available");
        let script_path = temp_fixture_path("tui-override-test", "sh");
        fs::write(
            &script_path,
            "#!/bin/sh\ntrap 'printf \"\\033[38;2;255;0;0mRESIZED\\033[0m\\n\"' WINCH\nprintf '\\033[?1049h\\033[?1000h\\033[?1006h\\033[38;2;255;0;0mHELLO\\033[0m\\n'\nwhile IFS= read -r line; do\n  [ \"$line\" = q ] && break\n  printf '\\033[38;2;0;255;0m%s\\033[0m\\n' \"$line\"\ndone\nprintf '\\033[?1049l'\n",
        )
        .expect("script should write");
        make_executable(&script_path);

        unsafe {
            env::set_var("NOCTRAIL_TUI_NVIM", &script_path);
        }

        let result = run_tui_matrix(&[String::from("nvim")]);

        unsafe {
            env::remove_var("NOCTRAIL_TUI_NVIM");
        }
        let _ = fs::remove_file(&script_path);

        result.expect("override probe should pass");
    }

    #[cfg(not(windows))]
    #[test]
    fn prompt_matrix_builtin_emulations_pass() {
        run_prompt_matrix(&[]).expect("builtin prompt probes should pass");
    }

    #[cfg(not(windows))]
    fn env_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[cfg(not(windows))]
    fn make_executable(path: &Path) {
        make_executable_path(path).expect("script should be executable");
    }
}
