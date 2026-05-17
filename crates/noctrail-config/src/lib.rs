//! Configuration boundary for Noctrail.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RendererBackend {
    #[default]
    Gpu,
    Software,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default)]
pub struct RendererConfig {
    pub backend: RendererBackend,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub renderer: RendererConfig,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
}

impl Config {
    pub const fn new() -> Self {
        Self {
            renderer: RendererConfig {
                backend: RendererBackend::Gpu,
            },
        }
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        toml::from_str(&raw).map_err(|source| ConfigError::Parse {
            path: path.to_path_buf(),
            source,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn defaults_to_gpu_backend() {
        assert_eq!(Config::default().renderer.backend, RendererBackend::Gpu);
    }

    #[test]
    fn loads_renderer_backend_from_toml() {
        let path = temp_config_path("backend-software");
        fs::write(&path, "[renderer]\nbackend = \"software\"\n").expect("write config");

        let config = Config::load_from_path(&path).expect("load config");
        assert_eq!(config.renderer.backend, RendererBackend::Software);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn parse_errors_keep_the_config_path() {
        let path = temp_config_path("parse-error");
        fs::write(&path, "[renderer\nbackend = \"gpu\"\n").expect("write config");

        let error = Config::load_from_path(&path).expect_err("config should fail");
        match error {
            ConfigError::Parse {
                path: error_path, ..
            } => assert_eq!(error_path, path),
            other => panic!("unexpected error: {other:?}"),
        }

        let _ = fs::remove_file(path);
    }

    fn temp_config_path(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("noctrail-config-{label}-{unique}.toml"))
    }
}
