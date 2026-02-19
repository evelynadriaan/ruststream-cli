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
use crate::models::{LibraryExport, PlaybackState, Track};

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
        match client.play(track.clone())? {
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
