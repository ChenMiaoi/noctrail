use std::{env, path::PathBuf, process};

use noctrail_app::{
    DesktopApp, PaneChromeConfig,
    gui::{self, GuiLaunchOptions},
};
use noctrail_config::{Config, ConfigError, RendererBackend as ConfigRendererBackend, ThemeConfig};
use noctrail_layout::LayoutRect;
use noctrail_pty::PtySize;
use noctrail_render::{PaneBorderStyle, RenderBackend, Rgba};

const HELP: &str = "\
Noctrail app smoke harness

Usage:
  noctrail-app [command] [options]

Commands:
  gui       Open the GUI shell window (default)
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
    Gui,
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
    requested_opacity: f32,
    effective_opacity: f32,
    transparency_fallback_reason: Option<&'static str>,
    blur_mode: &'static str,
    blur_fallback_reason: Option<&'static str>,
}

fn main() {
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
        StartupCommand::Gui => {
            if let Err(error) = run_gui(&options) {
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
            "gui" | "run" if !command_set => {
                command = StartupCommand::Gui;
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

fn run_smoke(options: &StartupOptions) -> Result<(), Box<dyn std::error::Error>> {
    let launch_options = resolve_launch_options(options)?;
    let mut app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;
    app.set_backend(launch_options.renderer_backend);
    app.set_pane_chrome(pane_chrome_from_theme(&launch_options.theme))?;
    let effects = visual_effects_mode(&launch_options);

    let frame = app.frame();
    println!(
        "pane={:?} pid={:?} backend={:?} pane={}x{} content={}x{} terminal={}x{} rows={} status_shell={} status_cwd={} status_git={} status_exit={} font={} size={} opacity={} requested_opacity={} transparency_fallback={} blur={} blur_fallback={} animation={} animation_duration_ms={}",
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
        effects.effective_opacity,
        effects.requested_opacity,
        effects.transparency_fallback_reason.unwrap_or("none"),
        effects.blur_mode,
        effects.blur_fallback_reason.unwrap_or("none"),
        if launch_options.theme.animation.enabled {
            "on"
        } else {
            "off"
        },
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
    if requested_opacity >= 1.0 {
        return VisualEffectsMode {
            requested_opacity,
            effective_opacity: 1.0,
            transparency_fallback_reason: None,
            blur_mode: "off",
            blur_fallback_reason: None,
        };
    }

    if launch_options.safe_mode {
        return VisualEffectsMode {
            requested_opacity,
            effective_opacity: 1.0,
            transparency_fallback_reason: Some("safe-mode"),
            blur_mode: if launch_options.theme.blur.enabled {
                "tinted-solid"
            } else {
                "off"
            },
            blur_fallback_reason: if launch_options.theme.blur.enabled {
                Some("safe-mode")
            } else {
                None
            },
        };
    }

    if launch_options.renderer_backend != RenderBackend::Gpu {
        return VisualEffectsMode {
            requested_opacity,
            effective_opacity: 1.0,
            transparency_fallback_reason: Some("software-backend"),
            blur_mode: if launch_options.theme.blur.enabled {
                "tinted-solid"
            } else {
                "off"
            },
            blur_fallback_reason: if launch_options.theme.blur.enabled {
                Some("software-backend")
            } else {
                None
            },
        };
    }

    if launch_options.theme.blur.enabled {
        return VisualEffectsMode {
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
            "[font]\nfamily = \"Iosevka\"\nsize = 15.5\n\n[theme]\nopacity = 0.85\n\n[theme.pane]\ngap = 10\npadding = 4\nradius = 12\n\n[theme.blur]\nenabled = true\nfallback-tint-opacity = 0.94\n\n[theme.animation]\nenabled = false\nduration-ms = 180\n\n[theme.cursor]\nblink-interval-ms = 420\n",
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

        assert_eq!(mode.effective_opacity, 1.0);
        assert_eq!(mode.transparency_fallback_reason, Some("safe-mode"));
        assert_eq!(mode.blur_mode, "tinted-solid");
        assert_eq!(mode.blur_fallback_reason, Some("safe-mode"));
    }

    fn temp_config_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("noctrail-app-{label}-{unique}.toml"))
    }
}
