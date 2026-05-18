use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::installer::{
    ENV_APP, ENV_DMG, InstallerArtifacts, require_path, run_command, run_packaged_smoke,
    temp_smoke_dir,
};

pub(super) fn run(artifacts: &InstallerArtifacts) -> Result<(), String> {
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

struct MountedDmg {
    mount_root: PathBuf,
}

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

impl Drop for MountedDmg {
    fn drop(&mut self) {
        let _ = Command::new("hdiutil")
            .arg("detach")
            .arg(&self.mount_root)
            .status();
    }
}

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

fn installed_app_binary(app: &Path) -> Result<PathBuf, String> {
    let binary = app.join("Contents/MacOS/noctrail-app");
    if !binary.exists() {
        return Err(format!("missing app binary {}", binary.display()));
    }
    Ok(binary)
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
