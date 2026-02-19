use anyhow::{Context, Result};
use interprocess::TryClone;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use crate::models::{PlaybackState, RepeatMode, Track};

pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DaemonErrorCode {
    DaemonUnavailable,
    IpcProtocolMismatch,
    TrackNotFound,
    TrackUnavailable,
    InvalidTimeFormat,
    DependencyMissing,
    DownloadFailed,
    AudioInitFailed,
    AudioPlayFailed,
    DbError,
    InternalError,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonRequestEnvelope {
    pub protocol_version: u16,
    pub command: DaemonCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonResponseEnvelope {
    pub protocol_version: u16,
    pub response: DaemonResponse,
    #[serde(default)]
    pub error_code: Option<DaemonErrorCode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonCommand {
    Play {
        track: Track,
    },
    PlayQueue {
        tracks: Vec<Track>,
        start_index: usize,
    },
    Pause,
    Resume,
    Stop,
    Next,
    Previous,
    Seek {
        position: u64,
    },
    SetVolume {
        volume: u8,
    },
    SetShuffle {
        enabled: bool,
    },
    SetRepeat {
        mode: RepeatMode,
    },
    QueueAdd {
        track: Track,
    },
    QueueClear,
    GetStatus,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonResponse {
    Ok,
    Status(PlaybackState),
    Error {
        #[serde(default)]
        code: Option<DaemonErrorCode>,
        message: String,
    },
}

pub struct DaemonClient {
    socket_path: std::path::PathBuf,
}

impl DaemonClient {
    pub fn new(socket_path: impl AsRef<Path>) -> Self {
        Self {
            socket_path: socket_path.as_ref().to_path_buf(),
        }
    }

    pub fn is_daemon_running(&self) -> bool {
        self.socket_path.exists() && self.send_command(DaemonCommand::GetStatus).is_ok()
    }

    pub fn wait_until_ready(&self, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if self.send_command(DaemonCommand::GetStatus).is_ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(75));
        }
        false
    }

    pub fn send_command(&self, command: DaemonCommand) -> Result<DaemonResponse> {
        use interprocess::local_socket::GenericFilePath;
        use interprocess::local_socket::prelude::*;

        let path = self.socket_path.as_os_str();
        let name = path
            .to_fs_name::<GenericFilePath>()
            .with_context(|| "Invalid socket path")?;

        let conn = interprocess::local_socket::Stream::connect(name).with_context(|| {
            format!(
                "Failed to connect to daemon at {}",
                self.socket_path.display()
            )
        })?;

        let mut writer = conn;
        let mut reader = BufReader::new(writer.try_clone()?);

        let request = DaemonRequestEnvelope {
            protocol_version: PROTOCOL_VERSION,
            command,
        };

        let msg = serde_json::to_string(&request)?;
        writeln!(writer, "{msg}")?;
        writer.flush()?;

        let mut response_line = String::new();
        reader.read_line(&mut response_line)?;

        let envelope: DaemonResponseEnvelope = serde_json::from_str(&response_line)
            .with_context(|| "Failed to parse daemon response envelope")?;

        if envelope.protocol_version != PROTOCOL_VERSION {
            anyhow::bail!(
                "Daemon protocol mismatch (client={}, daemon={})",
                PROTOCOL_VERSION,
                envelope.protocol_version
            );
        }

        let mut response = envelope.response;
        if let DaemonResponse::Error { code, .. } = &mut response
            && code.is_none()
        {
            *code = envelope.error_code;
        }

        Ok(response)
    }

    pub fn play(&self, track: Track) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::Play { track })
    }

    #[allow(dead_code)]
    pub fn play_queue(&self, tracks: Vec<Track>, start_index: usize) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::PlayQueue {
            tracks,
            start_index,
        })
    }

    pub fn pause(&self) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::Pause)
    }

    pub fn resume(&self) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::Resume)
    }

    pub fn stop(&self) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::Stop)
    }

    #[allow(dead_code)]
    pub fn next(&self) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::Next)
    }

    #[allow(dead_code)]
    pub fn previous(&self) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::Previous)
    }

    pub fn seek(&self, position: u64) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::Seek { position })
    }

    pub fn set_volume(&self, volume: u8) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::SetVolume { volume })
    }

    #[allow(dead_code)]
    pub fn set_shuffle(&self, enabled: bool) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::SetShuffle { enabled })
    }

    #[allow(dead_code)]
    pub fn set_repeat(&self, mode: RepeatMode) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::SetRepeat { mode })
    }

    #[allow(dead_code)]
    pub fn queue_add(&self, track: Track) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::QueueAdd { track })
    }

    #[allow(dead_code)]
    pub fn queue_clear(&self) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::QueueClear)
    }

    pub fn get_status(&self) -> Result<PlaybackState> {
        match self.send_command(DaemonCommand::GetStatus)? {
            DaemonResponse::Status(state) => Ok(state),
            DaemonResponse::Error { code, message } => {
                if let Some(code) = code {
                    anyhow::bail!("{:?}: {}", code, message)
                } else {
                    anyhow::bail!("{}", message)
                }
            }
            _ => anyhow::bail!("Unexpected response"),
        }
    }

    pub fn shutdown(&self) -> Result<DaemonResponse> {
        self.send_command(DaemonCommand::Shutdown)
    }
}
