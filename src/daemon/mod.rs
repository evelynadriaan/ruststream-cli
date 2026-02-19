use anyhow::{Context, Result};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::{error, info, warn};

use crate::audio::AudioPlayer;
use crate::config::Config;
use crate::ipc::{
    DaemonCommand, DaemonErrorCode, DaemonRequestEnvelope, DaemonResponse, DaemonResponseEnvelope,
    PROTOCOL_VERSION,
};
use crate::models::{PlaybackState, RepeatMode, Track};

// Internal commands for the audio thread
#[derive(Clone)]
enum AudioCommand {
    Play(Track),
    Pause,
    Resume,
    Stop,
    SetVolume(u8),
    Seek(u64),
    CheckFinished(Sender<bool>),
    GetPosition(Sender<u64>),
}

pub struct Daemon {
    config: Config,
}

impl Daemon {
    pub fn new(config: Config) -> Result<Self> {
        Ok(Self { config })
    }

    pub fn run(&self) -> Result<()> {
        use interprocess::local_socket::prelude::*;
        use interprocess::local_socket::{GenericFilePath, ListenerOptions};

        let socket_path = self.config.socket_path();

        // Remove stale socket
        if socket_path.exists() {
            fs::remove_file(&socket_path)?;
        }

        // Write PID file
        let pid_path = self.config.pid_path();
        fs::write(&pid_path, std::process::id().to_string())?;

        // Create listener
        let name = socket_path.as_os_str().to_fs_name::<GenericFilePath>()?;
        let listener = ListenerOptions::new()
            .name(name)
            .create_sync()
            .with_context(|| "Failed to create socket listener")?;

        info!("Daemon started, listening on {}", socket_path.display());

        // Shared state
        let state = Arc::new(Mutex::new(PlaybackState::new()));
        state.lock().unwrap().volume = self.config.playback.default_volume;

        let running = Arc::new(AtomicBool::new(true));

        // Create channel for audio commands
        let (audio_tx, audio_rx): (Sender<AudioCommand>, Receiver<AudioCommand>) = mpsc::channel();

        // Spawn audio thread - AudioPlayer stays on this single thread
        let audio_running = Arc::clone(&running);
        let audio_state = Arc::clone(&state);
        let default_volume = self.config.playback.default_volume;
        thread::spawn(move || {
            run_audio_thread(audio_rx, audio_state, audio_running, default_volume);
        });

        // Spawn playback monitor thread
        let monitor_state = Arc::clone(&state);
        let monitor_running = Arc::clone(&running);
        let monitor_audio_tx = audio_tx.clone();
        thread::spawn(move || {
            playback_monitor(monitor_state, monitor_running, monitor_audio_tx);
        });

        // Initialize media controls (for system media keys)
        let media_controls = init_media_controls(Arc::clone(&state), audio_tx.clone());
        if media_controls.is_none() {
            warn!("Media controls not available - media keys won't work");
        }

        // Spawn media controls update thread
        if let Some(controls) = media_controls {
            let mc_state = Arc::clone(&state);
            let mc_running = Arc::clone(&running);
            thread::spawn(move || {
                update_media_controls_loop(controls, mc_state, mc_running);
            });
        }

        // Accept connections on main thread
        while running.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok(conn) => {
                    let response = handle_connection(conn, &state, &running, &audio_tx);

                    if let Err(e) = response {
                        error!("Connection error: {e}");
                    }
                }
                Err(e) => {
                    if running.load(Ordering::SeqCst) {
                        error!("Accept error: {e}");
                    }
                }
            }
        }

        // Cleanup
        let _ = fs::remove_file(&socket_path);
        let _ = fs::remove_file(&pid_path);

        info!("Daemon stopped");
        Ok(())
    }

    pub fn start_detached(config: &Config) -> Result<()> {
        use std::process::Command;

        let socket_path = config.socket_path();
        if socket_path.exists() {
            let client = crate::ipc::DaemonClient::new(&socket_path);
            if client.is_daemon_running() {
                anyhow::bail!("Daemon is already running");
            }
            fs::remove_file(&socket_path)?;
        }

        let exe = std::env::current_exe()?;

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;

            Command::new(&exe)
                .arg("daemon")
                .arg("run")
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .process_group(0)
                .spawn()
                .with_context(|| "Failed to start daemon")?;
        }

        let client = crate::ipc::DaemonClient::new(&socket_path);
        if client.wait_until_ready(std::time::Duration::from_secs(5)) {
            return Ok(());
        }

        anyhow::bail!("Daemon failed to start")
    }

    pub fn stop(config: &Config) -> Result<()> {
        let client = crate::ipc::DaemonClient::new(config.socket_path());
        if client.is_daemon_running() {
            client.shutdown()?;
            for _ in 0..50 {
                if !config.socket_path().exists() {
                    return Ok(());
                }
                thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        Ok(())
    }

    pub fn is_running(config: &Config) -> bool {
        let client = crate::ipc::DaemonClient::new(config.socket_path());
        client.is_daemon_running()
    }
}

fn init_media_controls(
    state: Arc<Mutex<PlaybackState>>,
    audio_tx: Sender<AudioCommand>,
) -> Option<souvlaki::MediaControls> {
    use souvlaki::{MediaControlEvent, MediaControls, PlatformConfig};

    #[cfg(target_os = "macos")]
    let hwnd = None;

    #[cfg(not(target_os = "macos"))]
    let hwnd = None;

    let config = PlatformConfig {
        dbus_name: "clistream",
        display_name: "clistream",
        hwnd,
    };

    let mut controls = MediaControls::new(config).ok()?;

    let state_clone = Arc::clone(&state);
    let tx = audio_tx.clone();

    controls
        .attach(move |event: MediaControlEvent| match event {
            MediaControlEvent::Play => {
                let _ = tx.send(AudioCommand::Resume);
            }
            MediaControlEvent::Pause => {
                let _ = tx.send(AudioCommand::Pause);
            }
            MediaControlEvent::Toggle => {
                let is_playing = state_clone.lock().unwrap().is_playing;
                if is_playing {
                    let _ = tx.send(AudioCommand::Pause);
                } else {
                    let _ = tx.send(AudioCommand::Resume);
                }
            }
            MediaControlEvent::Stop => {
                let _ = tx.send(AudioCommand::Stop);
            }
            _ => {}
        })
        .ok()?;

    Some(controls)
}

fn update_media_controls_loop(
    mut controls: souvlaki::MediaControls,
    state: Arc<Mutex<PlaybackState>>,
    running: Arc<AtomicBool>,
) {
    use souvlaki::{MediaMetadata, MediaPlayback};

    let mut last_track_id: Option<uuid::Uuid> = None;
    let mut last_playing: Option<bool> = None;

    while running.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_millis(500));

        let (current_track, is_playing) = {
            let s = state.lock().unwrap();
            (s.current_track.clone(), s.is_playing)
        };

        // Update playback state if changed
        if last_playing != Some(is_playing) {
            let playback = if is_playing {
                MediaPlayback::Playing { progress: None }
            } else if current_track.is_some() {
                MediaPlayback::Paused { progress: None }
            } else {
                MediaPlayback::Stopped
            };
            let _ = controls.set_playback(playback);
            last_playing = Some(is_playing);
        }

        // Update metadata if track changed
        if let Some(ref track) = current_track {
            if last_track_id != Some(track.id) {
                let _ = controls.set_metadata(MediaMetadata {
                    title: Some(&track.title),
                    artist: Some("clistream"),
                    album: None,
                    cover_url: None,
                    duration: Some(std::time::Duration::from_secs(track.duration)),
                });
                last_track_id = Some(track.id);
            }
        } else if last_track_id.is_some() {
            let _ = controls.set_metadata(MediaMetadata {
                title: None,
                artist: None,
                album: None,
                cover_url: None,
                duration: None,
            });
            last_track_id = None;
        }
    }
}

fn run_audio_thread(
    rx: Receiver<AudioCommand>,
    state: Arc<Mutex<PlaybackState>>,
    running: Arc<AtomicBool>,
    default_volume: u8,
) {
    let player = match AudioPlayer::new() {
        Ok(p) => {
            p.set_volume(default_volume);
            Some(p)
        }
        Err(e) => {
            error!("Failed to initialize audio player: {e}");
            None
        }
    };

    while running.load(Ordering::SeqCst) {
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(cmd) => {
                if let Some(ref p) = player {
                    match cmd {
                        AudioCommand::Play(track) => {
                            let path = std::path::Path::new(&track.file_path);
                            if let Err(e) = p.play_file(path) {
                                error!("Failed to play: {e}");
                                state.lock().unwrap().is_playing = false;
                            } else {
                                let mut s = state.lock().unwrap();
                                s.current_track = Some(track);
                                s.is_playing = true;
                                s.position = 0;
                            }
                        }
                        AudioCommand::Pause => {
                            p.pause();
                            state.lock().unwrap().is_playing = false;
                        }
                        AudioCommand::Resume => {
                            p.resume();
                            state.lock().unwrap().is_playing = true;
                        }
                        AudioCommand::Stop => {
                            p.stop();
                            let mut s = state.lock().unwrap();
                            s.is_playing = false;
                            s.current_track = None;
                            s.position = 0;
                        }
                        AudioCommand::SetVolume(vol) => {
                            p.set_volume(vol);
                            state.lock().unwrap().volume = vol;
                        }
                        AudioCommand::Seek(position) => {
                            let duration = std::time::Duration::from_secs(position);
                            if p.seek(duration) {
                                state.lock().unwrap().position = position;
                            }
                        }
                        AudioCommand::CheckFinished(response_tx) => {
                            let _ = response_tx.send(p.is_finished());
                        }
                        AudioCommand::GetPosition(response_tx) => {
                            let pos = p.get_position().as_secs();
                            let _ = response_tx.send(pos);
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn playback_monitor(
    state: Arc<Mutex<PlaybackState>>,
    running: Arc<AtomicBool>,
    audio_tx: Sender<AudioCommand>,
) {
    while running.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_secs(1));

        let should_check = {
            let s = state.lock().unwrap();
            s.is_playing && s.current_track.is_some()
        };

        if !should_check {
            continue;
        }

        // Get real position from audio player
        let (pos_tx, pos_rx) = mpsc::channel();
        if audio_tx.send(AudioCommand::GetPosition(pos_tx)).is_ok()
            && let Ok(pos) = pos_rx.recv_timeout(std::time::Duration::from_millis(100))
        {
            state.lock().unwrap().position = pos;
        }

        // Check if audio finished
        let (tx, rx) = mpsc::channel();
        let sent = audio_tx.send(AudioCommand::CheckFinished(tx)).is_ok();
        let finished = sent
            && rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .unwrap_or(false);

        if finished {
            let mut s = state.lock().unwrap();
            // Track finished, stop playback
            s.is_playing = false;
            s.current_track = None;
            s.position = 0;
        }
    }
}

fn handle_connection(
    conn: interprocess::local_socket::Stream,
    state: &Arc<Mutex<PlaybackState>>,
    running: &Arc<AtomicBool>,
    audio_tx: &Sender<AudioCommand>,
) -> Result<()> {
    let mut reader = BufReader::new(&conn);
    let mut writer = &conn;

    let mut line = String::new();
    reader.read_line(&mut line)?;

    let response = match serde_json::from_str::<DaemonRequestEnvelope>(&line) {
        Ok(request) => {
            if request.protocol_version != PROTOCOL_VERSION {
                error_response(
                    DaemonErrorCode::IpcProtocolMismatch,
                    format!(
                        "Protocol mismatch (client={}, daemon={})",
                        request.protocol_version, PROTOCOL_VERSION
                    ),
                )
            } else {
                handle_command(request.command, state, running, audio_tx)
            }
        }
        Err(e) => error_response(
            DaemonErrorCode::IpcProtocolMismatch,
            format!("Invalid daemon request payload: {e}"),
        ),
    };

    let response_json = serde_json::to_string(&response)?;
    writeln!(writer, "{response_json}")?;
    writer.flush()?;

    Ok(())
}

fn handle_command(
    command: DaemonCommand,
    state: &Arc<Mutex<PlaybackState>>,
    running: &Arc<AtomicBool>,
    audio_tx: &Sender<AudioCommand>,
) -> DaemonResponseEnvelope {
    match command {
        DaemonCommand::Play { track } => {
            if audio_tx.send(AudioCommand::Play(track)).is_ok() {
                ok_response(DaemonResponse::Ok)
            } else {
                error_response(
                    DaemonErrorCode::InternalError,
                    "Audio thread not running".to_string(),
                )
            }
        }
        DaemonCommand::PlayQueue {
            tracks,
            start_index,
        } => {
            if tracks.is_empty() {
                return error_response(
                    DaemonErrorCode::TrackNotFound,
                    "Queue is empty".to_string(),
                );
            }

            let idx = start_index.min(tracks.len() - 1);
            let track = tracks[idx].clone();

            {
                let mut s = state.lock().unwrap();
                s.queue = tracks;
                s.queue_index = idx;
            }

            if audio_tx.send(AudioCommand::Play(track)).is_ok() {
                ok_response(DaemonResponse::Ok)
            } else {
                error_response(
                    DaemonErrorCode::InternalError,
                    "Audio thread not running".to_string(),
                )
            }
        }
        DaemonCommand::Pause => {
            let _ = audio_tx.send(AudioCommand::Pause);
            ok_response(DaemonResponse::Ok)
        }
        DaemonCommand::Resume => {
            let _ = audio_tx.send(AudioCommand::Resume);
            ok_response(DaemonResponse::Ok)
        }
        DaemonCommand::Stop => {
            let _ = audio_tx.send(AudioCommand::Stop);
            ok_response(DaemonResponse::Ok)
        }
        DaemonCommand::Next => {
            let next_track = {
                let mut s = state.lock().unwrap();
                if s.queue.is_empty() {
                    return error_response(
                        DaemonErrorCode::TrackNotFound,
                        "Queue is empty".to_string(),
                    );
                }

                let next_idx = if s.shuffle {
                    use std::collections::hash_map::RandomState;
                    use std::hash::{BuildHasher, Hasher};
                    let random = RandomState::new().build_hasher().finish() as usize;
                    random % s.queue.len()
                } else {
                    (s.queue_index + 1) % s.queue.len()
                };

                if !s.shuffle && next_idx == 0 && s.repeat == RepeatMode::Off {
                    s.is_playing = false;
                    s.current_track = None;
                    return ok_response(DaemonResponse::Ok);
                }

                s.queue_index = next_idx;
                s.queue[next_idx].clone()
            };

            if audio_tx.send(AudioCommand::Play(next_track)).is_ok() {
                ok_response(DaemonResponse::Ok)
            } else {
                error_response(
                    DaemonErrorCode::InternalError,
                    "Audio thread not running".to_string(),
                )
            }
        }
        DaemonCommand::Previous => {
            let prev_track = {
                let mut s = state.lock().unwrap();
                if s.queue.is_empty() {
                    return error_response(
                        DaemonErrorCode::TrackNotFound,
                        "Queue is empty".to_string(),
                    );
                }

                let prev_idx = if s.queue_index == 0 {
                    s.queue.len() - 1
                } else {
                    s.queue_index - 1
                };

                s.queue_index = prev_idx;
                s.queue[prev_idx].clone()
            };

            if audio_tx.send(AudioCommand::Play(prev_track)).is_ok() {
                ok_response(DaemonResponse::Ok)
            } else {
                error_response(
                    DaemonErrorCode::InternalError,
                    "Audio thread not running".to_string(),
                )
            }
        }
        DaemonCommand::Seek { position } => {
            let _ = audio_tx.send(AudioCommand::Seek(position));
            ok_response(DaemonResponse::Ok)
        }
        DaemonCommand::SetVolume { volume } => {
            let _ = audio_tx.send(AudioCommand::SetVolume(volume));
            ok_response(DaemonResponse::Ok)
        }
        DaemonCommand::SetShuffle { enabled } => {
            state.lock().unwrap().shuffle = enabled;
            ok_response(DaemonResponse::Ok)
        }
        DaemonCommand::SetRepeat { mode } => {
            state.lock().unwrap().repeat = mode;
            ok_response(DaemonResponse::Ok)
        }
        DaemonCommand::QueueAdd { track } => {
            state.lock().unwrap().queue.push(track);
            ok_response(DaemonResponse::Ok)
        }
        DaemonCommand::QueueClear => {
            let mut s = state.lock().unwrap();
            s.queue.clear();
            s.queue_index = 0;
            ok_response(DaemonResponse::Ok)
        }
        DaemonCommand::GetStatus => {
            let s = state.lock().unwrap().clone();
            ok_response(DaemonResponse::Status(s))
        }
        DaemonCommand::Shutdown => {
            running.store(false, Ordering::SeqCst);
            let _ = audio_tx.send(AudioCommand::Stop);
            ok_response(DaemonResponse::Ok)
        }
    }
}

fn ok_response(response: DaemonResponse) -> DaemonResponseEnvelope {
    DaemonResponseEnvelope {
        protocol_version: PROTOCOL_VERSION,
        response,
        error_code: None,
    }
}

fn error_response(code: DaemonErrorCode, message: String) -> DaemonResponseEnvelope {
    DaemonResponseEnvelope {
        protocol_version: PROTOCOL_VERSION,
        response: DaemonResponse::Error {
            code: Some(code),
            message,
        },
        error_code: Some(code),
    }
}
