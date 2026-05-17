use std::{env, process};

use noctrail_app::DesktopApp;
use noctrail_layout::LayoutRect;
use noctrail_pty::PtySize;

const HELP: &str = "\
Noctrail app smoke harness

Usage:
  noctrail-app [command]

Commands:
  smoke     Spawn a shell, build the single-pane frame, and shut it down
  help      Print this help text

Options:
  -h, --help     Print this help text
";

fn main() {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None | Some("help" | "-h" | "--help") => print!("{HELP}"),
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

    let _ = app.close_runtime()?;
    Ok(())
}
