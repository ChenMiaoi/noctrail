use std::{
    env, fs, panic,
    path::PathBuf,
    process,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use noctrail_app::{
    DesktopApp, PaneChromeConfig,
    gui::{self, GuiLaunchOptions},
    input,
};
use noctrail_config::{Config, ConfigError, RendererBackend as ConfigRendererBackend, ThemeConfig};
use noctrail_layout::{FocusDirection, LayoutRect, SplitAxis};
use noctrail_pty::{PtyCommand, PtySize};
use noctrail_render::{PaneBorderStyle, RenderBackend, Rgba};
use winit::keyboard::{Key, ModifiersState};

const HELP: &str = "\
Noctrail app smoke harness

Usage:
  noctrail-app [command] [options]

Commands:
  crash-smoke Run the panic-hook recovery probe
  gui       Open the GUI shell window (default)
  perf-smoke Run the performance smoke probe
  soak-smoke Run the split/close/resize soak probe
  smoke     Spawn a shell, build the single-pane frame, and shut it down
  help      Print this help text

Options:
  --config <path>    Load a TOML config file
  --safe-mode        Ignore config failures and force software backend
  -h, --help         Print this help text
";

#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupOptions {
    command: StartupCommand,
    config_path: Option<PathBuf>,
    safe_mode: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartupCommand {
    CrashSmoke,
    Gui,
    PerfSmoke,
    SoakSmoke,
    Smoke,
    Help,
}

#[derive(Debug, thiserror::Error)]
enum StartupError {
    #[error("missing value for --config")]
    MissingConfigPath,
    #[error("unknown option: {0}")]
    UnknownOption(String),
    #[error("unknown command: {0}")]
    UnknownCommand(String),
    #[error("config load failed outside safe mode: {0}")]
    Config(#[from] ConfigError),
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct VisualEffectsMode {
    low_power_enabled: bool,
    requested_opacity: f32,
    effective_opacity: f32,
    transparency_fallback_reason: Option<&'static str>,
    blur_mode: &'static str,
    blur_fallback_reason: Option<&'static str>,
}

fn main() {
    install_process_panic_hook(crash_diagnostic_path());
    let args = env::args().skip(1).collect::<Vec<_>>();
    let options = match parse_startup_options(&args) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            if matches!(
                error,
                StartupError::UnknownCommand(_) | StartupError::UnknownOption(_)
            ) {
                eprintln!("run `noctrail-app help` for usage");
            }
            process::exit(2);
        }
    };

    match options.command {
        StartupCommand::Help => print!("{HELP}"),
        StartupCommand::CrashSmoke => {
            if let Err(error) = run_crash_smoke() {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        StartupCommand::Gui => {
            if let Err(error) = run_gui(&options) {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        StartupCommand::PerfSmoke => {
            if let Err(error) = run_perf_smoke(&options) {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        StartupCommand::SoakSmoke => {
            if let Err(error) = run_soak_smoke() {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        StartupCommand::Smoke => {
            if let Err(error) = run_smoke(&options) {
                eprintln!("{error}");
                process::exit(1);
            }
        }
    }
}

fn parse_startup_options(args: &[String]) -> Result<StartupOptions, StartupError> {
    let mut command = StartupCommand::Gui;
    let mut command_set = false;
    let mut config_path = None;
    let mut safe_mode = false;
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "crash-smoke" if !command_set => {
                command = StartupCommand::CrashSmoke;
                command_set = true;
            }
            "gui" | "run" if !command_set => {
                command = StartupCommand::Gui;
                command_set = true;
            }
            "perf-smoke" if !command_set => {
                command = StartupCommand::PerfSmoke;
                command_set = true;
            }
            "soak-smoke" if !command_set => {
                command = StartupCommand::SoakSmoke;
                command_set = true;
            }
            "smoke" if !command_set => {
                command = StartupCommand::Smoke;
                command_set = true;
            }
            "help" | "-h" | "--help" if !command_set => {
                command = StartupCommand::Help;
                command_set = true;
            }
            "--config" => {
                index += 1;
                let Some(path) = args.get(index) else {
                    return Err(StartupError::MissingConfigPath);
                };
                config_path = Some(PathBuf::from(path));
            }
            "--safe-mode" => safe_mode = true,
            option if option.starts_with('-') => {
                return Err(StartupError::UnknownOption(option.to_string()));
            }
            other if !command_set => return Err(StartupError::UnknownCommand(other.to_string())),
            other => return Err(StartupError::UnknownOption(other.to_string())),
        }
        index += 1;
    }

    Ok(StartupOptions {
        command,
        config_path,
        safe_mode,
    })
}

fn resolve_launch_options(options: &StartupOptions) -> Result<GuiLaunchOptions, StartupError> {
    let config = load_config(options)?;
    let renderer_backend = if options.safe_mode {
        RenderBackend::Software
    } else {
        match config.renderer.backend {
            ConfigRendererBackend::Gpu => RenderBackend::Gpu,
            ConfigRendererBackend::Software => RenderBackend::Software,
        }
    };

    Ok(GuiLaunchOptions {
        safe_mode: options.safe_mode,
        renderer_backend,
        config_path: options.config_path.clone(),
        theme: config.theme,
        font: config.font,
    })
}

fn load_config(options: &StartupOptions) -> Result<Config, StartupError> {
    let Some(path) = options.config_path.as_ref() else {
        return Ok(Config::default());
    };

    match Config::load_from_path(path) {
        Ok(config) => Ok(config),
        Err(error) if options.safe_mode => {
            eprintln!("ignoring config error in safe mode: {error}");
            Ok(Config::default())
        }
        Err(error) => Err(StartupError::Config(error)),
    }
}

fn run_gui(options: &StartupOptions) -> Result<(), Box<dyn std::error::Error>> {
    gui::run_with_options(resolve_launch_options(options)?)?;
    Ok(())
}

fn run_crash_smoke() -> Result<(), Box<dyn std::error::Error>> {
    let diagnostic_path = crash_diagnostic_path();
    let _ = fs::remove_file(&diagnostic_path);
    let _guard = panic_hook_lock()
        .lock()
        .expect("panic hook test lock should be available");
    let previous = panic::take_hook();
    panic::set_hook(build_panic_hook(diagnostic_path.clone()));

    let panic_result = panic::catch_unwind(|| {
        panic!("crash smoke token=sk-live-secret password=hunter2");
    });

    let _ = panic::take_hook();
    panic::set_hook(previous);
    if panic_result.is_ok() {
        return Err("crash smoke did not panic".into());
    }

    let diagnostic = fs::read_to_string(&diagnostic_path)?;
    if diagnostic.contains("sk-live-secret") || diagnostic.contains("hunter2") {
        return Err("crash diagnostic leaked an unredacted secret".into());
    }

    println!("crash_diagnostic={}", diagnostic_path.display());
    println!("crash smoke ok");
    Ok(())
}

fn run_smoke(options: &StartupOptions) -> Result<(), Box<dyn std::error::Error>> {
    let launch_options = resolve_launch_options(options)?;
    let mut app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;
    app.set_backend(launch_options.renderer_backend);
    app.set_pane_chrome(pane_chrome_from_theme(&launch_options.theme))?;
    let effects = visual_effects_mode(&launch_options);

    let frame = app.frame();
    println!(
        "pane={:?} pid={:?} backend={:?} pane={}x{} content={}x{} terminal={}x{} rows={} status_shell={} status_cwd={} status_git={} status_exit={} font={} size={} low_power={} opacity={} requested_opacity={} transparency_fallback={} blur={} blur_fallback={} animation={} animation_duration_ms={}",
        frame.pane_id,
        frame.process_id,
        frame.render_plan.backend,
        frame.pane_surface.width,
        frame.pane_surface.height,
        frame.surface.width,
        frame.surface.height,
        frame.terminal_size.cols,
        frame.terminal_size.rows,
        frame.render_plan.rows.len(),
        frame.status_line.shell.as_deref().unwrap_or("none"),
        display_status_path(frame.status_line.cwd.as_deref()),
        frame.status_line.git_branch.as_deref().unwrap_or("none"),
        frame.status_line.exit_status.as_deref().unwrap_or("none"),
        launch_options.font.family,
        launch_options.font.size,
        on_off(effects.low_power_enabled),
        effects.effective_opacity,
        effects.requested_opacity,
        effects.transparency_fallback_reason.unwrap_or("none"),
        effects.blur_mode,
        effects.blur_fallback_reason.unwrap_or("none"),
        on_off(animations_enabled(&launch_options.theme)),
        launch_options.theme.animation.duration_ms,
    );

    app.write_input(shell_marker_command("NOCTRAIL_APP_SMOKE_WRITE").as_bytes())?;
    app.paste_text(&shell_marker_command("NOCTRAIL_APP_SMOKE_PASTE"))?;
    app.write_input(shell_exit_command().as_bytes())?;

    let output = read_all_runtime_output(&mut app)?;
    let text = String::from_utf8_lossy(&output);
    if !text.contains("NOCTRAIL_APP_SMOKE_WRITE") {
        return Err(format!("smoke output missing write marker: {text:?}").into());
    }
    if !text.contains("NOCTRAIL_APP_SMOKE_PASTE") {
        return Err(format!("smoke output missing paste marker: {text:?}").into());
    }
    println!("input smoke ok");

    let _ = app.close_runtime()?;
    let final_frame = app.frame();
    println!(
        "final_status_shell={} final_status_cwd={} final_status_git={} final_status_exit={}",
        final_frame.status_line.shell.as_deref().unwrap_or("none"),
        display_status_path(final_frame.status_line.cwd.as_deref()),
        final_frame
            .status_line
            .git_branch
            .as_deref()
            .unwrap_or("none"),
        final_frame
            .status_line
            .exit_status
            .as_deref()
            .unwrap_or("none"),
    );
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PerfSmokeReport {
    high_output_p95_ms: f64,
    scrollback_p95_ms: f64,
    multi_pane_p95_ms: f64,
    input_encode_p95_ms: f64,
    idle_premature_redraw: bool,
    idle_next_wakeup_ms: u128,
}

fn run_perf_smoke(options: &StartupOptions) -> Result<(), Box<dyn std::error::Error>> {
    let launch_options = resolve_launch_options(options)?;
    let high_output_p95_ms = measure_high_output_p95_ms();
    let scrollback_p95_ms = measure_scrollback_p95_ms();
    let multi_pane_p95_ms = measure_multi_pane_p95_ms()?;
    let input_encode_p95_ms = measure_input_encode_p95_ms();
    let idle = gui::idle_schedule_probe(&launch_options.theme);
    let report = PerfSmokeReport {
        high_output_p95_ms,
        scrollback_p95_ms,
        multi_pane_p95_ms,
        input_encode_p95_ms,
        idle_premature_redraw: idle.premature_redraw,
        idle_next_wakeup_ms: idle.next_wakeup.as_millis(),
    };

    println!(
        "high_output_p95_ms={:.3} scrollback_p95_ms={:.3} multi_pane_p95_ms={:.3} input_encode_p95_ms={:.3} idle_next_wakeup_ms={} idle_premature_redraw={}",
        report.high_output_p95_ms,
        report.scrollback_p95_ms,
        report.multi_pane_p95_ms,
        report.input_encode_p95_ms,
        report.idle_next_wakeup_ms,
        report.idle_premature_redraw,
    );

    const P95_BUDGET_MS: f64 = 30.0;
    if report.high_output_p95_ms > P95_BUDGET_MS {
        return Err(format!(
            "high output p95 exceeded budget: {:.3}ms",
            report.high_output_p95_ms
        )
        .into());
    }
    if report.scrollback_p95_ms > P95_BUDGET_MS {
        return Err(format!(
            "scrollback p95 exceeded budget: {:.3}ms",
            report.scrollback_p95_ms
        )
        .into());
    }
    if report.multi_pane_p95_ms > P95_BUDGET_MS {
        return Err(format!(
            "multi-pane p95 exceeded budget: {:.3}ms",
            report.multi_pane_p95_ms
        )
        .into());
    }
    if report.input_encode_p95_ms > P95_BUDGET_MS {
        return Err(format!(
            "input encoding p95 exceeded budget: {:.3}ms",
            report.input_encode_p95_ms
        )
        .into());
    }
    if report.idle_premature_redraw {
        return Err("idle scheduler requested redraw before the blink deadline".into());
    }

    println!("perf smoke ok");
    Ok(())
}

fn run_soak_smoke() -> Result<(), Box<dyn std::error::Error>> {
    const MAX_PANES: usize = 8;
    const CYCLES: usize = 256;
    const RSS_GROWTH_BUDGET_PERCENT: f64 = 20.0;

    let rss_start = current_rss_bytes()?;
    let mut app = DesktopApp::spawn(
        LayoutRect::new(0, 0, 120, 80),
        perf_pane_command(),
        PtySize::new(120, 40),
    )?;
    run_soak_cycles(&mut app, CYCLES / 2, MAX_PANES)?;
    while app.pane_count() > 1 {
        let _ = app.close_active_pane()?;
    }
    let _ = app.frame();
    let rss_baseline = current_rss_bytes()?;

    run_soak_cycles(&mut app, CYCLES, MAX_PANES)?;
    while app.pane_count() > 1 {
        let _ = app.close_active_pane()?;
    }
    let _ = app.frame();

    let rss_end = current_rss_bytes()?;
    let growth_percent = rss_growth_percent(rss_baseline, rss_end);
    println!(
        "soak_cycles={} pane_count={} rss_start_kb={} rss_baseline_kb={} rss_end_kb={} rss_growth_pct={:.2}",
        CYCLES,
        app.pane_count(),
        rss_start / 1024,
        rss_baseline / 1024,
        rss_end / 1024,
        growth_percent,
    );

    if growth_percent > RSS_GROWTH_BUDGET_PERCENT {
        return Err(format!(
            "rss growth exceeded budget: {:.2}% > {:.2}%",
            growth_percent, RSS_GROWTH_BUDGET_PERCENT
        )
        .into());
    }

    println!("soak smoke ok");
    Ok(())
}

fn run_soak_cycles(
    app: &mut DesktopApp,
    cycles: usize,
    max_panes: usize,
) -> Result<(), Box<dyn std::error::Error>> {
    let directions = [
        FocusDirection::Left,
        FocusDirection::Up,
        FocusDirection::Right,
        FocusDirection::Down,
    ];

    for index in 0..cycles {
        if app.pane_count() < max_panes {
            let axis = if index % 2 == 0 {
                SplitAxis::Horizontal
            } else {
                SplitAxis::Vertical
            };
            app.split_active_pane_with_axis(perf_pane_command(), axis)?;
        } else if index % 3 == 0 {
            let _ = app.close_active_pane()?;
        }

        if app.pane_count() > 1 {
            let _ = focus_any_direction(app, &directions);
            if index % 4 == 0 {
                let _ = app.resize_active_split(FocusDirection::Right, 1);
            }
        }

        let line = format!("soak-{index:04}\r\n");
        app.advance_output(line.as_bytes());
        let _ = app.frame();
    }
    Ok(())
}

fn install_process_panic_hook(path: PathBuf) {
    let _guard = panic_hook_lock()
        .lock()
        .expect("panic hook lock should be available");
    let _ = panic::take_hook();
    panic::set_hook(build_panic_hook(path));
}

fn build_panic_hook(
    path: PathBuf,
) -> Box<dyn Fn(&panic::PanicHookInfo<'_>) + Sync + Send + 'static> {
    Box::new(move |info| {
        let _ = write_crash_diagnostic(&path, info);
        eprintln!("noctrail panic diagnostic written to {}", path.display());
    })
}

fn crash_diagnostic_path() -> PathBuf {
    env::temp_dir().join("noctrail-last-diagnostic.log")
}

fn write_crash_diagnostic(
    path: &std::path::Path,
    info: &panic::PanicHookInfo<'_>,
) -> Result<(), Box<dyn std::error::Error>> {
    let message = redact_diagnostic_text(&panic_payload_text(info));
    let location = info
        .location()
        .map(|location| format!("{}:{}", location.file(), location.line()))
        .unwrap_or_else(|| "unknown".to_string());
    let command_line = redact_diagnostic_text(&env::args().collect::<Vec<_>>().join(" "));
    let diagnostic = format!(
        "pid={}\nlocation={}\nmessage={}\ncommand={}\n",
        process::id(),
        location,
        message,
        command_line,
    );
    fs::write(path, diagnostic)?;
    Ok(())
}

fn panic_payload_text(info: &panic::PanicHookInfo<'_>) -> String {
    if let Some(message) = info.payload().downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = info.payload().downcast_ref::<String>() {
        return message.clone();
    }

    "non-string panic payload".to_string()
}

fn redact_diagnostic_text(text: &str) -> String {
    let mut redacted = text.to_string();
    for marker in [
        "token=",
        "password=",
        "secret=",
        "api_key=",
        "authorization=",
        "Bearer ",
        "sk-",
        "ghp_",
    ] {
        redacted = redact_after_marker(&redacted, marker);
    }
    redacted
}

fn redact_after_marker(text: &str, marker: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut cursor = 0;

    while let Some(found) = text[cursor..].find(marker) {
        let start = cursor + found;
        let value_start = start + marker.len();
        result.push_str(&text[cursor..value_start]);
        let value_end = text[value_start..]
            .find(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | '"' | '\''))
            .map(|offset| value_start + offset)
            .unwrap_or(text.len());
        result.push_str("[REDACTED]");
        cursor = value_end;
    }

    result.push_str(&text[cursor..]);
    result
}

fn read_all_runtime_output(app: &mut DesktopApp) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let runtime = app
        .pane_mut()
        .runtime_mut()
        .ok_or("active pane is missing a runtime")?;
    let mut output = Vec::new();
    let mut chunk = [0_u8; 1024];

    loop {
        let count = runtime.read_output(&mut chunk)?;
        if count == 0 {
            break;
        }
        output.extend_from_slice(&chunk[..count]);
    }

    Ok(output)
}

fn pane_chrome_from_theme(theme: &ThemeConfig) -> PaneChromeConfig {
    PaneChromeConfig {
        border: PaneBorderStyle {
            width: usize::from(theme.border.width),
            active: rgba_from_config(theme.border.active),
            inactive: rgba_from_config(theme.border.inactive),
        },
        gap: theme.pane.gap,
        padding: theme.pane.padding,
        radius: theme.pane.radius,
    }
}

fn rgba_from_config(color: noctrail_config::RgbaColor) -> Rgba {
    Rgba {
        red: color.red,
        green: color.green,
        blue: color.blue,
        alpha: color.alpha,
    }
}

fn visual_effects_mode(launch_options: &GuiLaunchOptions) -> VisualEffectsMode {
    let requested_opacity = launch_options.theme.opacity;
    let low_power_enabled = launch_options.theme.low_power.enabled;
    if launch_options.safe_mode {
        return VisualEffectsMode {
            low_power_enabled,
            requested_opacity,
            effective_opacity: 1.0,
            transparency_fallback_reason: Some("safe-mode"),
            blur_mode: if low_power_enabled {
                "off"
            } else if launch_options.theme.blur.enabled {
                "tinted-solid"
            } else {
                "off"
            },
            blur_fallback_reason: if low_power_enabled && launch_options.theme.blur.enabled {
                Some("low-power")
            } else if launch_options.theme.blur.enabled {
                Some("safe-mode")
            } else {
                None
            },
        };
    }

    if launch_options.renderer_backend != RenderBackend::Gpu {
        return VisualEffectsMode {
            low_power_enabled,
            requested_opacity,
            effective_opacity: 1.0,
            transparency_fallback_reason: Some("software-backend"),
            blur_mode: if low_power_enabled {
                "off"
            } else if launch_options.theme.blur.enabled {
                "tinted-solid"
            } else {
                "off"
            },
            blur_fallback_reason: if low_power_enabled && launch_options.theme.blur.enabled {
                Some("low-power")
            } else if launch_options.theme.blur.enabled {
                Some("software-backend")
            } else {
                None
            },
        };
    }

    if low_power_enabled {
        return VisualEffectsMode {
            low_power_enabled,
            requested_opacity,
            effective_opacity: requested_opacity,
            transparency_fallback_reason: None,
            blur_mode: "off",
            blur_fallback_reason: launch_options.theme.blur.enabled.then_some("low-power"),
        };
    }

    if requested_opacity >= 1.0 {
        return VisualEffectsMode {
            low_power_enabled,
            requested_opacity,
            effective_opacity: 1.0,
            transparency_fallback_reason: None,
            blur_mode: "off",
            blur_fallback_reason: None,
        };
    }

    if launch_options.theme.blur.enabled {
        return VisualEffectsMode {
            low_power_enabled,
            requested_opacity,
            effective_opacity: launch_options
                .theme
                .blur
                .fallback_tint_opacity
                .max(requested_opacity),
            transparency_fallback_reason: None,
            blur_mode: "tinted-solid",
            blur_fallback_reason: Some("unsupported-platform"),
        };
    }

    VisualEffectsMode {
        low_power_enabled,
        requested_opacity,
        effective_opacity: requested_opacity,
        transparency_fallback_reason: None,
        blur_mode: "off",
        blur_fallback_reason: None,
    }
}

fn shell_marker_command(marker: &str) -> String {
    #[cfg(windows)]
    {
        format!("echo {marker}\r\n")
    }

    #[cfg(not(windows))]
    {
        format!("printf '{marker}\\n'\r")
    }
}

fn shell_exit_command() -> &'static str {
    "exit\r\n"
}

fn measure_high_output_p95_ms() -> f64 {
    let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(120, 40));
    let mut samples = Vec::with_capacity(512);

    for index in 0..512 {
        let line = format!("line-{index:04}\r\n");
        let started_at = Instant::now();
        app.advance_output(line.as_bytes());
        let _ = app.frame();
        samples.push(started_at.elapsed());
    }

    p95_millis(&mut samples)
}

fn measure_scrollback_p95_ms() -> f64 {
    let mut app = DesktopApp::new(LayoutRect::new(0, 0, 120, 80), PtySize::new(120, 40));
    for index in 0..2_000 {
        let line = format!("scrollback-{index:05}\r\n");
        app.advance_output(line.as_bytes());
    }

    let mut samples = Vec::with_capacity(128);
    for step in 0..128 {
        let delta = if step % 2 == 0 { 1 } else { -1 };
        let started_at = Instant::now();
        app.scroll_scrollback(delta);
        let _ = app.frame();
        samples.push(started_at.elapsed());
    }

    p95_millis(&mut samples)
}

fn measure_multi_pane_p95_ms() -> Result<f64, Box<dyn std::error::Error>> {
    let mut app = DesktopApp::spawn(
        LayoutRect::new(0, 0, 120, 80),
        perf_pane_command(),
        PtySize::new(120, 40),
    )?;
    app.advance_output(b"pane-1\r\n");

    for index in 0..7 {
        let axis = if index % 2 == 0 {
            SplitAxis::Horizontal
        } else {
            SplitAxis::Vertical
        };
        app.split_active_pane_with_axis(perf_pane_command(), axis)?;
        let line = format!("pane-{}\r\n", index + 2);
        app.advance_output(line.as_bytes());
    }

    let directions = [
        FocusDirection::Left,
        FocusDirection::Up,
        FocusDirection::Right,
        FocusDirection::Down,
    ];
    let mut samples = Vec::with_capacity(128);
    for index in 0..128 {
        focus_any_direction(&mut app, &directions)?;
        let started_at = Instant::now();
        app.advance_output(b"tick\r\n");
        let _ = app.frame();
        samples.push(started_at.elapsed());
        if index % 16 == 15 {
            app.resize_active_split(FocusDirection::Right, 1)?;
        }
    }

    Ok(p95_millis(&mut samples))
}

fn measure_input_encode_p95_ms() -> f64 {
    let key = Key::Character("a".into());
    let modifiers = ModifiersState::default();
    let mut samples = Vec::with_capacity(128);

    for _ in 0..128 {
        let started_at = Instant::now();
        let bytes = input::key_to_pty_bytes(&key, Some("a"), modifiers)
            .expect("plain character key should encode");
        samples.push(started_at.elapsed());
        debug_assert_eq!(bytes, b"a");
    }

    p95_millis(&mut samples)
}

fn focus_any_direction(
    app: &mut DesktopApp,
    directions: &[FocusDirection],
) -> Result<(), Box<dyn std::error::Error>> {
    for direction in directions {
        if app.focus_direction(*direction).is_ok() {
            return Ok(());
        }
    }

    Err("unable to move focus between panes during perf smoke".into())
}

fn perf_pane_command() -> PtyCommand {
    #[cfg(windows)]
    {
        let mut command = PtyCommand::new("cmd");
        command.args(["/C", "exit 0"]);
        command
    }

    #[cfg(not(windows))]
    {
        let mut command = PtyCommand::new("sh");
        command.args(["-lc", "exit 0"]);
        command
    }
}

fn current_rss_bytes() -> Result<u64, Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        let output = process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-Process -Id {}).WorkingSet64", process::id()),
            ])
            .output()?;
        if !output.status.success() {
            return Err("failed to query RSS with PowerShell".into());
        }
        let text = String::from_utf8(output.stdout)?;
        return text
            .trim()
            .parse::<u64>()
            .map_err(|error| format!("failed to parse RSS bytes: {error}").into());
    }

    #[cfg(not(windows))]
    {
        let output = process::Command::new("ps")
            .args(["-o", "rss=", "-p", &process::id().to_string()])
            .output()?;
        if !output.status.success() {
            return Err("failed to query RSS with ps".into());
        }
        let text = String::from_utf8(output.stdout)?;
        let rss_kb = parse_rss_kb(&text).ok_or("failed to parse RSS kilobytes from ps output")?;
        Ok(rss_kb.saturating_mul(1024))
    }
}

fn rss_growth_percent(start_bytes: u64, end_bytes: u64) -> f64 {
    if end_bytes <= start_bytes || start_bytes == 0 {
        return 0.0;
    }

    ((end_bytes - start_bytes) as f64 / start_bytes as f64) * 100.0
}

fn parse_rss_kb(text: &str) -> Option<u64> {
    text.split_whitespace()
        .find_map(|token| token.parse::<u64>().ok())
}

fn panic_hook_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn p95_millis(samples: &mut [Duration]) -> f64 {
    samples.sort_unstable();
    let index = ((samples.len().saturating_sub(1)) * 95) / 100;
    samples[index].as_secs_f64() * 1000.0
}

fn animations_enabled(theme: &ThemeConfig) -> bool {
    theme.animation.enabled && !theme.low_power.enabled
}

fn on_off(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

fn display_status_path(path: Option<&std::path::Path>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "none".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn parses_gui_options_without_explicit_command() {
        let options = parse_startup_options(&[
            "--config".to_string(),
            "/tmp/noctrail.toml".to_string(),
            "--safe-mode".to_string(),
        ])
        .expect("options should parse");

        assert_eq!(options.command, StartupCommand::Gui);
        assert_eq!(
            options.config_path,
            Some(PathBuf::from("/tmp/noctrail.toml"))
        );
        assert!(options.safe_mode);
    }

    #[test]
    fn parses_crash_smoke_command() {
        let options =
            parse_startup_options(&["crash-smoke".to_string()]).expect("options should parse");

        assert_eq!(options.command, StartupCommand::CrashSmoke);
        assert_eq!(options.config_path, None);
        assert!(!options.safe_mode);
    }

    #[test]
    fn parses_perf_smoke_command() {
        let options =
            parse_startup_options(&["perf-smoke".to_string()]).expect("options should parse");

        assert_eq!(options.command, StartupCommand::PerfSmoke);
        assert_eq!(options.config_path, None);
        assert!(!options.safe_mode);
    }

    #[test]
    fn parses_soak_smoke_command() {
        let options =
            parse_startup_options(&["soak-smoke".to_string()]).expect("options should parse");

        assert_eq!(options.command, StartupCommand::SoakSmoke);
        assert_eq!(options.config_path, None);
        assert!(!options.safe_mode);
    }

    #[test]
    fn broken_config_fails_outside_safe_mode() {
        let path = temp_config_path("broken");
        fs::write(&path, "[renderer\nbackend = \"gpu\"\n").expect("write config");
        let options = StartupOptions {
            command: StartupCommand::Gui,
            config_path: Some(path.clone()),
            safe_mode: false,
        };

        let error = resolve_launch_options(&options).expect_err("config should fail");
        assert!(matches!(error, StartupError::Config(_)));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn safe_mode_ignores_broken_config_and_forces_software_backend() {
        let path = temp_config_path("safe-mode");
        fs::write(&path, "[renderer\nbackend = \"gpu\"\n").expect("write config");
        let options = StartupOptions {
            command: StartupCommand::Gui,
            config_path: Some(path.clone()),
            safe_mode: true,
        };

        let launch = resolve_launch_options(&options).expect("safe mode should continue");
        assert!(launch.safe_mode);
        assert_eq!(launch.renderer_backend, RenderBackend::Software);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn software_backend_can_be_loaded_from_config() {
        let path = temp_config_path("software");
        fs::write(&path, "[renderer]\nbackend = \"software\"\n").expect("write config");
        let options = StartupOptions {
            command: StartupCommand::Smoke,
            config_path: Some(path.clone()),
            safe_mode: false,
        };

        let launch = resolve_launch_options(&options).expect("config should load");
        assert_eq!(launch.renderer_backend, RenderBackend::Software);
        assert!(!launch.safe_mode);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn theme_and_font_are_loaded_into_launch_options() {
        let path = temp_config_path("theme-font");
        fs::write(
            &path,
            "[font]\nfamily = \"Iosevka\"\nsize = 15.5\n\n[theme]\nopacity = 0.85\n\n[theme.pane]\ngap = 10\npadding = 4\nradius = 12\n\n[theme.blur]\nenabled = true\nfallback-tint-opacity = 0.94\n\n[theme.animation]\nenabled = false\nduration-ms = 180\n\n[theme.low-power]\nenabled = true\n\n[theme.cursor]\nblink-interval-ms = 420\n",
        )
        .expect("write config");
        let options = StartupOptions {
            command: StartupCommand::Gui,
            config_path: Some(path.clone()),
            safe_mode: false,
        };

        let launch = resolve_launch_options(&options).expect("config should load");
        assert_eq!(launch.font.family, "Iosevka");
        assert_eq!(launch.font.size, 15.5);
        assert_eq!(launch.theme.opacity, 0.85);
        assert_eq!(launch.theme.pane.gap, 10);
        assert_eq!(launch.theme.pane.padding, 4);
        assert_eq!(launch.theme.pane.radius, 12);
        assert!(launch.theme.blur.enabled);
        assert_eq!(launch.theme.blur.fallback_tint_opacity, 0.94);
        assert!(!launch.theme.animation.enabled);
        assert_eq!(launch.theme.animation.duration_ms, 180);
        assert!(launch.theme.low_power.enabled);
        assert_eq!(launch.theme.cursor.blink_interval_ms, 420);
        assert_eq!(launch.config_path, Some(path.clone()));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn visual_effects_mode_stays_requested_on_gpu() {
        let launch = GuiLaunchOptions {
            renderer_backend: RenderBackend::Gpu,
            theme: ThemeConfig {
                opacity: 0.8,
                ..ThemeConfig::default()
            },
            ..GuiLaunchOptions::default()
        };

        let mode = visual_effects_mode(&launch);

        assert!(!mode.low_power_enabled);
        assert_eq!(mode.requested_opacity, 0.8);
        assert_eq!(mode.effective_opacity, 0.8);
        assert_eq!(mode.transparency_fallback_reason, None);
        assert_eq!(mode.blur_mode, "off");
        assert_eq!(mode.blur_fallback_reason, None);
    }

    #[test]
    fn visual_effects_mode_uses_tinted_solid_when_blur_is_requested() {
        let launch = GuiLaunchOptions {
            renderer_backend: RenderBackend::Gpu,
            theme: ThemeConfig {
                opacity: 0.7,
                blur: noctrail_config::BlurTheme {
                    enabled: true,
                    fallback_tint_opacity: 0.9,
                },
                ..ThemeConfig::default()
            },
            ..GuiLaunchOptions::default()
        };

        let mode = visual_effects_mode(&launch);

        assert!(!mode.low_power_enabled);
        assert_eq!(mode.effective_opacity, 0.9);
        assert_eq!(mode.transparency_fallback_reason, None);
        assert_eq!(mode.blur_mode, "tinted-solid");
        assert_eq!(mode.blur_fallback_reason, Some("unsupported-platform"));
    }

    #[test]
    fn visual_effects_mode_falls_back_on_software() {
        let launch = GuiLaunchOptions {
            renderer_backend: RenderBackend::Software,
            theme: ThemeConfig {
                opacity: 0.8,
                blur: noctrail_config::BlurTheme {
                    enabled: true,
                    fallback_tint_opacity: 0.92,
                },
                ..ThemeConfig::default()
            },
            ..GuiLaunchOptions::default()
        };

        let mode = visual_effects_mode(&launch);

        assert!(!mode.low_power_enabled);
        assert_eq!(mode.effective_opacity, 1.0);
        assert_eq!(mode.transparency_fallback_reason, Some("software-backend"));
        assert_eq!(mode.blur_mode, "tinted-solid");
        assert_eq!(mode.blur_fallback_reason, Some("software-backend"));
    }

    #[test]
    fn visual_effects_mode_falls_back_in_safe_mode() {
        let launch = GuiLaunchOptions {
            safe_mode: true,
            renderer_backend: RenderBackend::Gpu,
            theme: ThemeConfig {
                opacity: 0.8,
                blur: noctrail_config::BlurTheme {
                    enabled: true,
                    fallback_tint_opacity: 0.92,
                },
                ..ThemeConfig::default()
            },
            ..GuiLaunchOptions::default()
        };

        let mode = visual_effects_mode(&launch);

        assert!(!mode.low_power_enabled);
        assert_eq!(mode.effective_opacity, 1.0);
        assert_eq!(mode.transparency_fallback_reason, Some("safe-mode"));
        assert_eq!(mode.blur_mode, "tinted-solid");
        assert_eq!(mode.blur_fallback_reason, Some("safe-mode"));
    }

    #[test]
    fn visual_effects_mode_disables_blur_in_low_power_mode() {
        let launch = GuiLaunchOptions {
            renderer_backend: RenderBackend::Gpu,
            theme: ThemeConfig {
                opacity: 0.7,
                blur: noctrail_config::BlurTheme {
                    enabled: true,
                    fallback_tint_opacity: 0.9,
                },
                low_power: noctrail_config::LowPowerTheme { enabled: true },
                ..ThemeConfig::default()
            },
            ..GuiLaunchOptions::default()
        };

        let mode = visual_effects_mode(&launch);

        assert!(mode.low_power_enabled);
        assert_eq!(mode.effective_opacity, 0.7);
        assert_eq!(mode.transparency_fallback_reason, None);
        assert_eq!(mode.blur_mode, "off");
        assert_eq!(mode.blur_fallback_reason, Some("low-power"));
        assert!(!animations_enabled(&launch.theme));
    }

    #[test]
    fn p95_millis_selects_the_upper_tail_sample() {
        let mut samples = vec![Duration::from_millis(1); 20];
        samples[18] = Duration::from_millis(40);
        samples[19] = Duration::from_millis(40);

        assert_eq!(p95_millis(&mut samples), 40.0);
    }

    #[test]
    fn parse_rss_kb_extracts_the_numeric_column() {
        assert_eq!(parse_rss_kb("  12345\n"), Some(12_345));
        assert_eq!(parse_rss_kb("rss\n"), None);
    }

    #[test]
    fn redaction_masks_secret_markers() {
        let redacted =
            redact_diagnostic_text("token=abc password=hunter2 Bearer ghp_example sk-live-secret");

        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("ghp_example"));
        assert!(!redacted.contains("sk-live-secret"));
        assert!(redacted.contains("[REDACTED]"));
    }

    #[test]
    fn crash_hook_writes_redacted_diagnostic() {
        let _guard = panic_hook_lock()
            .lock()
            .expect("panic hook lock should be available");
        let path = temp_config_path("crash-diag");
        let previous = panic::take_hook();
        panic::set_hook(build_panic_hook(path.clone()));

        let result = panic::catch_unwind(|| {
            panic!("boom token=sk-live-secret");
        });

        let _ = panic::take_hook();
        panic::set_hook(previous);
        assert!(result.is_err());

        let diagnostic = fs::read_to_string(&path).expect("diagnostic should exist");
        assert!(diagnostic.contains("message=boom token=[REDACTED]"));
        assert!(!diagnostic.contains("sk-live-secret"));

        let _ = fs::remove_file(path);
    }

    fn temp_config_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("noctrail-app-{label}-{unique}.toml"))
    }
}
