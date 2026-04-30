use std::path::PathBuf;

use directories::ProjectDirs;
use iridium_core::{HashAlg, ImageFormat};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub last_output_dir: Option<PathBuf>,
    pub default_hash_algs: Vec<HashAlg>,
    pub default_format: ImageFormat,
    pub window_width: f32,
    pub window_height: f32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            last_output_dir: None,
            default_hash_algs: vec![HashAlg::Sha256],
            default_format: ImageFormat::Ewf,
            window_width: 1024.0,
            window_height: 768.0,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    ProjectDirs::from("org", "iridium", "iridium")
        .map(|pd| pd.config_dir().join("config.toml"))
}

pub fn load() -> Config {
    let Some(path) = config_path() else {
        return Config::default();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Config::default();
    };
    toml::from_str(&raw).unwrap_or_default()
}

pub fn save(cfg: &Config) -> Result<(), AppError> {
    let path = config_path()
        .ok_or_else(|| AppError::Config("cannot determine config directory".into()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| AppError::Config(format!("create config dir: {e}")))?;
    }
    let serialized = toml::to_string_pretty(cfg)
        .map_err(|e| AppError::Config(format!("serialize config: {e}")))?;
    std::fs::write(&path, serialized)
        .map_err(|e| AppError::Config(format!("write config: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let cfg = Config {
            last_output_dir: Some(PathBuf::from("/tmp/evidence")),
            default_hash_algs: vec![HashAlg::Md5, HashAlg::Sha256],
            default_format: ImageFormat::Raw,
            window_width: 1280.0,
            window_height: 800.0,
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        let restored: Config = toml::from_str(&s).unwrap();
        assert_eq!(restored.last_output_dir, cfg.last_output_dir);
        assert_eq!(restored.default_hash_algs, cfg.default_hash_algs);
        assert_eq!(restored.default_format as u8, cfg.default_format as u8);
        assert!((restored.window_width - cfg.window_width).abs() < f32::EPSILON);
        assert!((restored.window_height - cfg.window_height).abs() < f32::EPSILON);
    }

    #[test]
    fn default_is_valid_toml() {
        let s = toml::to_string_pretty(&Config::default()).unwrap();
        let _: Config = toml::from_str(&s).unwrap();
    }
}
