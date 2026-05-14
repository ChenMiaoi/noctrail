use std::{env, process};

const HELP: &str = "\
Noctrail development CLI

Usage:
  noctrail [command]

Commands:
  doctor      Print basic environment diagnostics
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
        Some("doctor") => print_doctor(),
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
}
