use anyhow::{Context, Result, bail};
use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use std::fs;
use std::path::Path;

use crate::config::Config;
use crate::daemon::Daemon;
use crate::db::Database;
use crate::download::{DownloadPhase, Downloader};
use crate::ipc::{DaemonClient, DaemonResponse};
use crate::models::{LibraryExport, PlaybackState, Playlist, RepeatMode, Track};

pub struct App {
    pub config: Config,
    pub db: Database,
}

impl App {
    pub fn new() -> Result<Self> {
        let config = Config::load()?;
        config.ensure_dirs()?;

        let db = Database::open(&config.db_path()).with_context(|| "Failed to open database")?;

        Ok(Self { config, db })
    }

    fn client(&self) -> DaemonClient {
        DaemonClient::new(self.config.socket_path())
    }

    fn ensure_daemon(&self) -> Result<DaemonClient> {
        let client = self.client();
        if !client.is_daemon_running() {
            if self.config.daemon.auto_start {
                println!("Starting daemon...");
                Daemon::start_detached(&self.config)?;
            } else {
                bail!("Daemon is not running. Start it with: clistream daemon start");
            }
        }
        Ok(client)
    }

    fn find_track(&self, query: &str) -> Result<Track> {
        let tracks = self.db.get_all_tracks()?;

        if tracks.is_empty() {
            bail!("Library is empty. Add tracks with: clistream add <url>");
        }

        // Try exact match first
        for track in &tracks {
            if track.alias.as_deref() == Some(query)
                || track.title.to_lowercase() == query.to_lowercase()
            {
                return Ok(track.clone());
            }
        }

        // Fuzzy search
        let matcher = SkimMatcherV2::default();
        let mut matches: Vec<_> = tracks
            .iter()
            .filter_map(|track| {
                let title_score = matcher.fuzzy_match(&track.title, query).unwrap_or(0);
                let alias_score = track
                    .alias
                    .as_ref()
                    .and_then(|a| matcher.fuzzy_match(a, query))
                    .unwrap_or(0);
                let score = title_score.max(alias_score);
                if score > 0 {
                    Some((track, score))
                } else {
                    None
                }
            })
            .collect();

        matches.sort_by_key(|entry| std::cmp::Reverse(entry.1));

        if let Some((track, _)) = matches.first() {
            Ok((*track).clone())
        } else {
            bail!("No track found matching '{query}'");
        }
    }

    // Command implementations

    pub fn add(&self, url: &str, alias: Option<&str>) -> Result<()> {
        println!("Checking dependencies...");
        Downloader::check_dependencies()?;

        let downloader = Downloader::new(self.config.clone());

        // Get canonical URL to check for duplicates
        println!("Checking video info...");
        let (title, canonical_url, _duration) = downloader.get_video_info(url)?;

        // Check if already in library by canonical URL
        if let Some(existing) = self.db.get_track_by_url(&canonical_url)? {
            println!("Track already in library: {}", existing.display_name());
            println!(
                "Use 'clistream remove \"{}\"' first if you want to re-add it.",
                title
            );
            return Ok(());
        }

        eprintln!("Downloading audio...");
        let mut track = downloader.download(url, |phase| match phase {
            DownloadPhase::Downloading {
                percent,
                speed,
                eta,
            } => {
                eprint!("\r  [{:5.1}%] {} ETA {}    ", percent, speed, eta);
            }
            DownloadPhase::Converting => {
                eprint!("\r  Converting audio...          \n");
            }
        })?;
        eprintln!();

        if let Some(a) = alias {
            track.alias = Some(a.to_string());
        }

        self.db.insert_track(&track)?;

        println!(
            "Added: {} ({})",
            track.display_name(),
            track.format_duration()
        );

        Ok(())
    }

    pub fn remove(&self, query: &str) -> Result<()> {
        let track = self.find_track(query)?;

        // Remove audio file
        let path = Path::new(&track.file_path);
        if path.exists() {
            fs::remove_file(path)?;
        }

        self.db.delete_track(&track.id)?;
        println!("Removed: {}", track.display_name());

        Ok(())
    }

    pub fn play(&self, query: &str) -> Result<()> {
        let track = self.find_track(query)?;

        if !track.available {
            bail!(
                "Track '{}' is marked as unavailable. Run 'clistream check' to verify.",
                track.display_name()
            );
        }

        let client = self.ensure_daemon()?;
        let queue = self.db.get_all_tracks()?;
        let start_index = queue.iter().position(|t| t.id == track.id).unwrap_or(0);

        match client.play_queue(queue, start_index)? {
            DaemonResponse::Ok => {
                println!(
                    "Playing: {} ({})",
                    track.display_name(),
                    track.format_duration()
                );
            }
            DaemonResponse::Error { message, .. } => bail!("{message}"),
            _ => {}
        }

        Ok(())
    }

    pub fn pause(&self) -> Result<()> {
        let client = self.ensure_daemon()?;
        client.pause()?;
        println!("Paused");
        Ok(())
    }

    pub fn resume(&self) -> Result<()> {
        let client = self.ensure_daemon()?;
        client.resume()?;
        println!("Resumed");
        Ok(())
    }

    pub fn stop(&self) -> Result<()> {
        let client = self.ensure_daemon()?;
        client.stop()?;
        println!("Stopped");
        Ok(())
    }

    pub fn seek(&self, position: &str) -> Result<()> {
        let seconds = parse_time(position)?;
        let client = self.ensure_daemon()?;
        client.seek(seconds)?;
        println!("Seeked to {}", format_duration(seconds));
        Ok(())
    }

    pub fn volume(&self, level: Option<u8>) -> Result<()> {
        let client = self.ensure_daemon()?;

        if let Some(vol) = level {
            let vol = vol.min(100);
            client.set_volume(vol)?;
            println!("Volume: {vol}%");
        } else {
            let status = client.get_status()?;
            println!("Volume: {}%", status.volume);
        }

        Ok(())
    }

    pub fn list(&self) -> Result<()> {
        let tracks = self.db.get_all_tracks()?;

        if tracks.is_empty() {
            println!("No tracks found.");
            return Ok(());
        }

        println!("{} tracks:\n", tracks.len());
        for (i, track) in tracks.iter().enumerate() {
            let status = if !track.available {
                " [unavailable]"
            } else {
                ""
            };
            let alias = track
                .alias
                .as_ref()
                .map(|a| format!(" ({a})"))
                .unwrap_or_default();
            println!(
                "{:3}. {}{} - {}{}",
                i + 1,
                track.title,
                alias,
                track.format_duration(),
                status
            );
        }

        Ok(())
    }

    pub fn search(&self, query: &str) -> Result<()> {
        let tracks = self.db.get_all_tracks()?;
        let matcher = SkimMatcherV2::default();

        let mut matches: Vec<_> = tracks
            .iter()
            .filter_map(|track| {
                let title_score = matcher.fuzzy_match(&track.title, query).unwrap_or(0);
                let alias_score = track
                    .alias
                    .as_ref()
                    .and_then(|a| matcher.fuzzy_match(a, query))
                    .unwrap_or(0);
                let score = title_score.max(alias_score);
                if score > 0 {
                    Some((track, score))
                } else {
                    None
                }
            })
            .collect();

        matches.sort_by_key(|entry| std::cmp::Reverse(entry.1));

        if matches.is_empty() {
            println!("No matches found for '{query}'");
            return Ok(());
        }

        println!("Search results for '{query}':\n");
        for (i, (track, _score)) in matches.iter().take(10).enumerate() {
            let alias = track
                .alias
                .as_ref()
                .map(|a| format!(" ({a})"))
                .unwrap_or_default();
            println!(
                "{:3}. {}{} - {}",
                i + 1,
                track.title,
                alias,
                track.format_duration()
            );
        }

        Ok(())
    }

    pub fn status(&self) -> Result<()> {
        let client = self.client();

        if !client.is_daemon_running() {
            println!("Daemon is not running.");
            return Ok(());
        }

        let status = client.get_status()?;
        print_status(&status);

        Ok(())
    }

    pub fn stream(&self, url: &str) -> Result<()> {
        let client = self.ensure_daemon()?;
        let response = if is_playlist_url(url) {
            client.stream_playlist(url.to_string())?
        } else {
            client.stream(url.to_string())?
        };

        match response {
            DaemonResponse::Ok => {
                if is_playlist_url(url) {
                    println!("Streaming playlist: {url}");
                } else {
                    println!("Streaming: {url}");
                }
            }
            DaemonResponse::Error { code, message } => {
                if let Some(code) = code {
                    let code = serde_json::to_string(&code)
                        .unwrap_or_else(|_| format!("{:?}", code))
                        .trim_matches('"')
                        .to_string();
                    bail!("{}: {}", code, message);
                }
                bail!("{message}");
            }
            _ => bail!("Unexpected daemon response"),
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        let client = self.ensure_daemon()?;
        match client.save_current_stream()? {
            DaemonResponse::Ok => {
                println!("Saving current stream in background.");
            }
            DaemonResponse::Error { code, message } => {
                if let Some(code) = code {
                    let code = serde_json::to_string(&code)
                        .unwrap_or_else(|_| format!("{:?}", code))
                        .trim_matches('"')
                        .to_string();
                    bail!("{}: {}", code, message);
                }
                bail!("{message}");
            }
            _ => bail!("Unexpected daemon response"),
        }
        Ok(())
    }

    pub fn next(&self) -> Result<()> {
        let client = self.ensure_daemon()?;
        match client.next()? {
            DaemonResponse::Ok => {
                println!("Skipped to next track.");
            }
            DaemonResponse::Error { message, .. } => bail!("{message}"),
            _ => bail!("Unexpected daemon response"),
        }
        Ok(())
    }

    pub fn prev(&self) -> Result<()> {
        let client = self.ensure_daemon()?;
        match client.previous()? {
            DaemonResponse::Ok => {
                println!("Returned to previous track.");
            }
            DaemonResponse::Error { message, .. } => bail!("{message}"),
            _ => bail!("Unexpected daemon response"),
        }
        Ok(())
    }

    pub fn queue_add(&self, query: &str) -> Result<()> {
        let track = self.find_track(query)?;
        let client = self.ensure_daemon()?;
        match client.queue_add(track.clone())? {
            DaemonResponse::Ok => {
                println!("Added to queue: {}", track.display_name());
            }
            DaemonResponse::Error { message, .. } => bail!("{message}"),
            _ => bail!("Unexpected daemon response"),
        }
        Ok(())
    }

    pub fn queue_list(&self) -> Result<()> {
        let client = self.ensure_daemon()?;
        let status = client.get_status()?;

        if status.queue.is_empty() {
            println!("Queue is empty.");
            return Ok(());
        }

        println!("Queue ({} tracks):", status.queue.len());
        for (i, track) in status.queue.iter().enumerate() {
            let marker = if i == status.queue_index { "*" } else { " " };
            println!(
                "{} {:3}. {} ({})",
                marker,
                i + 1,
                track.display_name(),
                track.format_duration()
            );
        }

        Ok(())
    }

    pub fn queue_clear(&self) -> Result<()> {
        let client = self.ensure_daemon()?;
        match client.queue_clear()? {
            DaemonResponse::Ok => {
                println!("Queue cleared.");
            }
            DaemonResponse::Error { message, .. } => bail!("{message}"),
            _ => bail!("Unexpected daemon response"),
        }
        Ok(())
    }

    pub fn shuffle(&self, mode: Option<&str>) -> Result<()> {
        let client = self.ensure_daemon()?;

        if let Some(mode) = mode {
            let enabled = parse_toggle_mode(mode)?;
            match client.set_shuffle(enabled)? {
                DaemonResponse::Ok => {
                    println!("Shuffle: {}", if enabled { "on" } else { "off" });
                }
                DaemonResponse::Error { message, .. } => bail!("{message}"),
                _ => bail!("Unexpected daemon response"),
            }
            return Ok(());
        }

        let status = client.get_status()?;
        println!("Shuffle: {}", if status.shuffle { "on" } else { "off" });
        Ok(())
    }

    pub fn repeat(&self, mode: Option<&str>) -> Result<()> {
        let client = self.ensure_daemon()?;

        if let Some(mode) = mode {
            let repeat_mode = parse_repeat_mode(mode)?;
            match client.set_repeat(repeat_mode)? {
                DaemonResponse::Ok => {
                    println!("Repeat: {}", repeat_mode);
                }
                DaemonResponse::Error { message, .. } => bail!("{message}"),
                _ => bail!("Unexpected daemon response"),
            }
            return Ok(());
        }

        let status = client.get_status()?;
        println!("Repeat: {}", status.repeat);
        Ok(())
    }

    pub fn playlist_create(&self, name: &str) -> Result<()> {
        if self.db.get_playlist_by_name(name)?.is_some() {
            bail!("Playlist already exists: {name}");
        }

        let playlist = Playlist::new(name.to_string());
        self.db.insert_playlist(&playlist)?;
        println!("Created playlist: {name}");
        Ok(())
    }

    pub fn playlist_add(&self, playlist_name: &str, query: &str) -> Result<()> {
        let playlist = self
            .db
            .get_playlist_by_name(playlist_name)?
            .with_context(|| format!("Playlist not found: {playlist_name}"))?;
        let track = self.find_track(query)?;

        self.db.add_track_to_playlist(&playlist.id, &track.id)?;
        println!(
            "Added '{}' to playlist '{}'.",
            track.display_name(),
            playlist.name
        );
        Ok(())
    }

    pub fn playlist_remove(&self, playlist_name: &str, query: &str) -> Result<()> {
        let playlist = self
            .db
            .get_playlist_by_name(playlist_name)?
            .with_context(|| format!("Playlist not found: {playlist_name}"))?;
        let track = self.find_track(query)?;

        self.db
            .remove_track_from_playlist(&playlist.id, &track.id)?;
        println!(
            "Removed '{}' from playlist '{}'.",
            track.display_name(),
            playlist.name
        );
        Ok(())
    }

    pub fn playlist_list(&self) -> Result<()> {
        let playlists = self.db.get_all_playlists()?;
        if playlists.is_empty() {
            println!("No playlists found.");
            return Ok(());
        }

        println!("Playlists ({}):", playlists.len());
        for playlist in playlists {
            let track_count = self.db.get_playlist_track_count(&playlist.id)?;
            println!("- {} ({} tracks)", playlist.name, track_count);
        }
        Ok(())
    }

    pub fn playlist_delete(&self, name: &str) -> Result<()> {
        let playlist = self
            .db
            .get_playlist_by_name(name)?
            .with_context(|| format!("Playlist not found: {name}"))?;
        self.db.delete_playlist(&playlist.id)?;
        println!("Deleted playlist: {}", playlist.name);
        Ok(())
    }

    pub fn daemon_start(&self) -> Result<()> {
        if Daemon::is_running(&self.config) {
            println!("Daemon is already running.");
            return Ok(());
        }

        Daemon::start_detached(&self.config)?;
        println!("Daemon started.");

        Ok(())
    }

    pub fn daemon_stop(&self) -> Result<()> {
        if !Daemon::is_running(&self.config) {
            println!("Daemon is not running.");
            return Ok(());
        }

        Daemon::stop(&self.config)?;
        println!("Daemon stopped.");

        Ok(())
    }

    pub fn daemon_status(&self) -> Result<()> {
        if Daemon::is_running(&self.config) {
            println!("Daemon is running.");
        } else {
            println!("Daemon is not running.");
        }

        Ok(())
    }

    pub fn daemon_run(&self) -> Result<()> {
        let daemon = Daemon::new(self.config.clone())?;
        daemon.run()
    }

    pub fn export(&self, file: Option<&str>) -> Result<()> {
        let tracks = self.db.get_all_tracks()?;
        let playlists = self.db.get_all_playlists()?;
        let playlist_tracks = self.db.get_all_playlist_tracks()?;

        let export = LibraryExport::new(tracks, playlists, playlist_tracks);
        let json = serde_json::to_string_pretty(&export)?;

        if let Some(path) = file {
            fs::write(path, &json)?;
            println!("Exported library to: {path}");
        } else {
            println!("{json}");
        }

        Ok(())
    }

    pub fn import(&self, file: &str) -> Result<()> {
        let content =
            fs::read_to_string(file).with_context(|| format!("Failed to read file: {file}"))?;

        let import: LibraryExport =
            serde_json::from_str(&content).with_context(|| "Failed to parse import file")?;

        println!("Importing {} tracks...", import.tracks.len());

        // Note: This only imports metadata, not audio files
        // Audio files would need to be re-downloaded
        let mut imported = 0;
        let mut skipped = 0;

        for track in import.tracks {
            if self.db.get_track_by_url(&track.url)?.is_some() {
                skipped += 1;
                continue;
            }

            // Mark as unavailable since we don't have the audio file
            let mut track = track;
            track.available = false;

            if self.db.insert_track(&track).is_ok() {
                imported += 1;
            }
        }

        for playlist in import.playlists {
            if self.db.get_playlist_by_name(&playlist.name)?.is_none() {
                let _ = self.db.insert_playlist(&playlist);
            }
        }

        println!("Imported: {imported}, Skipped (already exist): {skipped}");
        println!("Note: Audio files need to be re-downloaded for imported tracks.");

        Ok(())
    }

    pub fn check(&self) -> Result<()> {
        let tracks = self.db.get_all_tracks()?;

        if tracks.is_empty() {
            println!("Library is empty.");
            return Ok(());
        }

        println!("Checking {} tracks...", tracks.len());

        let downloader = Downloader::new(self.config.clone());
        let mut available = 0;
        let mut unavailable = 0;

        for track in &tracks {
            // Check if local file exists
            let file_exists = Path::new(&track.file_path).exists();

            // Check if URL is still available
            let url_available = downloader.check_availability(&track.url).unwrap_or(false);

            let is_available = file_exists && url_available;

            if is_available != track.available {
                self.db.update_track_availability(&track.id, is_available)?;
            }

            if is_available {
                available += 1;
            } else {
                unavailable += 1;
                let reason = if !file_exists {
                    "file missing"
                } else {
                    "URL unavailable"
                };
                println!("  [!] {} - {}", track.display_name(), reason);
            }
        }

        println!("\nAvailable: {available}, Unavailable: {unavailable}");

        Ok(())
    }

    pub fn history(&self, limit: usize) -> Result<()> {
        let events = self.db.get_listen_history(limit)?;
        if events.is_empty() {
            println!("No listening history yet.");
            return Ok(());
        }

        for event in events {
            println!(
                "{}  {}  {}  ({}s)",
                event.started_at, event.source, event.track_title, event.duration_played
            );
        }
        Ok(())
    }
}

fn parse_time(s: &str) -> Result<u64> {
    if s.contains(':') {
        let parts: Vec<&str> = s.split(':').collect();
        if parts.len() == 2 {
            let mins: u64 = parts[0].parse().context("Invalid minutes")?;
            let secs: u64 = parts[1].parse().context("Invalid seconds")?;
            return Ok(mins * 60 + secs);
        }
    }

    s.parse()
        .context("Invalid time format. Use seconds or MM:SS")
}

fn parse_toggle_mode(mode: &str) -> Result<bool> {
    match mode.to_lowercase().as_str() {
        "on" => Ok(true),
        "off" => Ok(false),
        _ => bail!("Invalid mode: {mode}. Expected 'on' or 'off'"),
    }
}

fn parse_repeat_mode(mode: &str) -> Result<RepeatMode> {
    mode.parse::<RepeatMode>()
        .map_err(|e| anyhow::anyhow!("{}. Expected 'off', 'one', or 'all'", e))
}

fn is_playlist_url(url: &str) -> bool {
    url.contains("list=") || url.ends_with(".m3u") || url.ends_with(".m3u8")
}

fn format_duration(seconds: u64) -> String {
    let mins = seconds / 60;
    let secs = seconds % 60;
    format!("{mins}:{secs:02}")
}

fn print_status(status: &PlaybackState) {
    if let Some(track) = &status.current_track {
        let state = if status.is_playing {
            "Playing"
        } else {
            "Paused"
        };
        println!("{}: {}", state, track.display_name());
        println!(
            "Duration: {} / {}",
            format_duration(status.position),
            track.format_duration()
        );
    } else {
        println!("Not playing");
    }

    println!("Volume: {}%", status.volume);
}
