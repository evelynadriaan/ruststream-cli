use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub app: AppConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub playback: PlaybackConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub config_version: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self { config_version: 1 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub path: PathBuf,
}

impl Default for StorageConfig {
    fn default() -> Self {
        let path = dirs::data_dir().unwrap_or_else(|| {
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            home.join(".local").join("share")
        });
        Self {
            path: path.join("clistream"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub format: String,
    pub quality: String,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            format: "mp3".to_string(),
            quality: "best".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    pub auto_start: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self { auto_start: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybackConfig {
    pub default_volume: u8,
}

impl Default for PlaybackConfig {
    fn default() -> Self {
        Self { default_volume: 80 }
    }
}

impl Config {
    pub fn config_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("clistream")
    }

    pub fn config_path() -> PathBuf {
        Self::config_dir().join("config.toml")
    }

    pub fn load() -> Result<Self> {
        let config_path = Self::config_path();

        if config_path.exists() {
            let content = fs::read_to_string(&config_path)
                .with_context(|| format!("Failed to read config from {}", config_path.display()))?;
            let config: Config =
                toml::from_str(&content).with_context(|| "Failed to parse config file")?;
            Ok(config)
        } else {
            Ok(Config::default())
        }
    }

    #[allow(dead_code)]
    pub fn save(&self) -> Result<()> {
        let config_dir = Self::config_dir();
        fs::create_dir_all(&config_dir).with_context(|| {
            format!(
                "Failed to create config directory: {}",
                config_dir.display()
            )
        })?;

        let config_path = Self::config_path();
        let content = toml::to_string_pretty(self).with_context(|| "Failed to serialize config")?;

        fs::write(&config_path, content)
            .with_context(|| format!("Failed to write config to {}", config_path.display()))?;

        Ok(())
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.storage.path
    }

    pub fn audio_dir(&self) -> PathBuf {
        self.storage.path.join("audio")
    }

    pub fn db_path(&self) -> PathBuf {
        self.storage.path.join("clistream.db")
    }

    pub fn socket_path(&self) -> PathBuf {
        let runtime_socket = dirs::runtime_dir()
            .map(|p| p.join("clistream"))
            .unwrap_or_else(|| self.storage.path.clone());
        runtime_socket.join("clistream.sock")
    }

    pub fn pid_path(&self) -> PathBuf {
        self.storage.path.join("clistream.pid")
    }

    pub fn ensure_dirs(&self) -> Result<()> {
        fs::create_dir_all(self.data_dir()).with_context(|| {
            format!(
                "Failed to create data directory: {}",
                self.data_dir().display()
            )
        })?;
        fs::create_dir_all(self.audio_dir()).with_context(|| {
            format!(
                "Failed to create audio directory: {}",
                self.audio_dir().display()
            )
        })?;
        if let Some(parent) = self.socket_path().parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create runtime socket directory: {}",
                    parent.display()
                )
            })?;
        }
        Ok(())
    }
}
