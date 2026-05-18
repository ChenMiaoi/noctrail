use super::InstallerArtifacts;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;

pub(super) fn run_installer_smoke(artifacts: &InstallerArtifacts) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        return macos::run(artifacts);
    }

    #[cfg(target_os = "linux")]
    {
        return linux::run(artifacts);
    }

    #[cfg(target_os = "windows")]
    {
        return windows::run(artifacts);
    }

    #[allow(unreachable_code)]
    Err(format!(
        "installer-smoke is not implemented for {}",
        std::env::consts::OS
    ))
}
