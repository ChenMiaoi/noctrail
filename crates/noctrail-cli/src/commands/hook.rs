use std::{
    env, fs,
    path::{Path, PathBuf},
};

use noctrail_pty::{PtySession, PtySize};
use noctrail_term::{ShellIntegrationEvent, TerminalState};

use crate::commands::{
    common::{find_executable, make_executable_path, temp_fixture_path},
    pty::read_all_output,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HookCapability {
    Prompt,
    Command,
    Cwd,
    Exit,
    Duration,
}

impl HookCapability {
    const fn label(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Command => "command",
            Self::Cwd => "cwd",
            Self::Exit => "exit",
            Self::Duration => "duration",
        }
    }
}

#[derive(Debug, Clone)]
struct ShellHookSpec {
    name: &'static str,
    aliases: &'static [&'static str],
    program_candidates: &'static [&'static str],
    render_script: fn() -> String,
    actual_probe: Option<HookActualProbe>,
    skip_hint: &'static str,
}

type HookActualProbe = fn(&Path, &str) -> Result<HookProbe, String>;

#[derive(Debug, Clone)]
struct HookProbe {
    target: &'static str,
    source: String,
    command: noctrail_pty::PtyCommand,
    initial_size: PtySize,
    input: Vec<u8>,
    expected_command: String,
    expected_cwd: String,
    required: &'static [HookCapability],
    cleanup_paths: Vec<PathBuf>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct ObservedHookCapabilities {
    prompt: bool,
    command: bool,
    cwd: bool,
    exit: bool,
    duration: bool,
}

impl ObservedHookCapabilities {
    fn contains(self, capability: HookCapability) -> bool {
        match capability {
            HookCapability::Prompt => self.prompt,
            HookCapability::Command => self.command,
            HookCapability::Cwd => self.cwd,
            HookCapability::Exit => self.exit,
            HookCapability::Duration => self.duration,
        }
    }

    fn labels(self) -> Vec<&'static str> {
        let mut labels = Vec::new();
        for capability in [
            HookCapability::Prompt,
            HookCapability::Command,
            HookCapability::Cwd,
            HookCapability::Exit,
            HookCapability::Duration,
        ] {
            if self.contains(capability) {
                labels.push(capability.label());
            }
        }
        labels
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HookProbeStatus {
    Passed,
    Skipped(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HookProbeReport {
    target: &'static str,
    source: String,
    observed: ObservedHookCapabilities,
    required: Vec<&'static str>,
    status: HookProbeStatus,
}

pub(crate) fn print_shell_hook(target: &str) -> Result<(), String> {
    let spec = select_hook_spec(target)?;
    print!("{}", (spec.render_script)());
    Ok(())
}

pub(crate) fn run_hook_smoke(filters: &[String]) -> Result<(), String> {
    let specs = shell_hook_specs();
    let selected = select_hook_targets(specs, filters)?;
    let mut ran_any = false;
    let mut failures = Vec::new();

    for spec in selected {
        match run_hook_target(spec) {
            Ok(report) => {
                println!("{}", format_hook_report(&report));
                if matches!(report.status, HookProbeStatus::Passed) {
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
        println!("hook-smoke: all selected targets were skipped");
    } else {
        println!("hook smoke ok");
    }
    Ok(())
}

pub(crate) fn render_bash_hook() -> String {
    r#"__noctrail_emit() {
  printf '\033]1337;Noctrail;%s\007' "$1"
}

__noctrail_precmd() {
  __NOCTRAIL_IN_PRECMD=1
  local exit_code=$?
  __NOCTRAIL_LAST_COMMAND=
  __noctrail_emit "Prompt"
  __noctrail_emit "Cwd;$PWD"
  if [ -n "${__NOCTRAIL_CMD_ACTIVE:-}" ]; then
    local duration_ms=$(( (SECONDS - __NOCTRAIL_CMD_START_SEC) * 1000 ))
    __noctrail_emit "ExitCode;$exit_code"
    __noctrail_emit "DurationMs;$duration_ms"
    __noctrail_emit "CommandEnd"
    unset __NOCTRAIL_CMD_ACTIVE
  fi
  unset __NOCTRAIL_IN_PRECMD
  return $exit_code
}

__noctrail_preexec() {
  [ -n "${__NOCTRAIL_IN_PRECMD:-}" ] && return
  [ -n "${__NOCTRAIL_IN_DEBUG:-}" ] && return
  __NOCTRAIL_IN_DEBUG=1
  local command=$BASH_COMMAND
  case "$command" in
    __noctrail_* ) __NOCTRAIL_IN_DEBUG=; return ;;
  esac
  if [ "${__NOCTRAIL_LAST_COMMAND:-}" = "$command" ]; then
    __NOCTRAIL_IN_DEBUG=
    return
  fi
  __NOCTRAIL_LAST_COMMAND=$command
  __NOCTRAIL_CMD_ACTIVE=1
  __NOCTRAIL_CMD_START_SEC=$SECONDS
  __noctrail_emit "CommandStart"
  __noctrail_emit "CommandText;$command"
  __NOCTRAIL_IN_DEBUG=
}

trap '__noctrail_preexec' DEBUG
PROMPT_COMMAND="__noctrail_precmd${PROMPT_COMMAND:+;$PROMPT_COMMAND}"
"#
    .to_string()
}

pub(crate) fn render_zsh_hook() -> String {
    r#"__noctrail_emit() {
  printf '\033]1337;Noctrail;%s\007' "$1"
}

__noctrail_preexec() {
  __NOCTRAIL_CMD_ACTIVE=1
  __NOCTRAIL_CMD_START_SEC=$SECONDS
  __noctrail_emit "CommandStart"
  __noctrail_emit "CommandText;$1"
}

__noctrail_precmd() {
  local exit_code=$?
  __noctrail_emit "Prompt"
  __noctrail_emit "Cwd;$PWD"
  if [[ -n ${__NOCTRAIL_CMD_ACTIVE:-} ]]; then
    local duration_ms=$(( (SECONDS - __NOCTRAIL_CMD_START_SEC) * 1000 ))
    __noctrail_emit "ExitCode;$exit_code"
    __noctrail_emit "DurationMs;$duration_ms"
    __noctrail_emit "CommandEnd"
    unset __NOCTRAIL_CMD_ACTIVE
  fi
  return $exit_code
}

autoload -Uz add-zsh-hook
add-zsh-hook preexec __noctrail_preexec
add-zsh-hook precmd __noctrail_precmd
"#
    .to_string()
}

pub(crate) fn render_fish_hook() -> String {
    r#"function __noctrail_emit
    printf '\e]1337;Noctrail;%s\a' "$argv[1]"
end

function __noctrail_preexec --on-event fish_preexec
    set -g __noctrail_cmd_active 1
    __noctrail_emit "CommandStart"
    __noctrail_emit "CommandText;$argv[1]"
end

function __noctrail_prompt --on-event fish_prompt
    set -l exit_code $status
    __noctrail_emit "Prompt"
    __noctrail_emit "Cwd;$PWD"
    if set -q __noctrail_cmd_active
        __noctrail_emit "ExitCode;$exit_code"
        __noctrail_emit "DurationMs;$CMD_DURATION"
        __noctrail_emit "CommandEnd"
        set -e __noctrail_cmd_active
    end
end
"#
    .to_string()
}

pub(crate) fn render_pwsh_hook() -> String {
    r#"function global:__NoctrailEmit([string]$Payload) {
    [Console]::Out.Write("$([char]27)]1337;Noctrail;$Payload$([char]7)")
}

$global:__NoctrailOriginalPrompt = $function:prompt

function global:prompt {
    $exitCode = if ($null -ne $global:LASTEXITCODE) { $global:LASTEXITCODE } else { 0 }
    __NoctrailEmit "Prompt"
    __NoctrailEmit ("Cwd;" + (Get-Location).Path)
    if ($global:__NoctrailCommandActive) {
        $durationMs = [int]((Get-Date) - $global:__NoctrailCommandStart).TotalMilliseconds
        __NoctrailEmit ("ExitCode;" + $exitCode)
        __NoctrailEmit ("DurationMs;" + $durationMs)
        __NoctrailEmit "CommandEnd"
        $global:__NoctrailCommandActive = $false
    }
    if ($global:__NoctrailOriginalPrompt) {
        & $global:__NoctrailOriginalPrompt
    } else {
        "PS $((Get-Location).Path)> "
    }
}

if (Get-Module -ListAvailable -Name PSReadLine) {
    Import-Module PSReadLine
    Set-PSReadLineKeyHandler -Chord Enter -ScriptBlock {
        $line = $null
        $cursor = $null
        [Microsoft.PowerShell.PSConsoleReadLine]::GetBufferState([ref]$line, [ref]$cursor)
        $global:__NoctrailCommandActive = $true
        $global:__NoctrailCommandStart = Get-Date
        __NoctrailEmit "CommandStart"
        __NoctrailEmit ("CommandText;" + $line)
        [Microsoft.PowerShell.PSConsoleReadLine]::AcceptLine()
    }
}
"#
    .to_string()
}

pub(crate) fn render_nu_hook() -> String {
    r#"let __noctrail_emit = {|payload|
    print -n $"(ansi esc)]1337;Noctrail;($payload)(char bel)"
}

$env.config = ($env.config | upsert hooks (($env.config.hooks? | default {}) | merge {
    pre_prompt: ((($env.config.hooks.pre_prompt? | default [])) ++ [{||
        do $__noctrail_emit "Prompt"
        do $__noctrail_emit $"Cwd;($env.PWD)"
        if ($env.__noctrail_cmd_active? | default false) {
            do $__noctrail_emit $"ExitCode;($env.LAST_EXIT_CODE? | default 0)"
            do $__noctrail_emit $"DurationMs;($env.CMD_DURATION_MS? | default 0)"
            do $__noctrail_emit "CommandEnd"
            hide-env __noctrail_cmd_active
        }
    }])
    pre_execution: ((($env.config.hooks.pre_execution? | default [])) ++ [{|cmd|
        let-env __noctrail_cmd_active = true
        do $__noctrail_emit "CommandStart"
        do $__noctrail_emit $"CommandText;($cmd)"
    }])
}))
"#
    .to_string()
}

fn run_hook_target(spec: &'static ShellHookSpec) -> Result<HookProbeReport, String> {
    let Some(probe) = resolve_hook_probe(spec)? else {
        return Ok(HookProbeReport {
            target: spec.name,
            source: "unavailable".to_string(),
            observed: ObservedHookCapabilities::default(),
            required: required_hook_capabilities()
                .iter()
                .map(|capability| capability.label())
                .collect(),
            status: HookProbeStatus::Skipped(spec.skip_hint.to_string()),
        });
    };

    let result = run_hook_probe(&probe);
    cleanup_hook_probe(&probe);
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

    Ok(HookProbeReport {
        target: probe.target,
        source: probe.source,
        observed,
        required: probe.required.iter().map(|cap| cap.label()).collect(),
        status: HookProbeStatus::Passed,
    })
}

fn run_hook_probe(probe: &HookProbe) -> Result<ObservedHookCapabilities, String> {
    let mut session = PtySession::spawn(probe.command.clone(), probe.initial_size)
        .map_err(|error| format!("failed to spawn {} hook probe: {error}", probe.target))?;
    session
        .write(&probe.input)
        .map_err(|error| format!("failed to write {} hook probe input: {error}", probe.target))?;
    let output = read_all_output(&mut session)
        .map_err(|error| format!("failed to read {} hook probe output: {error}", probe.target))?;
    let status = session
        .close()
        .map_err(|error| format!("failed to close {} hook probe: {error}", probe.target))?;
    let mut terminal = TerminalState::new(
        usize::from(probe.initial_size.cols),
        usize::from(probe.initial_size.rows),
    );
    terminal.advance_bytes(&output);
    let events = terminal.drain_shell_integration_events();
    let command = events.iter().any(|event| {
        matches!(
            event,
            ShellIntegrationEvent::CommandText(text) if text.contains(&probe.expected_command)
        )
    }) && events
        .iter()
        .any(|event| matches!(event, ShellIntegrationEvent::CommandStart))
        && events
            .iter()
            .any(|event| matches!(event, ShellIntegrationEvent::CommandEnd));

    Ok(ObservedHookCapabilities {
        prompt: events
            .iter()
            .any(|event| matches!(event, ShellIntegrationEvent::Prompt)),
        command,
        cwd: events.iter().any(|event| {
            matches!(
                event,
                ShellIntegrationEvent::Cwd(cwd) if cwd == &probe.expected_cwd
            )
        }),
        exit: status.as_ref().is_some_and(|status| status.success())
            && events
                .iter()
                .any(|event| matches!(event, ShellIntegrationEvent::ExitCode(0))),
        duration: events
            .iter()
            .any(|event| matches!(event, ShellIntegrationEvent::DurationMs(_))),
    })
}

fn format_hook_report(report: &HookProbeReport) -> String {
    let required = report.required.join(",");
    let observed = {
        let labels = report.observed.labels();
        if labels.is_empty() {
            "none".to_string()
        } else {
            labels.join(",")
        }
    };

    match &report.status {
        HookProbeStatus::Passed => format!(
            "pass {} source={} required={} observed={}",
            report.target, report.source, required, observed
        ),
        HookProbeStatus::Skipped(reason) => format!(
            "skip {} source={} required={} reason={}",
            report.target, report.source, required, reason
        ),
    }
}

fn select_hook_targets(
    specs: &'static [ShellHookSpec],
    filters: &[String],
) -> Result<Vec<&'static ShellHookSpec>, String> {
    if filters.is_empty() {
        return Ok(specs.iter().collect());
    }

    let mut selected = Vec::new();
    for filter in filters {
        let Some(spec) = specs
            .iter()
            .find(|spec| spec.name == filter || spec.aliases.iter().any(|alias| alias == filter))
        else {
            return Err(format!("unknown hook target: {filter}"));
        };
        if !selected
            .iter()
            .any(|existing: &&ShellHookSpec| existing.name == spec.name)
        {
            selected.push(spec);
        }
    }

    Ok(selected)
}

fn select_hook_spec(target: &str) -> Result<&'static ShellHookSpec, String> {
    shell_hook_specs()
        .iter()
        .find(|spec| spec.name == target || spec.aliases.iter().any(|alias| alias == &target))
        .ok_or_else(|| format!("unknown shell hook target: {target}"))
}

fn required_hook_capabilities() -> &'static [HookCapability] {
    &[
        HookCapability::Prompt,
        HookCapability::Command,
        HookCapability::Cwd,
        HookCapability::Exit,
        HookCapability::Duration,
    ]
}

fn resolve_hook_probe(spec: &'static ShellHookSpec) -> Result<Option<HookProbe>, String> {
    let Some(actual_probe) = spec.actual_probe else {
        return Ok(None);
    };
    let Some(program_path) = find_executable(spec.program_candidates) else {
        return Ok(None);
    };
    actual_probe(&program_path, &(spec.render_script)()).map(Some)
}

fn shell_hook_specs() -> &'static [ShellHookSpec] {
    &[
        ShellHookSpec {
            name: "bash",
            aliases: &[],
            program_candidates: &["bash"],
            render_script: render_bash_hook,
            actual_probe: Some(bash_hook_probe),
            skip_hint: "install bash to run hook-smoke for bash",
        },
        ShellHookSpec {
            name: "zsh",
            aliases: &["oh-my-zsh"],
            program_candidates: &["zsh"],
            render_script: render_zsh_hook,
            actual_probe: Some(zsh_hook_probe),
            skip_hint: "install zsh to run hook-smoke for zsh",
        },
        ShellHookSpec {
            name: "fish",
            aliases: &[],
            program_candidates: &["fish"],
            render_script: render_fish_hook,
            actual_probe: Some(fish_hook_probe),
            skip_hint: "install fish to run hook-smoke for fish",
        },
        ShellHookSpec {
            name: "pwsh",
            aliases: &["powershell"],
            program_candidates: &["pwsh"],
            render_script: render_pwsh_hook,
            actual_probe: None,
            skip_hint: "pwsh hook generation is available; smoke requires a platform-specific probe",
        },
        ShellHookSpec {
            name: "nu",
            aliases: &["nushell"],
            program_candidates: &["nu"],
            render_script: render_nu_hook,
            actual_probe: None,
            skip_hint: "nu hook generation is available; smoke requires a platform-specific probe",
        },
    ]
}

fn bash_hook_probe(program_path: &Path, script: &str) -> Result<HookProbe, String> {
    unix_hook_probe(
        "bash",
        program_path,
        &["--noprofile", "--norc", "-i"],
        script,
    )
}

fn zsh_hook_probe(program_path: &Path, script: &str) -> Result<HookProbe, String> {
    unix_hook_probe("zsh", program_path, &["-f", "-i"], script)
}

fn fish_hook_probe(program_path: &Path, script: &str) -> Result<HookProbe, String> {
    unix_hook_probe("fish", program_path, &["-i"], script)
}

fn unix_hook_probe(
    target: &'static str,
    program_path: &Path,
    args: &[&str],
    script: &str,
) -> Result<HookProbe, String> {
    let cwd = env::current_dir()
        .map_err(|error| format!("failed to resolve current working directory: {error}"))?;
    let extension = if target == "fish" { "fish" } else { "sh" };
    let hook_path = temp_fixture_path(&format!("{target}-hook"), extension);
    fs::write(&hook_path, script)
        .map_err(|error| format!("failed to write {target} hook fixture: {error}"))?;
    make_executable_path(&hook_path)?;

    let mut command = noctrail_pty::PtyCommand::new(program_path.as_os_str());
    command.args(args.iter().copied());
    command.cwd_path(&cwd);

    Ok(HookProbe {
        target,
        source: format!(
            "program:{}+hook:{}",
            program_path.display(),
            hook_path.display()
        ),
        command,
        initial_size: PtySize::new(120, 24),
        input: format!(
            "source '{}'\rprintf 'NOCTRAIL_HOOK_READY\\n'\rpwd\rexit\r",
            hook_path.display()
        )
        .into_bytes(),
        expected_command: "NOCTRAIL_HOOK_READY".to_string(),
        expected_cwd: cwd.display().to_string(),
        required: required_hook_capabilities(),
        cleanup_paths: vec![hook_path],
    })
}

fn cleanup_hook_probe(probe: &HookProbe) {
    for path in &probe.cleanup_paths {
        let _ = fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_target_filter_accepts_aliases() {
        let selected = select_hook_targets(
            shell_hook_specs(),
            &[String::from("powershell"), String::from("nushell")],
        )
        .expect("filters should resolve");

        assert_eq!(selected.len(), 2);
        assert_eq!(selected[0].name, "pwsh");
        assert_eq!(selected[1].name, "nu");
    }

    #[test]
    fn shell_hook_scripts_emit_required_markers() {
        for spec in shell_hook_specs() {
            let script = (spec.render_script)();
            for marker in [
                "Noctrail;",
                "\"Prompt\"",
                "\"CommandStart\"",
                "\"CommandText;",
                "\"ExitCode;",
                "\"DurationMs;",
                "\"CommandEnd\"",
            ] {
                assert!(
                    script.contains(marker),
                    "{} hook was missing marker {marker}",
                    spec.name
                );
            }
        }
    }

    #[test]
    fn shell_hook_scripts_use_shell_native_integration_points() {
        assert!(render_bash_hook().contains("PROMPT_COMMAND"));
        assert!(render_bash_hook().contains("trap '__noctrail_preexec' DEBUG"));
        assert!(render_zsh_hook().contains("add-zsh-hook preexec"));
        assert!(render_fish_hook().contains("fish_preexec"));
        assert!(render_pwsh_hook().contains("Set-PSReadLineKeyHandler"));
        assert!(render_nu_hook().contains("pre_execution"));
    }
}
