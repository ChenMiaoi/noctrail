use std::{
    env,
    path::{Path, PathBuf},
};

use noctrail_pty::{PtySession, PtySize};

use crate::commands::{common::find_executable, pty::read_all_output};

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

pub(crate) fn run_shell_matrix(filters: &[String]) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(windows))]
    use crate::commands::common::{make_executable_path, temp_fixture_path};
    #[cfg(not(windows))]
    use std::sync::{Mutex, OnceLock};

    #[test]
    fn shell_target_filter_accepts_aliases() {
        let selected = select_shell_targets(shell_target_specs(), &[String::from("wsl")])
            .expect("filters should resolve");

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "WSL");
    }

    #[cfg(not(windows))]
    #[test]
    fn shell_matrix_accepts_override_probe() {
        let _guard = env_test_lock()
            .lock()
            .expect("env test lock should be available");
        let script_path = temp_fixture_path("shell-override-test", "sh");
        std::fs::write(
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
        let _ = std::fs::remove_file(&script_path);

        result.expect("override probe should pass");
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
