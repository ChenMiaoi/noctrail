use std::{env, path::PathBuf, process};

use noctrail_app::{
    DesktopApp,
    gui::{self, GuiLaunchOptions},
};
use noctrail_config::{Config, ConfigError, RendererBackend as ConfigRendererBackend};
use noctrail_layout::LayoutRect;
use noctrail_pty::PtySize;
use noctrail_render::RenderBackend;

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

    let frame = app.frame();
    println!(
        "pane={:?} pid={:?} backend={:?} surface={}x{} terminal={}x{} rows={}",
        frame.pane_id,
        frame.process_id,
        frame.render_plan.backend,
        frame.surface.width,
        frame.surface.height,
        frame.terminal_size.cols,
        frame.terminal_size.rows,
        frame.render_plan.rows.len()
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

    fn temp_config_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("noctrail-app-{label}-{unique}.toml"))
    }
}
