use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use noctrail_config::{Config, LayoutSplitAxis, RendererBackend as ConfigRendererBackend};
use noctrail_pty::{PtySize, ResolvedShell};
use noctrail_render::{
    FontDiagnostics, FontFamilyDiagnostics, FontPreferences, probe_font_diagnostics,
    probe_gpu_backend,
};
use noctrail_runtime::PaneRuntimeRegistry;

use crate::commands::pty::{collect_runtime_events, pty_smoke_probe};

pub(crate) struct DoctorPtySummary {
    pub(crate) process_id: Option<u32>,
    pub(crate) size: PtySize,
}

pub(crate) struct DoctorConfigSummary {
    pub(crate) path: String,
    pub(crate) renderer_backend: &'static str,
    pub(crate) default_split_axis: &'static str,
    pub(crate) startup_workspace: u8,
    pub(crate) agent_enabled: bool,
}

pub(crate) struct DoctorFontSummary {
    pub(crate) primary_label: String,
    pub(crate) sample_statuses: Vec<String>,
}

pub(crate) struct DoctorPermissionsSummary {
    pub(crate) cwd_readable: bool,
    pub(crate) cwd_writable: bool,
    pub(crate) home_readable: Option<bool>,
    pub(crate) temp_writable: bool,
}

pub(crate) fn print_doctor() {
    println!("noctrail {}", env!("CARGO_PKG_VERSION"));
    println!("target: {}", env::consts::OS);
    println!("arch: {}", env::consts::ARCH);
    let shell = ResolvedShell::detect();
    println!(
        "shell={} source={}",
        shell.command().program().to_string_lossy(),
        shell.source().label()
    );

    match doctor_pty_summary() {
        Ok(summary) => println!(
            "pty=ok pid={} size={}x{}",
            summary
                .process_id
                .map(|pid| pid.to_string())
                .unwrap_or_else(|| "none".to_string()),
            summary.size.cols,
            summary.size.rows,
        ),
        Err(error) => println!("pty=error {error}"),
    }

    match doctor_gpu_summary() {
        Ok(summary) => println!(
            "gpu={} backend={:?} device={:?}",
            summary.adapter_name, summary.backend, summary.device_type
        ),
        Err(error) => println!("gpu=error {error}"),
    }

    let font = doctor_font_summary();
    println!(
        "font.primary={} samples={}",
        font.primary_label,
        font.sample_statuses.join(",")
    );

    match doctor_config_summary(None) {
        Ok(summary) => println!(
            "config.path={} renderer={} split_axis={} workspace={} agent={}",
            summary.path,
            summary.renderer_backend,
            summary.default_split_axis,
            summary.startup_workspace,
            on_off(summary.agent_enabled),
        ),
        Err(error) => println!("config=error {error}"),
    }

    match doctor_permissions_summary() {
        Ok(summary) => {
            println!(
                "permissions.cwd_readable={} cwd_writable={} temp_writable={}",
                summary.cwd_readable, summary.cwd_writable, summary.temp_writable
            );
            println!(
                "permissions.home_readable={}",
                summary
                    .home_readable
                    .map(|readable| readable.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            );
        }
        Err(error) => println!("permissions=error {error}"),
    }
}

pub(crate) fn doctor_pty_summary() -> Result<DoctorPtySummary, String> {
    let probe = pty_smoke_probe("NOCTRAIL_DOCTOR_PTY")?;
    let mut registry = PaneRuntimeRegistry::new();
    let pane_id = registry
        .spawn_shell(PtySize::new(80, 24))
        .map_err(|error| format!("failed to spawn PTY shell: {error}"))?;
    let process_id = registry
        .get(pane_id)
        .and_then(noctrail_runtime::PaneRuntime::process_id);
    let size = registry
        .get(pane_id)
        .map(noctrail_runtime::PaneRuntime::size)
        .ok_or_else(|| format!("PTY doctor pane {pane_id:?} disappeared before probing"))?;
    registry
        .write_input(pane_id, &probe.input)
        .map_err(|error| format!("failed to write PTY doctor probe: {error}"))?;
    let (output, _) = collect_runtime_events(&mut registry, pane_id)
        .map_err(|error| format!("failed to read PTY doctor output: {error}"))?;
    if registry.contains(pane_id) {
        let _ = registry.close(pane_id);
    }

    let haystack = String::from_utf8_lossy(&output);
    for expected in &probe.expected_fragments {
        if !haystack.contains(expected) {
            return Err(format!(
                "doctor PTY probe output missing {:?}; output was {:?}",
                expected, haystack
            ));
        }
    }

    Ok(DoctorPtySummary { process_id, size })
}

pub(crate) fn doctor_config_summary(path: Option<&Path>) -> Result<DoctorConfigSummary, String> {
    let (path_label, config) = match path {
        Some(path) => (
            path.display().to_string(),
            Config::load_from_path(path).map_err(|error| format!("{error}"))?,
        ),
        None => ("(default)".to_string(), Config::default()),
    };

    Ok(DoctorConfigSummary {
        path: path_label,
        renderer_backend: match config.renderer.backend {
            ConfigRendererBackend::Gpu => "gpu",
            ConfigRendererBackend::Software => "software",
        },
        default_split_axis: match config.layout.default_split_axis {
            LayoutSplitAxis::Auto => "auto",
            LayoutSplitAxis::Horizontal => "horizontal",
            LayoutSplitAxis::Vertical => "vertical",
        },
        startup_workspace: config.layout.startup_workspace,
        agent_enabled: config.agent.enabled,
    })
}

pub(crate) fn doctor_font_summary() -> DoctorFontSummary {
    let diagnostics = probe_font_diagnostics(&FontPreferences::default());
    DoctorFontSummary {
        primary_label: diagnostics
            .primary
            .resolved_family
            .clone()
            .unwrap_or_else(|| diagnostics.primary.requested_family.clone()),
        sample_statuses: diagnostics
            .samples
            .iter()
            .map(|sample| format!("{}={}", sample.label, sample.status.label()))
            .collect(),
    }
}

pub(crate) fn doctor_permissions_summary() -> Result<DoctorPermissionsSummary, String> {
    let cwd = env::current_dir().map_err(|error| format!("failed to resolve cwd: {error}"))?;
    let cwd_readable = directory_readable(&cwd);
    let cwd_writable = directory_writable(&cwd);
    let home = doctor_home_dir();
    let home_readable = home.as_deref().map(directory_readable);
    let temp_writable = directory_writable(&env::temp_dir());

    Ok(DoctorPermissionsSummary {
        cwd_readable,
        cwd_writable,
        home_readable,
        temp_writable,
    })
}

pub(crate) fn print_doctor_shell() {
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

pub(crate) fn print_doctor_pty() -> Result<(), String> {
    let summary = doctor_pty_summary()?;
    println!(
        "pty.status=ok\npty.pid={}\npty.size={}x{}",
        summary
            .process_id
            .map(|pid| pid.to_string())
            .unwrap_or_else(|| "none".to_string()),
        summary.size.cols,
        summary.size.rows,
    );
    Ok(())
}

pub(crate) fn print_doctor_gpu() -> Result<(), String> {
    let diagnostics = doctor_gpu_summary()?;
    println!("gpu.adapter={}", diagnostics.adapter_name);
    println!("gpu.backend={:?}", diagnostics.backend);
    println!("gpu.device_type={:?}", diagnostics.device_type);
    Ok(())
}

pub(crate) fn doctor_gpu_summary() -> Result<noctrail_render::GpuBackendDiagnostics, String> {
    probe_gpu_backend().map_err(|error| error.to_string())
}

pub(crate) fn print_doctor_font() {
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

pub(crate) fn print_doctor_config(path: Option<&Path>) -> Result<(), String> {
    let summary = doctor_config_summary(path)?;
    println!("config.path={}", summary.path);
    println!("config.status=ok");
    println!("config.renderer={}", summary.renderer_backend);
    println!(
        "config.layout.default_split_axis={}",
        summary.default_split_axis
    );
    println!(
        "config.layout.startup_workspace={}",
        summary.startup_workspace
    );
    println!("config.agent.enabled={}", on_off(summary.agent_enabled));
    Ok(())
}

pub(crate) fn print_doctor_permissions() -> Result<(), String> {
    let summary = doctor_permissions_summary()?;
    println!("permissions.cwd.readable={}", summary.cwd_readable);
    println!("permissions.cwd.writable={}", summary.cwd_writable);
    println!("permissions.temp.writable={}", summary.temp_writable);
    println!(
        "permissions.home.readable={}",
        summary
            .home_readable
            .map(|readable| readable.to_string())
            .unwrap_or_else(|| "unknown".to_string())
    );
    Ok(())
}

fn on_off(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
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
    match diagnostics.resolved_source {
        Some(source) => println!("{prefix}.source={}", source.label()),
        None => println!("{prefix}.source=(missing)"),
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

fn doctor_home_dir() -> Option<PathBuf> {
    env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn directory_readable(path: &Path) -> bool {
    fs::read_dir(path).is_ok()
}

fn directory_writable(path: &Path) -> bool {
    let probe_name = format!(
        ".noctrail-doctor-write-{}-{}",
        process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default()
    );
    let probe_path = path.join(probe_name);
    let wrote = fs::write(&probe_path, b"ok").is_ok();
    let _ = fs::remove_file(&probe_path);
    wrote
}
