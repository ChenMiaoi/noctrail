use std::{
    env,
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

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
    #[cfg(target_os = "macos")]
    {
        return run_macos_installer_smoke(&artifacts);
    }
    #[cfg(target_os = "linux")]
    {
        return run_linux_installer_smoke(&artifacts);
    }
    #[cfg(target_os = "windows")]
    {
        return run_windows_installer_smoke(&artifacts);
    }
    #[allow(unreachable_code)]
    Err(format!(
        "installer-smoke is not implemented for {}",
        env::consts::OS
    ))
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

#[cfg(target_os = "macos")]
fn run_macos_installer_smoke(artifacts: &InstallerArtifacts) -> Result<(), String> {
    let app = require_path(&artifacts.app, ".app bundle", ENV_APP)?;
    let dmg = require_path(&artifacts.dmg, ".dmg bundle", ENV_DMG)?;
    let temp = temp_smoke_dir("noctrail-installer-macos")?;
    let install_root = temp.join("Applications");
    let install_app = install_root.join(
        app.file_name()
            .ok_or_else(|| format!("invalid app bundle path {}", app.display()))?,
    );
    fs::create_dir_all(&install_root)
        .map_err(|error| format!("create {}: {error}", install_root.display()))?;

    copy_dir_recursive(&app, &install_app)?;
    run_packaged_smoke(&installed_app_binary(&install_app)?)?;

    fs::remove_dir_all(&install_app)
        .map_err(|error| format!("remove {}: {error}", install_app.display()))?;

    let mount_root = temp.join("mnt");
    fs::create_dir_all(&mount_root)
        .map_err(|error| format!("create {}: {error}", mount_root.display()))?;
    let _mounted = MountedDmg::attach(&dmg, &mount_root)?;
    let mounted_app = find_first_app_bundle(&mount_root)?;
    copy_dir_recursive(&mounted_app, &install_app)?;
    run_packaged_smoke(&installed_app_binary(&install_app)?)?;

    fs::remove_dir_all(&install_app)
        .map_err(|error| format!("remove {}: {error}", install_app.display()))?;
    if install_app.exists() {
        return Err(format!("expected {} to be removed", install_app.display()));
    }

    println!(
        "platform=macos install={} upgrade_source={} uninstall=ok",
        app.display(),
        dmg.display(),
    );
    println!("installer smoke ok");
    Ok(())
}

#[cfg(target_os = "macos")]
struct MountedDmg {
    mount_root: PathBuf,
}

#[cfg(target_os = "macos")]
impl MountedDmg {
    fn attach(dmg: &Path, mount_root: &Path) -> Result<Self, String> {
        run_command(
            "hdiutil",
            &[
                OsStr::new("attach"),
                OsStr::new("-nobrowse"),
                OsStr::new("-readonly"),
                OsStr::new("-mountpoint"),
                mount_root.as_os_str(),
                dmg.as_os_str(),
            ],
        )?;
        Ok(Self {
            mount_root: mount_root.to_path_buf(),
        })
    }
}

#[cfg(target_os = "macos")]
impl Drop for MountedDmg {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .arg("detach")
            .arg(&self.mount_root)
            .status();
    }
}

#[cfg(target_os = "macos")]
fn find_first_app_bundle(root: &Path) -> Result<PathBuf, String> {
    let entries =
        fs::read_dir(root).map_err(|error| format!("read {}: {error}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("read dir entry: {error}"))?;
        let path = entry.path();
        if path.extension() == Some(OsStr::new("app")) {
            return Ok(path);
        }
    }
    Err(format!("no .app bundle found under {}", root.display()))
}

#[cfg(target_os = "macos")]
fn installed_app_binary(app: &Path) -> Result<PathBuf, String> {
    let binary = app.join("Contents/MacOS/noctrail-app");
    if !binary.exists() {
        return Err(format!("missing app binary {}", binary.display()));
    }
    Ok(binary)
}

#[cfg(target_os = "linux")]
fn run_linux_installer_smoke(artifacts: &InstallerArtifacts) -> Result<(), String> {
    let deb = require_path(&artifacts.deb, ".deb package", ENV_DEB)?;
    let appimage = require_path(&artifacts.appimage, ".AppImage bundle", ENV_APPIMAGE)?;
    let rpm = require_path(&artifacts.rpm, ".rpm package", ENV_RPM)?;
    let temp = temp_smoke_dir("noctrail-installer-linux")?;

    let appimage_copy = temp.join("Noctrail.AppImage");
    fs::copy(&appimage, &appimage_copy)
        .map_err(|error| format!("copy {}: {error}", appimage.display()))?;
    make_executable(&appimage_copy)?;
    run_packaged_smoke(&appimage_copy)?;

    let deb_root = temp.join("deb-root");
    fs::create_dir_all(&deb_root)
        .map_err(|error| format!("create {}: {error}", deb_root.display()))?;
    run_command(
        "dpkg-deb",
        &[OsStr::new("-x"), deb.as_os_str(), deb_root.as_os_str()],
    )?;
    run_packaged_smoke(&deb_root.join("usr/bin/noctrail-app"))?;

    let rpm_root = temp.join("rpm-root");
    fs::create_dir_all(&rpm_root)
        .map_err(|error| format!("create {}: {error}", rpm_root.display()))?;
    if command_exists("bsdtar") {
        run_command(
            "bsdtar",
            &[
                OsStr::new("-xf"),
                rpm.as_os_str(),
                OsStr::new("-C"),
                rpm_root.as_os_str(),
            ],
        )?;
    } else if command_exists("rpm2cpio") && command_exists("cpio") {
        let pipeline = format!(
            "rpm2cpio '{}' | (cd '{}' && cpio -idm --quiet)",
            rpm.display(),
            rpm_root.display()
        );
        run_shell_command(&pipeline)?;
    } else {
        return Err("need bsdtar or rpm2cpio+cpio to inspect RPM payloads".to_string());
    }
    run_packaged_smoke(&rpm_root.join("usr/bin/noctrail-app"))?;

    fs::remove_file(&appimage_copy)
        .map_err(|error| format!("remove {}: {error}", appimage_copy.display()))?;
    fs::remove_dir_all(&deb_root)
        .map_err(|error| format!("remove {}: {error}", deb_root.display()))?;
    fs::remove_dir_all(&rpm_root)
        .map_err(|error| format!("remove {}: {error}", rpm_root.display()))?;

    println!(
        "platform=linux appimage={} deb={} rpm={} uninstall=ok",
        appimage.display(),
        deb.display(),
        rpm.display(),
    );
    println!("installer smoke ok");
    Ok(())
}

#[cfg(target_os = "windows")]
fn run_windows_installer_smoke(artifacts: &InstallerArtifacts) -> Result<(), String> {
    let msi = require_path(&artifacts.msi, ".msi package", ENV_MSI)?;
    let temp = temp_smoke_dir("noctrail-installer-windows")?;
    let install_root = temp.join("Noctrail");
    fs::create_dir_all(&install_root)
        .map_err(|error| format!("create {}: {error}", install_root.display()))?;

    run_command(
        "msiexec",
        &[
            OsStr::new("/i"),
            msi.as_os_str(),
            OsStr::new("/qn"),
            OsStr::new("INSTALLDIR"),
            install_root.as_os_str(),
        ],
    )?;
    run_packaged_smoke(&install_root.join("noctrail-app.exe"))?;
    run_command(
        "msiexec",
        &[
            OsStr::new("/i"),
            msi.as_os_str(),
            OsStr::new("/qn"),
            OsStr::new("REINSTALL=ALL"),
            OsStr::new("REINSTALLMODE=vomus"),
            OsStr::new("INSTALLDIR"),
            install_root.as_os_str(),
        ],
    )?;
    run_packaged_smoke(&install_root.join("noctrail-app.exe"))?;
    run_command(
        "msiexec",
        &[OsStr::new("/x"), msi.as_os_str(), OsStr::new("/qn")],
    )?;

    println!("platform=windows msi={} uninstall=ok", msi.display(),);
    println!("installer smoke ok");
    Ok(())
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

#[cfg(target_os = "linux")]
fn command_exists(name: &str) -> bool {
    env::var_os("PATH")
        .map(|paths| env::split_paths(&paths).any(|path| path.join(name).exists()))
        .unwrap_or(false)
}

#[cfg(target_os = "linux")]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .map_err(|error| format!("stat {}: {error}", path.display()))?
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions)
        .map_err(|error| format!("chmod {}: {error}", path.display()))
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

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<(), String> {
    if source.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        fs::copy(source, destination).map_err(|error| {
            format!(
                "copy file {} -> {}: {error}",
                source.display(),
                destination.display(),
            )
        })?;
        return Ok(());
    }

    fs::create_dir_all(destination)
        .map_err(|error| format!("create {}: {error}", destination.display()))?;
    let entries =
        fs::read_dir(source).map_err(|error| format!("read {}: {error}", source.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("read dir entry: {error}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("stat {}: {error}", source_path.display()))?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "copy file {} -> {}: {error}",
                    source_path.display(),
                    destination_path.display(),
                )
            })?;
        }
    }
    Ok(())
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
