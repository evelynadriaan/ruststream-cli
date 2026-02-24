use anyhow::{Context, Result};
use chrono::Utc;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use tracing::{debug, error, info, warn};

use crate::audio::{
    AudioBackendError, MpvBackend, PlaybackBackend, RodioBackend, mpv_is_available, mpv_socket_path,
};
use crate::config::Config;
use crate::db::Database;
use crate::discord::PresenceUpdate;
use crate::download::Downloader;
use crate::ipc::{
    DaemonCommand, DaemonErrorCode, DaemonRequestEnvelope, DaemonResponse, DaemonResponseEnvelope,
    PROTOCOL_VERSION,
};
use crate::models::{PlaybackState, RepeatMode, StreamEntry, Track};

// Internal commands for the audio thread
#[derive(Clone)]
enum AudioCommand {
    Play(Track),
    Stream {
        url: String,
        response_tx: Sender<std::result::Result<(), AudioBackendError>>,
    },
    Pause,
    Resume,
    Stop,
    SetVolume(u8),
    Seek(u64),
    CheckFinished(Sender<bool>),
    GetPosition(Sender<u64>),
}

#[derive(Debug, Clone, Default)]
struct StreamContext {
    current_stream_url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackendKind {
    Rodio,
    Mpv,
}

struct UnavailableBackend {
    reason: String,
}

impl PlaybackBackend for UnavailableBackend {
    fn play(&mut self, _source: &str) -> std::result::Result<(), AudioBackendError> {
        Err(AudioBackendError::AudioInit(self.reason.clone()))
    }

    fn pause(&mut self) -> std::result::Result<(), AudioBackendError> {
        Err(AudioBackendError::AudioInit(self.reason.clone()))
    }

    fn resume(&mut self) -> std::result::Result<(), AudioBackendError> {
        Err(AudioBackendError::AudioInit(self.reason.clone()))
    }

    fn stop(&mut self) -> std::result::Result<(), AudioBackendError> {
        Ok(())
    }

    fn seek(
        &mut self,
        _position: std::time::Duration,
    ) -> std::result::Result<bool, AudioBackendError> {
        Err(AudioBackendError::AudioInit(self.reason.clone()))
    }

    fn get_position(&mut self) -> std::result::Result<std::time::Duration, AudioBackendError> {
        Err(AudioBackendError::AudioInit(self.reason.clone()))
    }

    fn set_volume(&mut self, _volume: u8) -> std::result::Result<(), AudioBackendError> {
        Err(AudioBackendError::AudioInit(self.reason.clone()))
    }

    fn is_finished(&mut self) -> std::result::Result<bool, AudioBackendError> {
        Ok(true)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlaybackLifecycle {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone)]
enum PlaybackEvent {
    Start(Track),
    Pause,
    Resume,
    Stop,
    Finished,
    Error,
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
        let pid_path = self.config.pid_path();

        cleanup_stale_runtime_files(&self.config)?;

        // Write PID file
        fs::write(&pid_path, std::process::id().to_string())
            .with_context(|| format!("Failed to write pid file at {}", pid_path.display()))?;

        // Create listener
        let name = socket_path.as_os_str().to_fs_name::<GenericFilePath>()?;
        let listener = ListenerOptions::new()
            .name(name)
            .create_sync()
            .with_context(|| "Failed to create socket listener")?;

        info!(
            socket_path = %socket_path.display(),
            pid_path = %pid_path.display(),
            "Daemon started"
        );

        // Shared state
        let state = Arc::new(Mutex::new(PlaybackState::new()));
        state.lock().unwrap().volume = self.config.playback.default_volume;
        let stream_context = Arc::new(Mutex::new(StreamContext::default()));

        let running = Arc::new(AtomicBool::new(true));

        // Discord Rich Presence (optional)
        let discord_tx: Option<SyncSender<PresenceUpdate>> =
            if self.config.app.discord_rich_presence {
                let (tx, rx) = mpsc::sync_channel(4);
                thread::spawn(move || crate::discord::run(rx));
                Some(tx)
            } else {
                None
            };

        // Create channel for audio commands
        let (audio_tx, audio_rx): (Sender<AudioCommand>, Receiver<AudioCommand>) = mpsc::channel();

        // Spawn audio thread - AudioPlayer stays on this single thread
        let audio_running = Arc::clone(&running);
        let audio_state = Arc::clone(&state);
        let default_volume = self.config.playback.default_volume;
        let audio_config = self.config.clone();
        let audio_discord_tx = discord_tx.clone();
        thread::spawn(move || {
            run_audio_thread(
                audio_rx,
                audio_state,
                audio_running,
                default_volume,
                audio_config,
                audio_discord_tx,
            );
        });

        // Spawn playback monitor thread
        let monitor_state = Arc::clone(&state);
        let monitor_running = Arc::clone(&running);
        let monitor_audio_tx = audio_tx.clone();
        let monitor_stream_context = Arc::clone(&stream_context);
        let monitor_db_path = self.config.db_path();
        let monitor_discord_tx = discord_tx.clone();
        thread::spawn(move || {
            playback_monitor(
                monitor_state,
                monitor_stream_context,
                monitor_running,
                monitor_audio_tx,
                monitor_db_path,
                monitor_discord_tx,
            );
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
                    debug!("Accepted IPC connection");
                    let response = handle_connection(
                        conn,
                        &state,
                        &stream_context,
                        &running,
                        &audio_tx,
                        &self.config,
                        &discord_tx,
                    );

                    if let Err(e) = response {
                        error!(error = %e, "IPC connection handling failed");
                    }
                }
                Err(e) => {
                    if running.load(Ordering::SeqCst) {
                        error!(error = %e, "IPC accept failed");
                    }
                }
            }
        }

        send_presence_update(&discord_tx, PresenceUpdate::Stopped);
        cleanup_runtime_files(&self.config);

        info!("Daemon stopped");
        Ok(())
    }

    pub fn start_detached(config: &Config) -> Result<()> {
        use std::process::Command;

        let socket_path = config.socket_path();
        cleanup_stale_runtime_files(config)?;

        if socket_path.exists() {
            let client = crate::ipc::DaemonClient::new(&socket_path);
            if client.is_daemon_running() {
                anyhow::bail!("Daemon is already running");
            }
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
            info!(socket_path = %socket_path.display(), "Detached daemon is ready");
            return Ok(());
        }

        cleanup_stale_runtime_files(config)?;
        anyhow::bail!("Daemon failed to start")
    }

    pub fn stop(config: &Config) -> Result<()> {
        let client = crate::ipc::DaemonClient::new(config.socket_path());
        if client.is_daemon_running() {
            client.shutdown()?;
            for _ in 0..50 {
                if !client.is_daemon_running() {
                    cleanup_runtime_files(config);
                    return Ok(());
                }
                thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        cleanup_stale_runtime_files(config)?;
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
    config: Config,
    discord_tx: Option<SyncSender<PresenceUpdate>>,
) {
    let create_rodio_backend = |volume: u8| -> Box<dyn PlaybackBackend> {
        match RodioBackend::new() {
            Ok(mut backend) => {
                let _ = backend.set_volume(volume);
                Box::new(backend)
            }
            Err(e) => {
                error!(error = %e, "Failed to initialize rodio backend");
                Box::new(UnavailableBackend {
                    reason: e.to_string(),
                })
            }
        }
    };

    let create_mpv_backend = |volume: u8| -> Box<dyn PlaybackBackend> {
        let mut backend = MpvBackend::new(mpv_socket_path(config.data_dir()));
        let _ = backend.set_volume(volume);
        Box::new(backend)
    };

    let mut backend_kind = BackendKind::Rodio;
    let mut backend: Box<dyn PlaybackBackend> = create_rodio_backend(default_volume);
    let mut current_volume = default_volume;

    while running.load(Ordering::SeqCst) {
        match rx.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(cmd) => match cmd {
                AudioCommand::Play(track) => {
                    if backend_kind != BackendKind::Rodio {
                        let _ = backend.stop();
                        backend = create_rodio_backend(current_volume);
                        backend_kind = BackendKind::Rodio;
                    }

                    if let Err(e) = backend.play(&track.file_path) {
                        error!(error = %e, file_path = %track.file_path, "Failed to play track");
                        apply_playback_transition(
                            &mut state.lock().unwrap(),
                            PlaybackEvent::Error,
                            &discord_tx,
                        );
                    } else {
                        apply_playback_transition(
                            &mut state.lock().unwrap(),
                            PlaybackEvent::Start(track),
                            &discord_tx,
                        );
                    }
                }
                AudioCommand::Stream { url, response_tx } => {
                    if backend_kind != BackendKind::Mpv {
                        let _ = backend.stop();
                        backend = create_mpv_backend(current_volume);
                        backend_kind = BackendKind::Mpv;
                    }

                    let result = match backend.play(&url) {
                        Ok(_) => {
                            apply_playback_transition(
                                &mut state.lock().unwrap(),
                                PlaybackEvent::Start(stream_track(&url)),
                                &discord_tx,
                            );
                            Ok(())
                        }
                        Err(e) => {
                            error!(error = %e, url = %url, "Failed to stream URL");
                            apply_playback_transition(
                                &mut state.lock().unwrap(),
                                PlaybackEvent::Error,
                                &discord_tx,
                            );
                            Err(e)
                        }
                    };
                    let _ = response_tx.send(result);
                }
                AudioCommand::Pause => {
                    if let Err(e) = backend.pause() {
                        error!(error = %e, "Pause failed on active backend");
                    } else {
                        apply_playback_transition(
                            &mut state.lock().unwrap(),
                            PlaybackEvent::Pause,
                            &discord_tx,
                        );
                    }
                }
                AudioCommand::Resume => {
                    if let Err(e) = backend.resume() {
                        error!(error = %e, "Resume failed on active backend");
                    } else {
                        apply_playback_transition(
                            &mut state.lock().unwrap(),
                            PlaybackEvent::Resume,
                            &discord_tx,
                        );
                    }
                }
                AudioCommand::Stop => {
                    if let Err(e) = backend.stop() {
                        error!(error = %e, "Stop failed on active backend");
                    }
                    apply_playback_transition(
                        &mut state.lock().unwrap(),
                        PlaybackEvent::Stop,
                        &discord_tx,
                    );
                }
                AudioCommand::SetVolume(vol) => {
                    current_volume = vol;
                    if let Err(e) = backend.set_volume(vol) {
                        error!(error = %e, "Set volume failed on active backend");
                    }
                    state.lock().unwrap().volume = vol;
                }
                AudioCommand::Seek(position) => {
                    let duration = std::time::Duration::from_secs(position);
                    match backend.seek(duration) {
                        Ok(true) => {
                            state.lock().unwrap().position = position;
                        }
                        Ok(false) => {}
                        Err(e) => {
                            error!(error = %e, position, "Seek failed on active backend");
                        }
                    }
                }
                AudioCommand::CheckFinished(response_tx) => {
                    let finished = backend.is_finished().unwrap_or_else(|e| {
                        error!(error = %e, "is_finished failed on active backend");
                        false
                    });
                    let _ = response_tx.send(finished);
                }
                AudioCommand::GetPosition(response_tx) => {
                    let pos = backend.get_position().unwrap_or_else(|e| {
                        error!(error = %e, "get_position failed on active backend");
                        std::time::Duration::from_secs(0)
                    });
                    let _ = response_tx.send(pos.as_secs());
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn stream_track(url: &str) -> Track {
    let mut track = Track::new(url.to_string(), url.to_string(), 0, String::new());
    track.available = false;
    track
}

fn playback_monitor(
    state: Arc<Mutex<PlaybackState>>,
    stream_context: Arc<Mutex<StreamContext>>,
    running: Arc<AtomicBool>,
    audio_tx: Sender<AudioCommand>,
    db_path: std::path::PathBuf,
    discord_tx: Option<SyncSender<PresenceUpdate>>,
) {
    let db = match Database::open(&db_path) {
        Ok(db) => Some(db),
        Err(e) => {
            warn!(
                db_path = %db_path.display(),
                error = %e,
                "Listen history logging disabled: failed to open database"
            );
            None
        }
    };
    let mut active_listen: Option<ActiveListen> = None;

    while running.load(Ordering::SeqCst) {
        thread::sleep(std::time::Duration::from_secs(1));

        let pre_state = state.lock().unwrap().clone();
        if pre_state.current_track.is_none() {
            flush_active_listen_event(&db, &mut active_listen);
            continue;
        }
        update_active_listen(&pre_state, &mut active_listen);

        if !pre_state.is_playing {
            continue;
        }

        // Get real position from audio player
        let (pos_tx, pos_rx) = mpsc::channel();
        if audio_tx.send(AudioCommand::GetPosition(pos_tx)).is_ok()
            && let Ok(pos) = pos_rx.recv_timeout(std::time::Duration::from_millis(100))
        {
            state.lock().unwrap().position = pos;
        }
        let post_state = state.lock().unwrap().clone();
        update_active_listen(&post_state, &mut active_listen);

        // Check if audio finished
        let (tx, rx) = mpsc::channel();
        let sent = audio_tx.send(AudioCommand::CheckFinished(tx)).is_ok();
        let finished = sent
            && rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .unwrap_or(false);

        if finished {
            flush_active_listen_event(&db, &mut active_listen);

            enum FinishAction {
                PlayTrack(Track),
                StreamUrl(String),
                Stop,
            }

            let action = {
                let mut s = state.lock().unwrap();

                if s.is_streaming && !s.stream_queue.is_empty() {
                    let next_idx = (s.stream_queue_index + 1) % s.stream_queue.len();
                    s.stream_queue_index = next_idx;
                    FinishAction::StreamUrl(s.stream_queue[next_idx].url.clone())
                } else if s.queue.is_empty() {
                    FinishAction::Stop
                } else if s.repeat == RepeatMode::One {
                    let idx = s.queue_index.min(s.queue.len() - 1);
                    s.queue_index = idx;
                    FinishAction::PlayTrack(s.queue[idx].clone())
                } else if s.shuffle {
                    use std::collections::hash_map::RandomState;
                    use std::hash::{BuildHasher, Hasher};
                    let random = RandomState::new().build_hasher().finish() as usize;
                    let next_idx = random % s.queue.len();
                    s.queue_index = next_idx;
                    FinishAction::PlayTrack(s.queue[next_idx].clone())
                } else if s.queue_index + 1 >= s.queue.len() {
                    if s.repeat == RepeatMode::All {
                        s.queue_index = 0;
                        FinishAction::PlayTrack(s.queue[0].clone())
                    } else {
                        FinishAction::Stop
                    }
                } else {
                    s.queue_index += 1;
                    FinishAction::PlayTrack(s.queue[s.queue_index].clone())
                }
            };

            match action {
                FinishAction::PlayTrack(track) => {
                    if audio_tx.send(AudioCommand::Play(track)).is_err() {
                        apply_playback_transition(
                            &mut state.lock().unwrap(),
                            PlaybackEvent::Error,
                            &discord_tx,
                        );
                    }
                }
                FinishAction::StreamUrl(url) => {
                    let (response_tx, response_rx) = mpsc::channel();
                    if audio_tx
                        .send(AudioCommand::Stream {
                            url: url.clone(),
                            response_tx,
                        })
                        .is_err()
                    {
                        apply_playback_transition(
                            &mut state.lock().unwrap(),
                            PlaybackEvent::Error,
                            &discord_tx,
                        );
                        continue;
                    }

                    match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
                        Ok(Ok(())) => {
                            stream_context.lock().unwrap().current_stream_url = Some(url);
                        }
                        Ok(Err(e)) => {
                            error!(error = %e, "Failed to auto-advance streaming queue");
                            apply_playback_transition(
                                &mut state.lock().unwrap(),
                                PlaybackEvent::Error,
                                &discord_tx,
                            );
                        }
                        Err(e) => {
                            error!(error = %e, "Timed out auto-advancing streaming queue");
                            apply_playback_transition(
                                &mut state.lock().unwrap(),
                                PlaybackEvent::Error,
                                &discord_tx,
                            );
                        }
                    }
                }
                FinishAction::Stop => {
                    apply_playback_transition(
                        &mut state.lock().unwrap(),
                        PlaybackEvent::Finished,
                        &discord_tx,
                    );
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ActiveListen {
    track_title: String,
    source: String,
    started_at: String,
    duration_played: u64,
}

fn update_active_listen(state: &PlaybackState, active: &mut Option<ActiveListen>) {
    let Some(current_track) = state.current_track.as_ref() else {
        return;
    };

    let track_title = current_track.display_name().to_string();
    let source = if state.is_streaming {
        "stream".to_string()
    } else {
        "local".to_string()
    };

    match active {
        Some(entry) if entry.track_title == track_title && entry.source == source => {
            entry.duration_played = state.position;
        }
        _ => {
            *active = Some(ActiveListen {
                track_title,
                source,
                started_at: Utc::now().to_rfc3339(),
                duration_played: state.position,
            });
        }
    }
}

fn flush_active_listen_event(db: &Option<Database>, active: &mut Option<ActiveListen>) {
    let Some(event) = active.take() else {
        return;
    };

    let Some(db) = db else {
        return;
    };

    if let Err(e) = db.insert_listen_event(
        &event.track_title,
        &event.source,
        &event.started_at,
        event.duration_played,
    ) {
        warn!(error = %e, "Failed to persist listen history event");
    }
}

fn handle_connection(
    conn: interprocess::local_socket::Stream,
    state: &Arc<Mutex<PlaybackState>>,
    stream_context: &Arc<Mutex<StreamContext>>,
    running: &Arc<AtomicBool>,
    audio_tx: &Sender<AudioCommand>,
    config: &Config,
    discord_tx: &Option<SyncSender<PresenceUpdate>>,
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
                handle_command(
                    request.command,
                    state,
                    stream_context,
                    running,
                    audio_tx,
                    config,
                    discord_tx,
                )
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
    stream_context: &Arc<Mutex<StreamContext>>,
    running: &Arc<AtomicBool>,
    audio_tx: &Sender<AudioCommand>,
    config: &Config,
    discord_tx: &Option<SyncSender<PresenceUpdate>>,
) -> DaemonResponseEnvelope {
    match command {
        DaemonCommand::Play { track } => {
            {
                let mut s = state.lock().unwrap();
                s.stream_queue.clear();
                s.stream_queue_index = 0;
                s.is_streaming = false;
            }
            stream_context.lock().unwrap().current_stream_url = None;
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
                s.stream_queue.clear();
                s.stream_queue_index = 0;
                s.is_streaming = false;
            }
            stream_context.lock().unwrap().current_stream_url = None;

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
            {
                let mut s = state.lock().unwrap();
                s.stream_queue.clear();
                s.stream_queue_index = 0;
                s.is_streaming = false;
            }
            stream_context.lock().unwrap().current_stream_url = None;
            let _ = audio_tx.send(AudioCommand::Stop);
            ok_response(DaemonResponse::Ok)
        }
        DaemonCommand::Next => {
            let stream_url = {
                let mut s = state.lock().unwrap();
                if s.is_streaming && !s.stream_queue.is_empty() {
                    s.stream_queue_index = (s.stream_queue_index + 1) % s.stream_queue.len();
                    Some(s.stream_queue[s.stream_queue_index].url.clone())
                } else {
                    None
                }
            };

            if let Some(url) = stream_url {
                if let Some(response) = send_stream_command(audio_tx, url.clone()) {
                    return response;
                }
                stream_context.lock().unwrap().current_stream_url = Some(url);
                return ok_response(DaemonResponse::Ok);
            }

            let next_track = {
                let mut s = state.lock().unwrap();
                if s.queue.is_empty() {
                    return error_response(
                        DaemonErrorCode::TrackNotFound,
                        "Queue is empty".to_string(),
                    );
                }

                if s.repeat == RepeatMode::One {
                    let idx = s.queue_index.min(s.queue.len() - 1);
                    s.queue_index = idx;
                    s.queue[idx].clone()
                } else {
                    let next_idx = if s.shuffle {
                        use std::collections::hash_map::RandomState;
                        use std::hash::{BuildHasher, Hasher};
                        let random = RandomState::new().build_hasher().finish() as usize;
                        random % s.queue.len()
                    } else {
                        (s.queue_index + 1) % s.queue.len()
                    };

                    if !s.shuffle && next_idx == 0 && s.repeat == RepeatMode::Off {
                        apply_playback_transition(&mut s, PlaybackEvent::Stop, discord_tx);
                        return ok_response(DaemonResponse::Ok);
                    }

                    s.queue_index = next_idx;
                    s.queue[next_idx].clone()
                }
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
            let stream_url = {
                let mut s = state.lock().unwrap();
                if s.is_streaming && !s.stream_queue.is_empty() {
                    s.stream_queue_index = if s.stream_queue_index == 0 {
                        s.stream_queue.len() - 1
                    } else {
                        s.stream_queue_index - 1
                    };
                    Some(s.stream_queue[s.stream_queue_index].url.clone())
                } else {
                    None
                }
            };

            if let Some(url) = stream_url {
                if let Some(response) = send_stream_command(audio_tx, url.clone()) {
                    return response;
                }
                stream_context.lock().unwrap().current_stream_url = Some(url);
                return ok_response(DaemonResponse::Ok);
            }

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
        DaemonCommand::Stream { url } => {
            handle_stream_request(url, false, state, stream_context, audio_tx)
        }
        DaemonCommand::StreamPlaylist { url } => {
            handle_stream_request(url, true, state, stream_context, audio_tx)
        }
        DaemonCommand::StreamQueueLoad { entries } => {
            handle_stream_queue_load(entries, state, stream_context, audio_tx)
        }
        DaemonCommand::SaveCurrentStream => {
            if !mpv_is_available() {
                return error_response(
                    DaemonErrorCode::MpvUnavailable,
                    "mpv is not installed. Install mpv to use stream/save commands.".to_string(),
                );
            }

            let Some(stream_url) = stream_context.lock().unwrap().current_stream_url.clone() else {
                return error_response(
                    DaemonErrorCode::TrackNotFound,
                    "No active stream to save".to_string(),
                );
            };

            let config_clone = config.clone();
            thread::spawn(move || {
                save_stream_to_library(config_clone, stream_url);
            });

            ok_response(DaemonResponse::Ok)
        }
        DaemonCommand::GetStatus => {
            let s = state.lock().unwrap().clone();
            ok_response(DaemonResponse::Status(s))
        }
        DaemonCommand::Shutdown => {
            stream_context.lock().unwrap().current_stream_url = None;
            running.store(false, Ordering::SeqCst);
            let _ = audio_tx.send(AudioCommand::Stop);
            ok_response(DaemonResponse::Ok)
        }
    }
}

fn handle_stream_request(
    url: String,
    is_playlist: bool,
    state: &Arc<Mutex<PlaybackState>>,
    stream_context: &Arc<Mutex<StreamContext>>,
    audio_tx: &Sender<AudioCommand>,
) -> DaemonResponseEnvelope {
    if let Some(response) = send_stream_command(audio_tx, url.clone()) {
        return response;
    }

    {
        let mut s = state.lock().unwrap();
        s.queue.clear();
        s.queue_index = 0;
        s.stream_queue.clear();
        s.stream_queue_index = 0;
        s.is_streaming = true;
    }
    stream_context.lock().unwrap().current_stream_url = Some(url.clone());
    if is_playlist {
        info!(url = %url, "Streaming playlist via mpv");
    } else {
        info!(url = %url, "Streaming URL via mpv");
    }
    ok_response(DaemonResponse::Ok)
}

fn handle_stream_queue_load(
    entries: Vec<StreamEntry>,
    state: &Arc<Mutex<PlaybackState>>,
    stream_context: &Arc<Mutex<StreamContext>>,
    audio_tx: &Sender<AudioCommand>,
) -> DaemonResponseEnvelope {
    if !mpv_is_available() {
        return error_response(
            DaemonErrorCode::MpvUnavailable,
            "mpv is not installed. Install mpv to use stream/save commands.".to_string(),
        );
    }

    if entries.is_empty() {
        return error_response(
            DaemonErrorCode::TrackNotFound,
            "Stream queue is empty".to_string(),
        );
    }

    let first_url = entries[0].url.clone();

    {
        let mut s = state.lock().unwrap();
        s.queue.clear();
        s.queue_index = 0;
        s.stream_queue = entries;
        s.stream_queue_index = 0;
        s.is_streaming = true;
    }

    if let Some(response) = send_stream_command(audio_tx, first_url.clone()) {
        return response;
    }

    stream_context.lock().unwrap().current_stream_url = Some(first_url);
    ok_response(DaemonResponse::Ok)
}

fn send_stream_command(
    audio_tx: &Sender<AudioCommand>,
    url: String,
) -> Option<DaemonResponseEnvelope> {
    let (response_tx, response_rx) = mpsc::channel();
    if audio_tx
        .send(AudioCommand::Stream {
            url: url.clone(),
            response_tx,
        })
        .is_err()
    {
        return Some(error_response(
            DaemonErrorCode::InternalError,
            "Audio thread not running".to_string(),
        ));
    }

    match response_rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(Ok(())) => None,
        Ok(Err(err)) => {
            let (code, message) = map_audio_error_to_daemon(err);
            Some(error_response(code, message))
        }
        Err(_) => Some(error_response(
            DaemonErrorCode::InternalError,
            "Timed out waiting for audio thread response".to_string(),
        )),
    }
}

fn map_audio_error_to_daemon(err: AudioBackendError) -> (DaemonErrorCode, String) {
    match err {
        AudioBackendError::MpvUnavailable => (
            DaemonErrorCode::MpvUnavailable,
            "mpv is not installed. Install mpv to use stream/save commands.".to_string(),
        ),
        AudioBackendError::MpvIpcError(message) => (DaemonErrorCode::MpvIpcError, message),
        AudioBackendError::MpvLoadFailed(message) => (DaemonErrorCode::MpvLoadFailed, message),
        AudioBackendError::AudioPlay(message) => (DaemonErrorCode::AudioPlayFailed, message),
        AudioBackendError::AudioInit(message) => (DaemonErrorCode::AudioInitFailed, message),
    }
}

fn save_stream_to_library(config: Config, stream_url: String) {
    if let Err(e) = Downloader::check_dependencies() {
        warn!(error = %e, "Skipping stream save due to missing downloader dependency");
        return;
    }

    let downloader = Downloader::new(config.clone());
    match downloader.download(&stream_url, |_| {}) {
        Ok(track) => match Database::open(&config.db_path()) {
            Ok(db) => {
                let already_exists = db.get_track_by_url(&track.url).ok().flatten().is_some();
                if !already_exists {
                    if let Err(e) = db.insert_track(&track) {
                        warn!(error = %e, "Failed to persist saved stream track");
                    } else {
                        info!(track = %track.title, "Saved stream track to library");
                        // TODO(streaming-v2): switch to local rodio playback after save completes, with position handoff
                    }
                } else {
                    info!(track = %track.title, "Saved stream track already exists in library");
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to open database for stream save");
            }
        },
        Err(e) => {
            warn!(error = %e, url = %stream_url, "Failed to download current stream");
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
    debug!(?code, message = %message, "Returning daemon error response");
    DaemonResponseEnvelope {
        protocol_version: PROTOCOL_VERSION,
        response: DaemonResponse::Error {
            code: Some(code),
            message,
        },
        error_code: Some(code),
    }
}

fn derive_playback_lifecycle(state: &PlaybackState) -> PlaybackLifecycle {
    match (state.current_track.is_some(), state.is_playing) {
        (false, _) => PlaybackLifecycle::Stopped,
        (true, true) => PlaybackLifecycle::Playing,
        (true, false) => PlaybackLifecycle::Paused,
    }
}

fn send_presence_update(discord_tx: &Option<SyncSender<PresenceUpdate>>, update: PresenceUpdate) {
    let Some(tx) = discord_tx else {
        return;
    };

    if let Err(err) = tx.try_send(update) {
        debug!(?err, "Dropped Discord presence update");
    }
}

fn apply_playback_transition(
    state: &mut PlaybackState,
    event: PlaybackEvent,
    discord_tx: &Option<SyncSender<PresenceUpdate>>,
) {
    let current = derive_playback_lifecycle(state);
    let mut presence_update: Option<PresenceUpdate> = None;

    match (current, event) {
        (_, PlaybackEvent::Stop | PlaybackEvent::Finished | PlaybackEvent::Error) => {
            state.is_playing = false;
            state.current_track = None;
            state.position = 0;
            presence_update = Some(PresenceUpdate::Stopped);
        }
        (_, PlaybackEvent::Start(track)) => {
            let title = track.display_name().to_string();
            state.current_track = Some(track);
            state.is_playing = true;
            state.position = 0;
            presence_update = Some(PresenceUpdate::Playing {
                title,
                artist: None,
            });
        }
        (PlaybackLifecycle::Playing, PlaybackEvent::Pause) => {
            state.is_playing = false;
            if let Some(track) = state.current_track.as_ref() {
                presence_update = Some(PresenceUpdate::Paused {
                    title: track.display_name().to_string(),
                });
            }
        }
        (PlaybackLifecycle::Paused, PlaybackEvent::Resume) => {
            state.is_playing = true;
            if let Some(track) = state.current_track.as_ref() {
                presence_update = Some(PresenceUpdate::Playing {
                    title: track.display_name().to_string(),
                    artist: None,
                });
            }
        }
        (PlaybackLifecycle::Stopped, PlaybackEvent::Pause | PlaybackEvent::Resume) => {
            warn!(lifecycle = ?current, "Ignoring invalid playback transition on stopped state");
        }
        (PlaybackLifecycle::Playing, PlaybackEvent::Resume) => {
            debug!("Playback already in Playing state");
        }
        (PlaybackLifecycle::Paused, PlaybackEvent::Pause) => {
            debug!("Playback already in Paused state");
        }
    }

    if let Some(update) = presence_update {
        send_presence_update(discord_tx, update);
    }
}

fn cleanup_runtime_files(config: &Config) {
    let socket_path = config.socket_path();
    let pid_path = config.pid_path();

    if socket_path.exists()
        && let Err(e) = fs::remove_file(&socket_path)
    {
        warn!(
            socket_path = %socket_path.display(),
            error = %e,
            "Failed to remove socket file during cleanup"
        );
    }
    if pid_path.exists()
        && let Err(e) = fs::remove_file(&pid_path)
    {
        warn!(
            pid_path = %pid_path.display(),
            error = %e,
            "Failed to remove pid file during cleanup"
        );
    }
}

fn cleanup_stale_runtime_files(config: &Config) -> Result<()> {
    let socket_path = config.socket_path();
    let pid_path = config.pid_path();

    if socket_path.exists() {
        let client = crate::ipc::DaemonClient::new(&socket_path);
        if !client.is_daemon_running() {
            warn!(
                socket_path = %socket_path.display(),
                "Removing stale daemon socket"
            );
            fs::remove_file(&socket_path).with_context(|| {
                format!(
                    "Failed to remove stale socket file at {}",
                    socket_path.display()
                )
            })?;
        }
    }

    if pid_path.exists() {
        let stale_pid = fs::read_to_string(&pid_path)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
            .map(|pid| !is_pid_alive(pid))
            .unwrap_or(true);

        if stale_pid {
            warn!(pid_path = %pid_path.display(), "Removing stale daemon pid file");
            fs::remove_file(&pid_path).with_context(|| {
                format!("Failed to remove stale pid file at {}", pid_path.display())
            })?;
        }
    }

    Ok(())
}

fn is_pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}
