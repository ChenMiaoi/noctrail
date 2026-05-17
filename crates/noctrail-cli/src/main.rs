use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    sync::mpsc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use noctrail_pty::ShellSource;
use noctrail_pty::{PtySession, PtySize, ResolvedShell};
use noctrail_render::{
    FontDiagnostics, FontFamilyDiagnostics, FontPreferences, GlyphRasterConfig, PaintLayer,
    PaneBorderStyle, RenderBackend, RenderPlan, RenderRect, Rgba, prepare_render_frame,
    probe_font_diagnostics, probe_gpu_backend,
};
use noctrail_runtime::{PaneId, PaneRuntimeRegistry, RuntimeCommand, RuntimeEvent};
use noctrail_term::recording::replay_recording_file;
use noctrail_term::{
    Cell, Color, Cursor, DamageSet, LineEnding, MouseTrackingMode, Position, ScreenRowSnapshot,
    Selection, SelectionMode, Style, TerminalSnapshot, TerminalState,
};
use serde::Deserialize;

const HELP: &str = "\
Noctrail development CLI

Usage:
  noctrail [command]

Commands:
  doctor      Print basic environment diagnostics
  doctor shell  Print shell resolution diagnostics
  doctor gpu  Print GPU backend diagnostics
  doctor font Print font fallback diagnostics
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
                Some("gpu") => {
                    if let Err(error) = print_doctor_gpu() {
                        eprintln!("{error}");
                        process::exit(1);
                    }
                }
                Some("font") => print_doctor_font(),
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

fn print_doctor() {
    println!("noctrail {}", env!("CARGO_PKG_VERSION"));
    println!("target: {}", env::consts::OS);
    println!("arch: {}", env::consts::ARCH);
    println!("hint: run `noctrail doctor shell` for shell diagnostics");
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnicodeCapability {
    Input,
    Selection,
    Copy,
    Cursor,
}

impl UnicodeCapability {
    const fn label(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Selection => "selection",
            Self::Copy => "copy",
            Self::Cursor => "cursor",
        }
    }
}

type UnicodeProbe = fn() -> ObservedUnicodeCapabilities;

#[derive(Debug, Clone)]
struct UnicodeTargetSpec {
    name: &'static str,
    probe: UnicodeProbe,
    required: &'static [UnicodeCapability],
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ObservedUnicodeCapabilities {
    input: bool,
    selection: bool,
    copy: bool,
    cursor: bool,
}

impl ObservedUnicodeCapabilities {
    fn contains(self, capability: UnicodeCapability) -> bool {
        match capability {
            UnicodeCapability::Input => self.input,
            UnicodeCapability::Selection => self.selection,
            UnicodeCapability::Copy => self.copy,
            UnicodeCapability::Cursor => self.cursor,
        }
    }

    fn labels(self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        for capability in [
            UnicodeCapability::Input,
            UnicodeCapability::Selection,
            UnicodeCapability::Copy,
            UnicodeCapability::Cursor,
        ] {
            if self.contains(capability) {
                labels.push(capability.label());
            }
        }
        labels
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnicodeProbeReport {
    target: &'static str,
    observed: ObservedUnicodeCapabilities,
    required: Vec<&'static str>,
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
enum ShellCapability {
    Startup,
    Input,
    Exit,
    Cwd,
}

impl ShellCapability {
    const fn label(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::Input => "input",
            Self::Exit => "exit",
            Self::Cwd => "cwd",
        }
    }
}

#[derive(Debug, Clone)]
struct ShellTargetSpec {
    name: &'static str,
    aliases: &'static [&'static str],
    override_env: &'static str,
    program_candidates: &'static [&'static str],
    required: &'static [ShellCapability],
    actual_probe: Option<ShellActualProbe>,
    skip_hint: &'static str,
}

type ShellActualProbe = fn(&Path) -> Result<ShellProbe, String>;

#[derive(Debug, Clone)]
struct ShellProbe {
    target: &'static str,
    source: String,
    command: noctrail_pty::PtyCommand,
    initial_size: PtySize,
    input: Vec<u8>,
    marker: String,
    expected_cwd: String,
    required: &'static [ShellCapability],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ObservedShellCapabilities {
    startup: bool,
    input: bool,
    exit: bool,
    cwd: bool,
}

impl ObservedShellCapabilities {
    fn contains(self, capability: ShellCapability) -> bool {
        match capability {
            ShellCapability::Startup => self.startup,
            ShellCapability::Input => self.input,
            ShellCapability::Exit => self.exit,
            ShellCapability::Cwd => self.cwd,
        }
    }

    fn labels(self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        for capability in [
            ShellCapability::Startup,
            ShellCapability::Input,
            ShellCapability::Exit,
            ShellCapability::Cwd,
        ] {
            if self.contains(capability) {
                labels.push(capability.label());
            }
        }
        labels
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ShellProbeStatus {
    Passed,
    Skipped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ShellProbeReport {
    target: &'static str,
    source: String,
    observed: ObservedShellCapabilities,
    required: Vec<&'static str>,
    status: ShellProbeStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PromptCapability {
    Layout,
    Escape,
}

impl PromptCapability {
    const fn label(self) -> &'static str {
        match self {
            Self::Layout => "layout",
            Self::Escape => "escape",
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
    input_line: String,
    input_row: usize,
    expected_result: String,
    required: &'static [PromptCapability],
    cleanup_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct ObservedPromptCapabilities {
    layout: bool,
    escape: bool,
}

impl ObservedPromptCapabilities {
    fn contains(self, capability: PromptCapability) -> bool {
        match capability {
            PromptCapability::Layout => self.layout,
            PromptCapability::Escape => self.escape,
        }
    }

    fn labels(self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        for capability in [PromptCapability::Layout, PromptCapability::Escape] {
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

fn run_shell_matrix(filters: &[String]) -> Result<(), String> {
    let specs = shell_target_specs();
    let selected = select_shell_targets(specs, filters)?;
    let mut ran_any = false;
    let mut failures = Vec::new();

    for spec in selected {
        match run_shell_target(spec) {
            Ok(report) => {
                println!("{}", format_shell_report(&report));
                if matches!(report.status, ShellProbeStatus::Passed) {
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
        println!("shell-matrix: all selected targets were skipped");
    } else {
        println!("shell matrix ok");
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

fn run_unicode_matrix(filters: &[String]) -> Result<(), String> {
    let specs = unicode_target_specs();
    let selected = select_unicode_targets(&specs, filters)?;

    for spec in selected {
        let report = run_unicode_target(spec)?;
        println!("{}", format_unicode_report(&report));
    }

    println!("unicode matrix ok");
    Ok(())
}

fn run_unicode_target(spec: &UnicodeTargetSpec) -> Result<UnicodeProbeReport, String> {
    let observed = (spec.probe)();
    let missing = spec
        .required
        .iter()
        .filter(|capability| !observed.contains(**capability))
        .map(|capability| capability.label())
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(format!(
            "missing capabilities [{}] from {}",
            missing.join(", "),
            spec.name
        ));
    }

    Ok(UnicodeProbeReport {
        target: spec.name,
        observed,
        required: spec.required.iter().map(|cap| cap.label()).collect(),
    })
}

fn format_unicode_report(report: &UnicodeProbeReport) -> String {
    let required = report.required.join(",");
    let observed = report.observed.labels().join(",");
    format!(
        "pass {} required={} observed={}",
        report.target, required, observed
    )
}

fn select_unicode_targets<'a>(
    specs: &'a [UnicodeTargetSpec],
    filters: &[String],
) -> Result<Vec<&'a UnicodeTargetSpec>, String> {
    if filters.is_empty() {
        return Ok(specs.iter().collect());
    }

    let mut selected = Vec::new();
    for filter in filters {
        let Some(spec) = specs.iter().find(|spec| spec.name == filter) else {
            return Err(format!("unknown unicode target: {filter}"));
        };
        if !selected
            .iter()
            .any(|existing: &&UnicodeTargetSpec| existing.name == spec.name)
        {
            selected.push(spec);
        }
    }

    Ok(selected)
}

fn unicode_target_specs() -> Vec<UnicodeTargetSpec> {
    vec![
        UnicodeTargetSpec {
            name: "cjk",
            probe: probe_cjk_unicode,
            required: &[
                UnicodeCapability::Input,
                UnicodeCapability::Selection,
                UnicodeCapability::Copy,
                UnicodeCapability::Cursor,
            ],
        },
        UnicodeTargetSpec {
            name: "emoji",
            probe: probe_emoji_unicode,
            required: &[
                UnicodeCapability::Input,
                UnicodeCapability::Selection,
                UnicodeCapability::Copy,
                UnicodeCapability::Cursor,
            ],
        },
        UnicodeTargetSpec {
            name: "combining",
            probe: probe_combining_unicode,
            required: &[
                UnicodeCapability::Input,
                UnicodeCapability::Selection,
                UnicodeCapability::Copy,
                UnicodeCapability::Cursor,
            ],
        },
        UnicodeTargetSpec {
            name: "fullwidth",
            probe: probe_fullwidth_unicode,
            required: &[
                UnicodeCapability::Input,
                UnicodeCapability::Selection,
                UnicodeCapability::Copy,
                UnicodeCapability::Cursor,
            ],
        },
    ]
}

fn probe_cjk_unicode() -> ObservedUnicodeCapabilities {
    let mut terminal = TerminalState::new(8, 2);
    terminal.advance_str("中a");
    verify_unicode_case(
        &mut terminal,
        Selection {
            mode: SelectionMode::Normal,
            start: Position { row: 0, col: 0 },
            end: Position { row: 0, col: 2 },
        },
        "中a",
        "中a",
        Cursor { row: 0, col: 3 },
        |snapshot| {
            let Some(row) = snapshot.rows.first() else {
                return false;
            };
            row.cells.first().is_some_and(|cell| cell.text == "中")
                && row.cells.get(1).is_some_and(|cell| cell.wide_continuation)
        },
    )
}

fn probe_emoji_unicode() -> ObservedUnicodeCapabilities {
    let mut terminal = TerminalState::new(8, 2);
    terminal.advance_str("🙂x");
    verify_unicode_case(
        &mut terminal,
        Selection {
            mode: SelectionMode::Normal,
            start: Position { row: 0, col: 0 },
            end: Position { row: 0, col: 2 },
        },
        "🙂x",
        "🙂x",
        Cursor { row: 0, col: 3 },
        |snapshot| {
            let Some(row) = snapshot.rows.first() else {
                return false;
            };
            row.cells.first().is_some_and(|cell| cell.text == "🙂")
                && row.cells.get(1).is_some_and(|cell| cell.wide_continuation)
        },
    )
}

fn probe_combining_unicode() -> ObservedUnicodeCapabilities {
    let mut terminal = TerminalState::new(8, 2);
    terminal.advance_char('e');
    terminal.advance_char('\u{301}');
    terminal.advance_char('x');
    verify_unicode_case(
        &mut terminal,
        Selection {
            mode: SelectionMode::Normal,
            start: Position { row: 0, col: 0 },
            end: Position { row: 0, col: 1 },
        },
        "e\u{301}x",
        "e\u{301}x",
        Cursor { row: 0, col: 2 },
        |snapshot| {
            snapshot.rows.first().is_some_and(|row| {
                row.cells
                    .first()
                    .is_some_and(|cell| cell.text == "e\u{301}")
            })
        },
    )
}

fn probe_fullwidth_unicode() -> ObservedUnicodeCapabilities {
    let mut terminal = TerminalState::new(8, 2);
    terminal.advance_str("Ａb");
    verify_unicode_case(
        &mut terminal,
        Selection {
            mode: SelectionMode::Normal,
            start: Position { row: 0, col: 0 },
            end: Position { row: 0, col: 2 },
        },
        "Ａb",
        "Ａb",
        Cursor { row: 0, col: 3 },
        |snapshot| {
            let Some(row) = snapshot.rows.first() else {
                return false;
            };
            row.cells.first().is_some_and(|cell| cell.text == "Ａ")
                && row.cells.get(1).is_some_and(|cell| cell.wide_continuation)
        },
    )
}

fn verify_unicode_case(
    terminal: &mut TerminalState,
    selection: Selection,
    expected_render: &str,
    expected_copy: &str,
    expected_cursor: Cursor,
    input_matches: impl Fn(&TerminalSnapshot) -> bool,
) -> ObservedUnicodeCapabilities {
    let snapshot = terminal.snapshot();
    let input = snapshot
        .rows
        .first()
        .is_some_and(|row| row.rendered_text().starts_with(expected_render))
        && input_matches(&snapshot);
    let cursor = snapshot.cursor == expected_cursor;

    let normalized = selection.clone().normalized();
    terminal.set_selection(Some(selection));
    let selection_snapshot = terminal.snapshot();
    let selection_seen = selection_snapshot.selection.as_ref() == Some(&normalized);
    let copy = terminal.selection_text(LineEnding::Lf).as_deref() == Some(expected_copy);

    ObservedUnicodeCapabilities {
        input,
        selection: selection_seen,
        copy,
        cursor,
    }
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
    let mut prompt_ready = false;
    let mut exit_seen = false;
    let started_at = Instant::now();
    let timeout = Duration::from_secs(4);

    while started_at.elapsed() <= timeout {
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(TuiProbeReaderEvent::Bytes(bytes)) => {
                terminal.advance_bytes(&bytes);
                if !input_sent && prompt_is_ready(&terminal.snapshot(), probe) {
                    prompt_ready = true;
                    session
                        .write(probe.input_line.as_bytes())
                        .map_err(|error| {
                            format!("failed to write {} prompt input: {error}", probe.target)
                        })?;
                    session.write(b"\r").map_err(|error| {
                        format!("failed to submit {} prompt input: {error}", probe.target)
                    })?;
                    input_sent = true;
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

    let snapshot = terminal.snapshot();
    let rendered_rows = snapshot
        .rows
        .iter()
        .map(ScreenRowSnapshot::rendered_text)
        .collect::<Vec<_>>();
    observed.layout = prompt_ready && prompt_layout_matches(&rendered_rows, probe);
    observed.escape = prompt_escape_is_clean(&snapshot, &rendered_rows, probe);

    Ok(observed)
}

fn prompt_is_ready(snapshot: &TerminalSnapshot, probe: &PromptProbe) -> bool {
    if snapshot.cursor.row != probe.input_row {
        return false;
    }
    if snapshot.cursor.col != probe.prompt_lines.last().map_or(0, String::len) {
        return false;
    }

    prompt_rows_match(
        &snapshot
            .rows
            .iter()
            .map(ScreenRowSnapshot::rendered_text)
            .collect::<Vec<_>>(),
        probe,
    )
}

fn prompt_layout_matches(rendered_rows: &[String], probe: &PromptProbe) -> bool {
    prompt_rows_match(rendered_rows, probe)
        && rendered_rows.get(probe.input_row).is_some_and(|row| {
            row.starts_with(&format!(
                "{}{}",
                probe.prompt_lines[probe.input_row], probe.input_line
            ))
        })
        && rendered_rows
            .iter()
            .any(|row| row.contains(&probe.expected_result))
}

fn prompt_rows_match(rendered_rows: &[String], probe: &PromptProbe) -> bool {
    probe
        .prompt_lines
        .iter()
        .enumerate()
        .all(|(index, expected)| {
            rendered_rows
                .get(index)
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

    prompt_rows_match(rendered_rows, probe) && prompt_has_non_default_style(snapshot, probe)
}

fn prompt_has_non_default_style(snapshot: &TerminalSnapshot, probe: &PromptProbe) -> bool {
    probe
        .prompt_lines
        .iter()
        .enumerate()
        .all(|(row_index, expected)| {
            let Some(row) = snapshot.rows.get(row_index) else {
                return false;
            };
            row.cells
                .iter()
                .take(expected.len())
                .filter(|cell| !cell.text.is_empty())
                .any(|cell| !cell.style.is_default())
        })
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

fn prompt_target_specs() -> Vec<PromptTargetSpec> {
    vec![
        PromptTargetSpec {
            name: "starship",
            build_probe: prompt_probe_builder(starship_prompt_probe),
            required: &[PromptCapability::Layout, PromptCapability::Escape],
            skip_hint: "prompt emulation is unavailable on this platform",
        },
        PromptTargetSpec {
            name: "oh-my-zsh",
            build_probe: prompt_probe_builder(oh_my_zsh_prompt_probe),
            required: &[PromptCapability::Layout, PromptCapability::Escape],
            skip_hint: "prompt emulation is unavailable on this platform",
        },
        PromptTargetSpec {
            name: "powerlevel10k",
            build_probe: prompt_probe_builder(powerlevel10k_prompt_probe),
            required: &[PromptCapability::Layout, PromptCapability::Escape],
            skip_hint: "prompt emulation is unavailable on this platform",
        },
    ]
}

#[cfg(not(windows))]
fn prompt_probe_builder(builder: PromptProbeBuilder) -> Option<PromptProbeBuilder> {
    Some(builder)
}

#[cfg(windows)]
fn prompt_probe_builder(_builder: PromptProbeBuilder) -> Option<PromptProbeBuilder> {
    None
}

#[cfg(not(windows))]
fn starship_prompt_probe() -> Result<PromptProbe, String> {
    prompt_script_probe(
        "starship",
        "printf '\\033[32mSTARSHIP\\033[0m \\033[34mPROMPT\\033[0m > '\n",
        vec!["STARSHIP PROMPT > ".to_string()],
        0,
        "status",
    )
}

#[cfg(not(windows))]
fn oh_my_zsh_prompt_probe() -> Result<PromptProbe, String> {
    prompt_script_probe(
        "oh-my-zsh",
        "printf '\\033[35mOHMYZSH\\033[0m \\033[33m%%\\033[0m '\n",
        vec!["OHMYZSH % ".to_string()],
        0,
        "pwd",
    )
}

#[cfg(not(windows))]
fn powerlevel10k_prompt_probe() -> Result<PromptProbe, String> {
    prompt_script_probe(
        "powerlevel10k",
        "printf '\\033[36mP10K-L1\\033[0m\\n\\033[35mP10K>\\033[0m '\n",
        vec!["P10K-L1".to_string(), "P10K> ".to_string()],
        1,
        "build",
    )
}

#[cfg(not(windows))]
fn prompt_script_probe(
    target: &'static str,
    prompt_body: &str,
    prompt_lines: Vec<String>,
    input_row: usize,
    input_line: &str,
) -> Result<PromptProbe, String> {
    let script_path = temp_fixture_path(target, "sh");
    fs::write(
        &script_path,
        format!(
            "#!/bin/sh\n{prompt_body}IFS= read -r line || exit 1\nprintf '\\nRESULT:%s\\n' \"$line\"\n"
        ),
    )
    .map_err(|error| format!("failed to write {target} prompt script: {error}"))?;
    make_executable_path(&script_path)?;

    let mut command = noctrail_pty::PtyCommand::new("/bin/sh");
    command.arg(&script_path);
    command.cwd_path(
        env::current_dir()
            .map_err(|error| format!("failed to resolve current working directory: {error}"))?,
    );

    Ok(PromptProbe {
        target,
        source: format!("builtin:{}", script_path.display()),
        command,
        initial_size: PtySize::new(120, 24),
        prompt_lines,
        input_line: input_line.to_string(),
        input_row,
        expected_result: format!("RESULT:{input_line}"),
        required: &[PromptCapability::Layout, PromptCapability::Escape],
        cleanup_paths: vec![script_path],
    })
}

fn run_shell_target(spec: &'static ShellTargetSpec) -> Result<ShellProbeReport, String> {
    let Some(probe) = resolve_shell_probe(spec)? else {
        return Ok(ShellProbeReport {
            target: spec.name,
            source: "unavailable".to_string(),
            observed: ObservedShellCapabilities::default(),
            required: spec.required.iter().map(|cap| cap.label()).collect(),
            status: ShellProbeStatus::Skipped(spec.skip_hint.to_string()),
        });
    };

    let observed = run_shell_probe(&probe)?;
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

    Ok(ShellProbeReport {
        target: probe.target,
        source: probe.source,
        observed,
        required: probe.required.iter().map(|cap| cap.label()).collect(),
        status: ShellProbeStatus::Passed,
    })
}

fn resolve_shell_probe(spec: &'static ShellTargetSpec) -> Result<Option<ShellProbe>, String> {
    if let Some(override_path) = env::var_os(spec.override_env) {
        return Ok(Some(override_shell_probe(
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

fn override_shell_probe(
    spec: &'static ShellTargetSpec,
    program_path: PathBuf,
    source_label: &str,
) -> Result<ShellProbe, String> {
    let cwd = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let marker = format!(
        "NOCTRAIL_SHELL_MATRIX_{}",
        spec.name.replace('/', "_").to_uppercase()
    );
    let mut command = noctrail_pty::PtyCommand::new(program_path.as_os_str());
    command.cwd_path(&cwd);

    Ok(ShellProbe {
        target: spec.name,
        source: format!("override:{source_label}"),
        command,
        initial_size: PtySize::new(80, 24),
        input: format!("{marker}\rpwd\rexit\r").into_bytes(),
        marker,
        expected_cwd: cwd.display().to_string(),
        required: spec.required,
    })
}

fn bash_shell_probe(program_path: &Path) -> Result<ShellProbe, String> {
    unix_shell_probe(
        "bash",
        program_path,
        &["--noprofile", "--norc", "-i"],
        "NOCTRAIL_SHELL_MATRIX_BASH",
    )
}

fn zsh_shell_probe(program_path: &Path) -> Result<ShellProbe, String> {
    unix_shell_probe(
        "zsh",
        program_path,
        &["-f", "-i"],
        "NOCTRAIL_SHELL_MATRIX_ZSH",
    )
}

fn unix_shell_probe(
    target: &'static str,
    program_path: &Path,
    args: &[&str],
    marker: &str,
) -> Result<ShellProbe, String> {
    let cwd = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let mut command = noctrail_pty::PtyCommand::new(program_path.as_os_str());
    command.args(args.iter().copied());
    command.cwd_path(&cwd);

    Ok(ShellProbe {
        target,
        source: format!("program:{}", program_path.display()),
        command,
        initial_size: PtySize::new(80, 24),
        input: format!("printf '{marker}\\n'; pwd; exit\r").into_bytes(),
        marker: marker.to_string(),
        expected_cwd: cwd.display().to_string(),
        required: &[
            ShellCapability::Startup,
            ShellCapability::Input,
            ShellCapability::Exit,
            ShellCapability::Cwd,
        ],
    })
}

fn run_shell_probe(probe: &ShellProbe) -> Result<ObservedShellCapabilities, String> {
    let mut session = PtySession::spawn(probe.command.clone(), probe.initial_size)
        .map_err(|error| format!("failed to spawn {} probe: {error}", probe.target))?;
    let mut observed = ObservedShellCapabilities {
        startup: session.process_id().is_some(),
        ..ObservedShellCapabilities::default()
    };

    session
        .write(&probe.input)
        .map_err(|error| format!("failed to write {} probe input: {error}", probe.target))?;
    let output = read_all_output(&mut session)
        .map_err(|error| format!("failed to read {} probe output: {error}", probe.target))?;
    let status = session
        .close()
        .map_err(|error| format!("failed to close {} probe: {error}", probe.target))?;
    let haystack = String::from_utf8_lossy(&output);

    observed.input = haystack.contains(&probe.marker);
    observed.cwd = haystack.contains(&probe.expected_cwd);
    observed.exit = status.as_ref().is_some_and(|status| status.success());

    Ok(observed)
}

fn format_shell_report(report: &ShellProbeReport) -> String {
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
        ShellProbeStatus::Passed => format!(
            "pass {} source={} required={} observed={}",
            report.target, report.source, required, observed
        ),
        ShellProbeStatus::Skipped(reason) => format!(
            "skip {} source={} required={} reason={}",
            report.target, report.source, required, reason
        ),
    }
}

fn select_shell_targets(
    specs: &'static [ShellTargetSpec],
    filters: &[String],
) -> Result<Vec<&'static ShellTargetSpec>, String> {
    if filters.is_empty() {
        return Ok(specs.iter().collect());
    }

    let mut selected = Vec::new();
    for filter in filters {
        let Some(spec) = specs
            .iter()
            .find(|spec| spec.name == filter || spec.aliases.iter().any(|alias| alias == filter))
        else {
            return Err(format!("unknown shell target: {filter}"));
        };
        if !selected
            .iter()
            .any(|existing: &&ShellTargetSpec| existing.name == spec.name)
        {
            selected.push(spec);
        }
    }

    Ok(selected)
}

fn shell_target_specs() -> &'static [ShellTargetSpec] {
    &[
        ShellTargetSpec {
            name: "bash",
            aliases: &[],
            override_env: "NOCTRAIL_SHELL_BASH",
            program_candidates: &["bash"],
            required: &[
                ShellCapability::Startup,
                ShellCapability::Input,
                ShellCapability::Exit,
                ShellCapability::Cwd,
            ],
            actual_probe: Some(bash_shell_probe),
            skip_hint: "install bash or set NOCTRAIL_SHELL_BASH",
        },
        ShellTargetSpec {
            name: "zsh",
            aliases: &[],
            override_env: "NOCTRAIL_SHELL_ZSH",
            program_candidates: &["zsh"],
            required: &[
                ShellCapability::Startup,
                ShellCapability::Input,
                ShellCapability::Exit,
                ShellCapability::Cwd,
            ],
            actual_probe: Some(zsh_shell_probe),
            skip_hint: "install zsh or set NOCTRAIL_SHELL_ZSH",
        },
        ShellTargetSpec {
            name: "fish",
            aliases: &[],
            override_env: "NOCTRAIL_SHELL_FISH",
            program_candidates: &[],
            required: &[
                ShellCapability::Startup,
                ShellCapability::Input,
                ShellCapability::Exit,
                ShellCapability::Cwd,
            ],
            actual_probe: None,
            skip_hint: "set NOCTRAIL_SHELL_FISH to a scripted probe",
        },
        ShellTargetSpec {
            name: "pwsh",
            aliases: &[],
            override_env: "NOCTRAIL_SHELL_PWSH",
            program_candidates: &[],
            required: &[
                ShellCapability::Startup,
                ShellCapability::Input,
                ShellCapability::Exit,
                ShellCapability::Cwd,
            ],
            actual_probe: None,
            skip_hint: "set NOCTRAIL_SHELL_PWSH to a scripted probe",
        },
        ShellTargetSpec {
            name: "nu",
            aliases: &[],
            override_env: "NOCTRAIL_SHELL_NU",
            program_candidates: &[],
            required: &[
                ShellCapability::Startup,
                ShellCapability::Input,
                ShellCapability::Exit,
                ShellCapability::Cwd,
            ],
            actual_probe: None,
            skip_hint: "set NOCTRAIL_SHELL_NU to a scripted probe",
        },
        ShellTargetSpec {
            name: "cmd",
            aliases: &[],
            override_env: "NOCTRAIL_SHELL_CMD",
            program_candidates: &[],
            required: &[
                ShellCapability::Startup,
                ShellCapability::Input,
                ShellCapability::Exit,
                ShellCapability::Cwd,
            ],
            actual_probe: None,
            skip_hint: "set NOCTRAIL_SHELL_CMD to a scripted probe",
        },
        ShellTargetSpec {
            name: "WSL",
            aliases: &["wsl"],
            override_env: "NOCTRAIL_SHELL_WSL",
            program_candidates: &[],
            required: &[
                ShellCapability::Startup,
                ShellCapability::Input,
                ShellCapability::Exit,
                ShellCapability::Cwd,
            ],
            actual_probe: None,
            skip_hint: "set NOCTRAIL_SHELL_WSL to a scripted probe",
        },
    ]
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

fn find_executable(candidates: &[&str]) -> Option<PathBuf> {
    candidates
        .iter()
        .find_map(|candidate| find_executable_in_path(candidate))
}

fn find_executable_in_path(program: &str) -> Option<PathBuf> {
    let program_path = Path::new(program);
    if program_path.components().count() > 1 && program_path.is_file() {
        return Some(program_path.to_path_buf());
    }

    let path_value = env::var_os("PATH")?;

    #[cfg(windows)]
    let extensions = executable_extensions();
    #[cfg(not(windows))]
    let extensions = vec![String::new()];

    for directory in env::split_paths(&path_value) {
        for extension in &extensions {
            let candidate = if extension.is_empty() || program.contains('.') {
                directory.join(program)
            } else {
                directory.join(format!("{program}{extension}"))
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(windows)]
fn executable_extensions() -> Vec<String> {
    env::var_os("PATHEXT")
        .map(|value| {
            env::split_paths(&value)
                .map(|path| path.to_string_lossy().to_string())
                .collect::<Vec<_>>()
        })
        .filter(|extensions| !extensions.is_empty())
        .unwrap_or_else(|| vec![".exe".to_string(), ".bat".to_string(), ".cmd".to_string()])
}

fn temp_fixture_path(label: &str, extension: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after epoch")
        .as_nanos();
    env::temp_dir().join(format!("noctrail-{label}-{unique}.{extension}"))
}

fn make_executable_path(path: &Path) -> Result<(), String> {
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::PermissionsExt;

        let metadata = fs::metadata(path)
            .map_err(|error| format!("failed to stat {}: {error}", path.display()))?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)
            .map_err(|error| format!("failed to chmod {}: {error}", path.display()))?;
        Ok(())
    }

    #[cfg(windows)]
    {
        let _ = path;
        Ok(())
    }
}

fn print_doctor_gpu() -> Result<(), String> {
    let diagnostics = probe_gpu_backend().map_err(|error| error.to_string())?;
    println!("gpu.adapter={}", diagnostics.adapter_name);
    println!("gpu.backend={:?}", diagnostics.backend);
    println!("gpu.device_type={:?}", diagnostics.device_type);
    Ok(())
}

fn print_doctor_font() {
    let diagnostics = probe_font_diagnostics(&FontPreferences::default());

    println!("font.locale={}", diagnostics.locale);
    println!("font.primary.size={}", diagnostics.preferences.size);
    print_font_family("font.primary", &diagnostics.primary);

    for (index, fallback) in diagnostics.fallbacks.iter().enumerate() {
        print_font_family(&format!("font.fallback.{}", index + 1), fallback);
    }

    for sample in &diagnostics.samples {
        println!("font.sample.{}.text={}", sample.label, sample.text);
        println!(
            "font.sample.{}.status={}",
            sample.label,
            sample.status.label()
        );
        if sample.fonts.is_empty() {
            println!("font.sample.{}.fonts=(none)", sample.label);
        } else {
            println!(
                "font.sample.{}.fonts={}",
                sample.label,
                sample.fonts.join(", ")
            );
        }
        if sample.missing_glyphs.is_empty() {
            println!("font.sample.{}.missing=(none)", sample.label);
        } else {
            println!(
                "font.sample.{}.missing={}",
                sample.label,
                sample.missing_glyphs.join(" ")
            );
        }
    }

    print_font_logs(&diagnostics);
}

fn print_font_family(prefix: &str, diagnostics: &FontFamilyDiagnostics) {
    println!("{prefix}.requested={}", diagnostics.requested_family);
    println!("{prefix}.resolution={}", diagnostics.resolution.label());
    match &diagnostics.resolved_family {
        Some(family) => println!("{prefix}.resolved_family={family}"),
        None => println!("{prefix}.resolved_family=(missing)"),
    }
    match &diagnostics.resolved_post_script_name {
        Some(name) => println!("{prefix}.resolved_postscript={name}"),
        None => println!("{prefix}.resolved_postscript=(missing)"),
    }
    match diagnostics.monospaced {
        Some(monospaced) => println!("{prefix}.monospaced={monospaced}"),
        None => println!("{prefix}.monospaced=(unknown)"),
    }
}

fn print_font_logs(diagnostics: &FontDiagnostics) {
    if diagnostics.logs.is_empty() {
        println!("font.logs=(none)");
        return;
    }

    for (index, log) in diagnostics.logs.iter().enumerate() {
        println!("font.log.{}={log}", index + 1);
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
                let path = canonicalize_fixture_path(path)?;
                paths.push(path);
            }
        } else {
            paths.push(canonicalize_fixture_path(PathBuf::from(pattern))?);
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

#[derive(Debug, Deserialize)]
struct RenderFixture {
    surface: RenderFixtureRect,
    #[serde(default)]
    backend: FixtureBackend,
    #[serde(default = "default_active")]
    active: bool,
    snapshot: TerminalSnapshot,
    damage: RenderFixtureDamage,
    #[serde(default)]
    border: FixtureBorder,
    #[serde(default)]
    glyph_raster: FixtureGlyphRaster,
    expect: RenderFixtureExpect,
}

#[derive(Debug, Deserialize)]
struct RenderFixtureRect {
    #[serde(default)]
    x: usize,
    #[serde(default)]
    y: usize,
    width: usize,
    height: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
enum FixtureBackend {
    Gpu,
    #[default]
    Software,
}

#[derive(Debug, Deserialize)]
struct RenderFixtureDamage {
    dirty_rows: Vec<usize>,
    #[serde(default)]
    full_frame: bool,
}

#[derive(Debug, Deserialize)]
struct FixtureBorder {
    #[serde(default)]
    width: usize,
    #[serde(default = "default_active_border")]
    active: FixtureColor,
    #[serde(default = "default_inactive_border")]
    inactive: FixtureColor,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
struct FixtureColor(HexColor);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
struct HexColor {
    red: u8,
    green: u8,
    blue: u8,
    alpha: u8,
}

impl Default for FixtureBorder {
    fn default() -> Self {
        Self {
            width: 0,
            active: default_active_border(),
            inactive: default_inactive_border(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct FixtureGlyphRaster {
    #[serde(default = "default_scale")]
    scale: f32,
    #[serde(default = "default_cell_width")]
    cell_width: f32,
    #[serde(default = "default_line_height")]
    line_height: f32,
}

impl Default for FixtureGlyphRaster {
    fn default() -> Self {
        Self {
            scale: default_scale(),
            cell_width: default_cell_width(),
            line_height: default_line_height(),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct RenderFixtureExpect {
    #[serde(default)]
    prepared_rows: Vec<usize>,
    glyph_rows: Option<Vec<usize>>,
    raster_jobs: Option<usize>,
    glyphs_prepared: Option<usize>,
    paint_rects: Option<usize>,
    full_frame: Option<bool>,
    background_rects: Option<Vec<ExpectedRect>>,
    selection_rects: Option<Vec<ExpectedRect>>,
    underline_rects: Option<Vec<ExpectedRect>>,
    cursor_rects: Option<Vec<ExpectedRect>>,
    border_segments: Option<Vec<ExpectedBorderSegment>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ExpectedRect {
    row: usize,
    col: usize,
    span: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct ExpectedBorderSegment {
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    color: FixtureColor,
}

fn default_scale() -> f32 {
    1.0
}

fn default_active() -> bool {
    true
}

fn default_cell_width() -> f32 {
    14.0
}

fn default_line_height() -> f32 {
    19.6
}

fn default_active_border() -> FixtureColor {
    FixtureColor(HexColor {
        red: 0x7a,
        green: 0xa2,
        blue: 0xf7,
        alpha: u8::MAX,
    })
}

fn default_inactive_border() -> FixtureColor {
    FixtureColor(HexColor {
        red: 0x3b,
        green: 0x42,
        blue: 0x61,
        alpha: u8::MAX,
    })
}

impl TryFrom<String> for HexColor {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        let Some(hex) = value.strip_prefix('#') else {
            return Err(format!("expected #RRGGBB or #RRGGBBAA, got {value:?}"));
        };
        let bytes = match hex.len() {
            6 => [
                parse_hex_byte(&hex[0..2])?,
                parse_hex_byte(&hex[2..4])?,
                parse_hex_byte(&hex[4..6])?,
                u8::MAX,
            ],
            8 => [
                parse_hex_byte(&hex[0..2])?,
                parse_hex_byte(&hex[2..4])?,
                parse_hex_byte(&hex[4..6])?,
                parse_hex_byte(&hex[6..8])?,
            ],
            _ => return Err(format!("expected 6 or 8 hex digits, got {value:?}")),
        };

        Ok(Self {
            red: bytes[0],
            green: bytes[1],
            blue: bytes[2],
            alpha: bytes[3],
        })
    }
}

impl From<FixtureColor> for Rgba {
    fn from(value: FixtureColor) -> Self {
        Self {
            red: value.0.red,
            green: value.0.green,
            blue: value.0.blue,
            alpha: value.0.alpha,
        }
    }
}

fn parse_hex_byte(raw: &str) -> Result<u8, String> {
    u8::from_str_radix(raw, 16).map_err(|error| format!("invalid hex byte {raw:?}: {error}"))
}

fn run_render_fixtures(patterns: &[String]) -> Result<(), String> {
    let owned_patterns = if patterns.is_empty() {
        default_render_fixture_patterns()
    } else {
        patterns.to_vec()
    };
    let paths = resolve_paths(&owned_patterns)?;

    for path in paths {
        run_render_fixture(&path)?;
        println!("rendered {}", path.display());
    }

    Ok(())
}

fn run_render_fixture(path: &Path) -> Result<(), String> {
    let fixture: RenderFixture =
        serde_json::from_slice(&std::fs::read(path).map_err(|error| {
            format!("failed to read render fixture {}: {error}", path.display())
        })?)
        .map_err(|error| format!("failed to parse render fixture {}: {error}", path.display()))?;
    let damage = DamageSet {
        dirty_rows: fixture.damage.dirty_rows.clone(),
        full_frame: fixture.damage.full_frame,
    };
    let backend = match fixture.backend {
        FixtureBackend::Gpu => RenderBackend::Gpu,
        FixtureBackend::Software => RenderBackend::Software,
    };
    let plan = RenderPlan::from_input(noctrail_render::RenderInput {
        pane_rect: RenderRect::new(
            fixture.surface.x,
            fixture.surface.y,
            fixture.surface.width,
            fixture.surface.height,
        ),
        viewport: RenderRect::new(
            fixture.surface.x,
            fixture.surface.y,
            fixture.surface.width,
            fixture.surface.height,
        ),
        backend,
        snapshot: &fixture.snapshot,
        damage: &damage,
        active: fixture.active,
        border: PaneBorderStyle {
            width: fixture.border.width,
            active: fixture.border.active.into(),
            inactive: fixture.border.inactive.into(),
        },
        corner_radius: 0,
    });
    let prepared = prepare_render_frame(
        &plan,
        &GlyphRasterConfig {
            scale: fixture.glyph_raster.scale,
            cell_width: fixture.glyph_raster.cell_width,
            line_height: fixture.glyph_raster.line_height,
            ..GlyphRasterConfig::default()
        },
    )
    .map_err(|error| {
        format!(
            "failed to prepare render fixture {}: {error}",
            path.display()
        )
    })?;

    assert_render_fixture(path, &fixture.expect, &prepared)
}

fn assert_render_fixture(
    path: &Path,
    expect: &RenderFixtureExpect,
    prepared: &noctrail_render::PreparedRenderFrame,
) -> Result<(), String> {
    if !expect.prepared_rows.is_empty() && prepared.glyphs.prepared_rows != expect.prepared_rows {
        return Err(format!(
            "{} prepared rows mismatch: expected {:?}, got {:?}",
            path.display(),
            expect.prepared_rows,
            prepared.glyphs.prepared_rows
        ));
    }

    if let Some(glyph_rows) = &expect.glyph_rows {
        let actual_rows = prepared
            .glyphs
            .glyphs
            .iter()
            .map(|glyph| glyph.row)
            .collect::<Vec<_>>();
        if &actual_rows != glyph_rows {
            return Err(format!(
                "{} glyph rows mismatch: expected {:?}, got {:?}",
                path.display(),
                glyph_rows,
                actual_rows
            ));
        }
    }

    if let Some(raster_jobs) = expect.raster_jobs
        && prepared.glyphs.raster_jobs() != raster_jobs
    {
        return Err(format!(
            "{} raster jobs mismatch: expected {}, got {}",
            path.display(),
            raster_jobs,
            prepared.glyphs.raster_jobs()
        ));
    }

    if let Some(glyphs_prepared) = expect.glyphs_prepared
        && prepared.stats.glyphs_prepared != glyphs_prepared
    {
        return Err(format!(
            "{} glyph count mismatch: expected {}, got {}",
            path.display(),
            glyphs_prepared,
            prepared.stats.glyphs_prepared
        ));
    }

    if let Some(paint_rects) = expect.paint_rects
        && prepared.stats.paint_rects != paint_rects
    {
        return Err(format!(
            "{} paint rect count mismatch: expected {}, got {}",
            path.display(),
            paint_rects,
            prepared.stats.paint_rects
        ));
    }

    if let Some(full_frame) = expect.full_frame
        && prepared.stats.full_frame != full_frame
    {
        return Err(format!(
            "{} full_frame mismatch: expected {}, got {}",
            path.display(),
            full_frame,
            prepared.stats.full_frame
        ));
    }

    assert_expected_rects(
        path,
        "background",
        PaintLayer::Background,
        expect.background_rects.as_deref(),
        prepared,
    )?;
    assert_expected_rects(
        path,
        "selection",
        PaintLayer::Selection,
        expect.selection_rects.as_deref(),
        prepared,
    )?;
    assert_expected_rects(
        path,
        "underline",
        PaintLayer::Underline,
        expect.underline_rects.as_deref(),
        prepared,
    )?;
    assert_expected_rects(
        path,
        "cursor",
        PaintLayer::Cursor,
        expect.cursor_rects.as_deref(),
        prepared,
    )?;
    assert_expected_border_segments(path, expect.border_segments.as_deref(), prepared)?;

    Ok(())
}

fn assert_expected_rects(
    path: &Path,
    label: &str,
    layer: PaintLayer,
    expected: Option<&[ExpectedRect]>,
    prepared: &noctrail_render::PreparedRenderFrame,
) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = prepared
        .paint
        .rects
        .iter()
        .filter(|rect| rect.layer == layer)
        .map(|rect| ExpectedRect {
            row: rect.row,
            col: rect.col,
            span: rect.span,
        })
        .collect::<Vec<_>>();

    if actual != expected {
        return Err(format!(
            "{} {} rects mismatch: expected {:?}, got {:?}",
            path.display(),
            label,
            expected,
            actual
        ));
    }

    Ok(())
}

fn assert_expected_border_segments(
    path: &Path,
    expected: Option<&[ExpectedBorderSegment]>,
    prepared: &noctrail_render::PreparedRenderFrame,
) -> Result<(), String> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let actual = prepared
        .border
        .segments
        .iter()
        .map(|segment| ExpectedBorderSegment {
            x: segment.x,
            y: segment.y,
            width: segment.width,
            height: segment.height,
            color: FixtureColor(HexColor {
                red: segment.color.red,
                green: segment.color.green,
                blue: segment.color.blue,
                alpha: segment.color.alpha,
            }),
        })
        .collect::<Vec<_>>();

    if actual != expected {
        return Err(format!(
            "{} border segments mismatch: expected {:?}, got {:?}",
            path.display(),
            expected,
            actual
        ));
    }

    Ok(())
}

fn resolve_paths(patterns: &[String]) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    for pattern in patterns {
        if contains_glob_meta(pattern) {
            let entries = glob::glob(pattern)
                .map_err(|error| format!("failed to parse glob pattern {pattern:?}: {error}"))?;
            for entry in entries {
                let path = entry.map_err(|error| format!("failed to read glob entry: {error}"))?;
                paths.push(canonicalize_fixture_path(path)?);
            }
        } else {
            paths.push(canonicalize_fixture_path(PathBuf::from(pattern))?);
        }
    }

    if paths.is_empty() {
        return Err("no fixtures matched the provided patterns".to_string());
    }

    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn default_render_fixture_patterns() -> Vec<String> {
    let workspace_pattern =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/render/*.ntshot");
    vec![
        "tests/fixtures/render/*.ntshot".to_string(),
        workspace_pattern.to_string_lossy().into_owned(),
    ]
}

fn canonicalize_fixture_path(path: PathBuf) -> Result<PathBuf, String> {
    path.canonicalize().map_err(|error| {
        format!(
            "failed to canonicalize fixture path {}: {error}",
            path.display()
        )
    })
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
    use std::sync::{Mutex, OnceLock};

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
    fn render_fixtures_succeed() {
        run_render_fixtures(&[]).expect("render fixtures should pass");
    }

    #[test]
    fn pty_smoke_probe_contains_sentinel() {
        let probe = pty_smoke_probe("NOCTRAIL_PTY_SMOKE").expect("pty smoke probe should build");
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
    fn shell_target_filter_accepts_aliases() {
        let selected = select_shell_targets(shell_target_specs(), &[String::from("wsl")])
            .expect("filters should resolve");

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "WSL");
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

    #[test]
    fn unicode_target_filter_accepts_named_targets() {
        let specs = unicode_target_specs();
        let selected =
            select_unicode_targets(&specs, &[String::from("cjk"), String::from("fullwidth")])
                .expect("filters should resolve");

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].name, "cjk");
        assert_eq!(selected[1].name, "fullwidth");
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
    fn shell_matrix_accepts_override_probe() {
        let _guard = env_test_lock()
            .lock()
            .expect("env test lock should be available");
        let script_path = temp_fixture_path("shell-override-test", "sh");
        fs::write(
            &script_path,
            "#!/bin/sh\nwhile IFS= read -r line; do\n  case \"$line\" in\n    pwd) pwd ;;\n    exit) exit 0 ;;\n    *) printf 'ECHO:%s\\n' \"$line\" ;;\n  esac\ndone\n",
        )
        .expect("script should write");
        make_executable(&script_path);

        unsafe {
            env::set_var("NOCTRAIL_SHELL_PWSH", &script_path);
        }

        let result = run_shell_matrix(&[String::from("pwsh")]);

        unsafe {
            env::remove_var("NOCTRAIL_SHELL_PWSH");
        }
        let _ = fs::remove_file(&script_path);

        result.expect("override probe should pass");
    }

    #[cfg(not(windows))]
    #[test]
    fn prompt_matrix_builtin_emulations_pass() {
        run_prompt_matrix(&[]).expect("builtin prompt probes should pass");
    }

    #[test]
    fn unicode_matrix_builtin_probes_pass() {
        run_unicode_matrix(&[]).expect("builtin unicode probes should pass");
    }

    fn env_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[cfg(not(windows))]
    fn make_executable(path: &Path) {
        make_executable_path(path).expect("script should be executable");
    }
}
