use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink};
use serde_json::{Value, json};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

type BackendResult<T> = std::result::Result<T, AudioBackendError>;

#[derive(Debug, thiserror::Error)]
pub enum AudioBackendError {
    #[error("Failed to initialize audio output: {0}")]
    AudioInit(String),
    #[error("Failed to play audio: {0}")]
    AudioPlay(String),
    #[error("mpv is not installed. Install mpv to use streaming commands.")]
    MpvUnavailable,
    #[error("mpv IPC error: {0}")]
    MpvIpcError(String),
    #[error("mpv failed to load stream: {0}")]
    MpvLoadFailed(String),
}

pub trait PlaybackBackend {
    fn play(&mut self, source: &str) -> BackendResult<()>;
    fn pause(&mut self) -> BackendResult<()>;
    fn resume(&mut self) -> BackendResult<()>;
    fn stop(&mut self) -> BackendResult<()>;
    fn seek(&mut self, position: Duration) -> BackendResult<bool>;
    fn get_position(&mut self) -> BackendResult<Duration>;
    fn set_volume(&mut self, volume: u8) -> BackendResult<()>;
    fn is_finished(&mut self) -> BackendResult<bool>;
}

pub fn mpv_is_available() -> bool {
    Command::new("mpv")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn mpv_socket_path(data_dir: &Path) -> PathBuf {
    let base = dirs::runtime_dir()
        .map(|p| p.join("clistream"))
        .unwrap_or_else(|| data_dir.to_path_buf());
    base.join("clistream-mpv.sock")
}

pub struct RodioBackend {
    _stream: OutputStream,
    _stream_handle: OutputStreamHandle,
    sink: Sink,
    volume: Arc<AtomicU8>,
    is_playing: Arc<AtomicBool>,
}

impl RodioBackend {
    pub fn new() -> BackendResult<Self> {
        let (stream, stream_handle) =
            OutputStream::try_default().map_err(|e| AudioBackendError::AudioInit(e.to_string()))?;
        let sink = Sink::try_new(&stream_handle)
            .map_err(|e| AudioBackendError::AudioInit(e.to_string()))?;

        let volume = Arc::new(AtomicU8::new(80));
        let is_playing = Arc::new(AtomicBool::new(false));
        sink.set_volume(0.8);

        Ok(Self {
            _stream: stream,
            _stream_handle: stream_handle,
            sink,
            volume,
            is_playing,
        })
    }
}

impl PlaybackBackend for RodioBackend {
    fn play(&mut self, source: &str) -> BackendResult<()> {
        let path = Path::new(source);
        if !path.exists() {
            return Err(AudioBackendError::AudioPlay(format!(
                "Audio file not found: {}",
                path.display()
            )));
        }

        let file = File::open(path).map_err(|e| {
            AudioBackendError::AudioPlay(format!(
                "Failed to open audio file {}: {}",
                path.display(),
                e
            ))
        })?;
        let reader = BufReader::new(file);
        let decoder = Decoder::new(reader).map_err(|e| {
            AudioBackendError::AudioPlay(format!(
                "Failed to decode audio file {}: {}",
                path.display(),
                e
            ))
        })?;

        self.sink.clear();
        self.sink.append(decoder);
        self.sink.play();
        self.is_playing.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn pause(&mut self) -> BackendResult<()> {
        self.sink.pause();
        self.is_playing.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn resume(&mut self) -> BackendResult<()> {
        self.sink.play();
        if !self.sink.empty() {
            self.is_playing.store(true, Ordering::SeqCst);
        }
        Ok(())
    }

    fn stop(&mut self) -> BackendResult<()> {
        self.sink.stop();
        self.is_playing.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn seek(&mut self, position: Duration) -> BackendResult<bool> {
        Ok(self.sink.try_seek(position).is_ok())
    }

    fn get_position(&mut self) -> BackendResult<Duration> {
        Ok(self.sink.get_pos())
    }

    fn set_volume(&mut self, volume: u8) -> BackendResult<()> {
        let vol = volume.min(100);
        self.volume.store(vol, Ordering::SeqCst);
        self.sink.set_volume(vol as f32 / 100.0);
        Ok(())
    }

    fn is_finished(&mut self) -> BackendResult<bool> {
        Ok(self.sink.empty() && !self.sink.is_paused())
    }
}

impl Drop for RodioBackend {
    fn drop(&mut self) {
        self.sink.stop();
    }
}

pub struct MpvBackend {
    socket_path: PathBuf,
    child: Option<Child>,
    volume: u8,
}

impl MpvBackend {
    pub fn new(socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            child: None,
            volume: 80,
        }
    }

    fn is_running(&mut self) -> BackendResult<bool> {
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(_)) => {
                    self.child = None;
                    Ok(false)
                }
                Ok(None) => Ok(true),
                Err(e) => Err(AudioBackendError::MpvIpcError(format!(
                    "failed to inspect mpv process: {}",
                    e
                ))),
            }
        } else {
            Ok(false)
        }
    }

    fn spawn_with_url(&mut self, url: &str) -> BackendResult<()> {
        if !mpv_is_available() {
            return Err(AudioBackendError::MpvUnavailable);
        }

        if let Some(parent) = self.socket_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                AudioBackendError::MpvIpcError(format!(
                    "failed to create mpv socket directory {}: {}",
                    parent.display(),
                    e
                ))
            })?;
        }
        if self.socket_path.exists() {
            let _ = fs::remove_file(&self.socket_path);
        }

        let mut command = Command::new("mpv");
        command
            .arg("--no-video")
            .arg("--no-terminal")
            .arg(format!(
                "--input-ipc-server={}",
                self.socket_path.to_string_lossy()
            ))
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = command.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                AudioBackendError::MpvUnavailable
            } else {
                AudioBackendError::MpvIpcError(format!("failed to spawn mpv: {}", e))
            }
        })?;
        self.child = Some(child);

        for _ in 0..80 {
            if self.socket_path.exists() && UnixStream::connect(&self.socket_path).is_ok() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        Err(AudioBackendError::MpvIpcError(
            "timed out waiting for mpv IPC socket".to_string(),
        ))
    }

    fn send_command(&self, command: Value) -> BackendResult<Value> {
        let mut stream = UnixStream::connect(&self.socket_path).map_err(|e| {
            AudioBackendError::MpvIpcError(format!(
                "failed to connect to mpv socket {}: {}",
                self.socket_path.display(),
                e
            ))
        })?;
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

        let payload = serde_json::to_vec(&command).map_err(|e| {
            AudioBackendError::MpvIpcError(format!("failed to encode mpv command JSON: {}", e))
        })?;
        stream.write_all(&payload).map_err(|e| {
            AudioBackendError::MpvIpcError(format!("failed to write mpv command: {}", e))
        })?;
        stream.write_all(b"\n").map_err(|e| {
            AudioBackendError::MpvIpcError(format!("failed to write mpv command terminator: {}", e))
        })?;
        stream.flush().map_err(|e| {
            AudioBackendError::MpvIpcError(format!("failed to flush mpv command: {}", e))
        })?;

        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        reader.read_line(&mut line).map_err(|e| {
            AudioBackendError::MpvIpcError(format!("failed to read mpv response: {}", e))
        })?;
        if line.trim().is_empty() {
            return Err(AudioBackendError::MpvIpcError(
                "empty response from mpv".to_string(),
            ));
        }

        serde_json::from_str(&line).map_err(|e| {
            AudioBackendError::MpvIpcError(format!("failed to parse mpv response JSON: {}", e))
        })
    }

    fn response_error(response: &Value) -> Option<String> {
        response
            .get("error")
            .and_then(|v| v.as_str())
            .filter(|e| *e != "success")
            .map(|e| e.to_string())
    }
}

impl PlaybackBackend for MpvBackend {
    fn play(&mut self, source: &str) -> BackendResult<()> {
        if self.is_running()? {
            let response = self.send_command(json!({"command": ["loadfile", source]}))?;
            if let Some(error) = Self::response_error(&response) {
                return Err(AudioBackendError::MpvLoadFailed(error));
            }
        } else {
            self.spawn_with_url(source)?;
        }
        self.set_volume(self.volume)?;
        Ok(())
    }

    fn pause(&mut self) -> BackendResult<()> {
        if !self.is_running()? {
            return Ok(());
        }
        let response = self.send_command(json!({"command": ["set_property", "pause", true]}))?;
        if let Some(error) = Self::response_error(&response) {
            return Err(AudioBackendError::MpvIpcError(format!(
                "pause command failed: {}",
                error
            )));
        }
        Ok(())
    }

    fn resume(&mut self) -> BackendResult<()> {
        if !self.is_running()? {
            return Ok(());
        }
        let response = self.send_command(json!({"command": ["set_property", "pause", false]}))?;
        if let Some(error) = Self::response_error(&response) {
            return Err(AudioBackendError::MpvIpcError(format!(
                "resume command failed: {}",
                error
            )));
        }
        Ok(())
    }

    fn stop(&mut self) -> BackendResult<()> {
        let _ = self.send_command(json!({"command": ["quit"]}));

        if let Some(child) = &mut self.child {
            for _ in 0..20 {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.child = None;

        if self.socket_path.exists() {
            let _ = fs::remove_file(&self.socket_path);
        }

        Ok(())
    }

    fn seek(&mut self, position: Duration) -> BackendResult<bool> {
        if !self.is_running()? {
            return Ok(false);
        }
        let response = self.send_command(json!({
            "command": ["seek", position.as_secs(), "absolute"]
        }))?;
        if let Some(error) = Self::response_error(&response) {
            return Err(AudioBackendError::MpvIpcError(format!(
                "seek command failed: {}",
                error
            )));
        }
        Ok(true)
    }

    fn get_position(&mut self) -> BackendResult<Duration> {
        if !self.is_running()? {
            return Ok(Duration::from_secs(0));
        }
        let response = self.send_command(json!({"command": ["get_property", "playback-time"]}))?;
        if let Some(error) = Self::response_error(&response) {
            return Err(AudioBackendError::MpvIpcError(format!(
                "get playback-time failed: {}",
                error
            )));
        }

        let seconds = response.get("data").and_then(|v| v.as_f64()).unwrap_or(0.0);
        Ok(Duration::from_secs_f64(seconds.max(0.0)))
    }

    fn set_volume(&mut self, volume: u8) -> BackendResult<()> {
        self.volume = volume.min(100);
        if !self.is_running()? {
            return Ok(());
        }
        let response = self.send_command(json!({
            "command": ["set_property", "volume", self.volume]
        }))?;
        if let Some(error) = Self::response_error(&response) {
            return Err(AudioBackendError::MpvIpcError(format!(
                "set volume failed: {}",
                error
            )));
        }
        Ok(())
    }

    fn is_finished(&mut self) -> BackendResult<bool> {
        Ok(!self.is_running()?)
    }
}
