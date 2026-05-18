use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

mod platform;

const ENV_APP: &str = "NOCTRAIL_INSTALLER_APP";
const ENV_DMG: &str = "NOCTRAIL_INSTALLER_DMG";
const ENV_DEB: &str = "NOCTRAIL_INSTALLER_DEB";
const ENV_APPIMAGE: &str = "NOCTRAIL_INSTALLER_APPIMAGE";
const ENV_RPM: &str = "NOCTRAIL_INSTALLER_RPM";
const ENV_MSI: &str = "NOCTRAIL_INSTALLER_MSI";

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct InstallerArtifacts {
    app: Option<PathBuf>,
    dmg: Option<PathBuf>,
    deb: Option<PathBuf>,
    appimage: Option<PathBuf>,
    rpm: Option<PathBuf>,
    msi: Option<PathBuf>,
}

pub fn run_installer_smoke() -> Result<(), String> {
    let artifacts = discover_installer_artifacts()?;
    platform::run_installer_smoke(&artifacts)
}

fn discover_installer_artifacts() -> Result<InstallerArtifacts, String> {
    let mut artifacts = InstallerArtifacts::default();
    for root in installer_search_roots()? {
        if root.exists() {
            discover_under(&root, &mut artifacts)?;
        }
    }
    merge_env_overrides(&mut artifacts);
    Ok(artifacts)
}

fn installer_search_roots() -> Result<Vec<PathBuf>, String> {
    let cwd = env::current_dir().map_err(|error| format!("resolve cwd: {error}"))?;
    Ok(vec![
        cwd.clone(),
        cwd.join("target"),
        cwd.join("crates/noctrail-app"),
        cwd.join("crates/noctrail-app/target"),
    ])
}

fn discover_under(root: &Path, artifacts: &mut InstallerArtifacts) -> Result<(), String> {
    let entries =
        fs::read_dir(root).map_err(|error| format!("read {}: {error}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("read dir entry: {error}"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("stat {}: {error}", path.display()))?;
        if file_type.is_dir() {
            if path.extension() == Some(OsStr::new("app")) {
                replace_if_newer(&mut artifacts.app, &path);
                continue;
            }
            discover_under(&path, artifacts)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        match path.extension().and_then(OsStr::to_str) {
            Some("dmg") => replace_if_newer(&mut artifacts.dmg, &path),
            Some("deb") => replace_if_newer(&mut artifacts.deb, &path),
            Some("rpm") => replace_if_newer(&mut artifacts.rpm, &path),
            Some("msi") => replace_if_newer(&mut artifacts.msi, &path),
            Some("AppImage") => replace_if_newer(&mut artifacts.appimage, &path),
            _ => {}
        }
    }
    Ok(())
}

fn replace_if_newer(slot: &mut Option<PathBuf>, candidate: &Path) {
    match slot {
        Some(current) => {
            let current_time = path_mtime(current);
            let candidate_time = path_mtime(candidate);
            if candidate_time >= current_time {
                *slot = Some(candidate.to_path_buf());
            }
        }
        None => *slot = Some(candidate.to_path_buf()),
    }
}

fn path_mtime(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .unwrap_or(UNIX_EPOCH)
}

fn merge_env_overrides(artifacts: &mut InstallerArtifacts) {
    apply_env_override(ENV_APP, &mut artifacts.app);
    apply_env_override(ENV_DMG, &mut artifacts.dmg);
    apply_env_override(ENV_DEB, &mut artifacts.deb);
    apply_env_override(ENV_APPIMAGE, &mut artifacts.appimage);
    apply_env_override(ENV_RPM, &mut artifacts.rpm);
    apply_env_override(ENV_MSI, &mut artifacts.msi);
}

fn apply_env_override(key: &str, slot: &mut Option<PathBuf>) {
    if let Some(path) = env::var_os(key) {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            *slot = Some(path);
        }
    }
}

fn require_path(slot: &Option<PathBuf>, label: &str, env_key: &str) -> Result<PathBuf, String> {
    slot.clone().ok_or_else(|| {
        format!(
            "missing {label} artifact; set {env_key} or place the installer output under target/"
        )
    })
}

fn run_packaged_smoke(binary: &Path) -> Result<(), String> {
    if !binary.exists() {
        return Err(format!("missing packaged binary {}", binary.display()));
    }
    #[cfg(target_os = "linux")]
    if binary.extension() == Some(OsStr::new("AppImage")) {
        return run_command_with_env(
            binary,
            &[OsStr::new("smoke")],
            &[("APPIMAGE_EXTRACT_AND_RUN", OsStr::new("1"))],
        );
    }
    run_command(binary, &[OsStr::new("smoke")])
}

fn run_command(program: impl AsRef<OsStr>, args: &[&OsStr]) -> Result<(), String> {
    run_command_with_env(program, args, &[])
}

fn run_command_with_env(
    program: impl AsRef<OsStr>,
    args: &[&OsStr],
    envs: &[(&str, &OsStr)],
) -> Result<(), String> {
    let mut command = Command::new(&program);
    command.args(args);
    for (key, value) in envs {
        command.env(key, value);
    }
    let status = command
        .status()
        .map_err(|error| format!("run {:?}: {error}", program.as_ref()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "{:?} exited with status {}",
            program.as_ref(),
            status
        ))
    }
}

#[cfg(target_os = "linux")]
fn run_shell_command(script: &str) -> Result<(), String> {
    run_command("sh", &[OsStr::new("-lc"), OsStr::new(script)])
}

fn temp_smoke_dir(prefix: &str) -> Result<PathBuf, String> {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("clock drift: {error}"))?
        .as_millis();
    let root = env::temp_dir().join(format!("{prefix}-{suffix}-{}", std::process::id()));
    fs::create_dir_all(&root).map_err(|error| format!("create {}: {error}", root.display()))?;
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_latest_artifacts_from_search_roots() -> Result<(), String> {
        let root = temp_smoke_dir("noctrail-installer-discovery")?;
        let target = root.join("target/release/bundle");
        fs::create_dir_all(&target)
            .map_err(|error| format!("create {}: {error}", target.display()))?;
        let app = target.join("Noctrail.app");
        fs::create_dir_all(&app).map_err(|error| format!("create {}: {error}", app.display()))?;
        let dmg = target.join("noctrail-new.dmg");
        fs::write(&dmg, b"dmg").map_err(|error| format!("write {}: {error}", dmg.display()))?;
        let older_dmg = target.join("noctrail-old.dmg");
        fs::write(&older_dmg, b"dmg")
            .map_err(|error| format!("write {}: {error}", older_dmg.display()))?;
        let deb = target.join("noctrail.deb");
        fs::write(&deb, b"deb").map_err(|error| format!("write {}: {error}", deb.display()))?;
        let appimage = target.join("noctrail.AppImage");
        fs::write(&appimage, b"appimage")
            .map_err(|error| format!("write {}: {error}", appimage.display()))?;
        let rpm = target.join("noctrail.rpm");
        fs::write(&rpm, b"rpm").map_err(|error| format!("write {}: {error}", rpm.display()))?;
        let msi = target.join("noctrail.msi");
        fs::write(&msi, b"msi").map_err(|error| format!("write {}: {error}", msi.display()))?;

        let mut artifacts = InstallerArtifacts::default();
        discover_under(&root, &mut artifacts)?;

        assert_eq!(artifacts.app, Some(app));
        assert_eq!(artifacts.dmg, Some(older_dmg));
        assert_eq!(artifacts.deb, Some(deb));
        assert_eq!(artifacts.appimage, Some(appimage));
        assert_eq!(artifacts.rpm, Some(rpm));
        assert_eq!(artifacts.msi, Some(msi));
        Ok(())
    }

    #[test]
    fn env_overrides_replace_discovered_artifacts() {
        let mut artifacts = InstallerArtifacts {
            dmg: Some(PathBuf::from("/tmp/old.dmg")),
            ..InstallerArtifacts::default()
        };
        unsafe {
            env::set_var(ENV_DMG, "/tmp/new.dmg");
        }
        merge_env_overrides(&mut artifacts);
        unsafe {
            env::remove_var(ENV_DMG);
        }

        assert_eq!(artifacts.dmg, Some(PathBuf::from("/tmp/new.dmg")));
    }
}
