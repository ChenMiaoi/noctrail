use std::{env, process};

use noctrail_app::{DesktopApp, gui};
use noctrail_layout::LayoutRect;
use noctrail_pty::PtySize;

const HELP: &str = "\
Noctrail app smoke harness

Usage:
  noctrail-app [command]

Commands:
  gui       Open the GUI shell window (default)
  smoke     Spawn a shell, build the single-pane frame, and shut it down
  help      Print this help text

Options:
  -h, --help     Print this help text
";

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None | Some("gui" | "run") => {
            if let Err(error) = run_gui() {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        Some("help" | "-h" | "--help") => print!("{HELP}"),
        Some("smoke") => {
            if let Err(error) = run_smoke() {
                eprintln!("{error}");
                process::exit(1);
            }
        }
        Some(command) => {
            eprintln!("unknown command: {command}");
            eprintln!("run `noctrail-app help` for usage");
            process::exit(2);
        }
    }
}

fn run_gui() -> Result<(), Box<dyn std::error::Error>> {
    gui::run()
}

fn run_smoke() -> Result<(), Box<dyn std::error::Error>> {
    let mut app = DesktopApp::spawn_shell(LayoutRect::new(0, 0, 120, 80), PtySize::new(80, 24))?;

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
