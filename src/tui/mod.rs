use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph},
};
use std::io;
use std::sync::mpsc;
use std::time::Duration;

use crate::config::Config;
use crate::db::Database;
use crate::download::{DownloadPhase, Downloader};
use crate::ipc::DaemonClient;
use crate::models::{PlaybackState, Track};

enum DownloadUpdate {
    Status(String),
    Progress(DownloadPhase),
    Done(Result<Track, String>),
}

pub struct Tui {
    config: Config,
    db: Database,
    client: DaemonClient,
    tracks: Vec<Track>,
    library_state: ListState,
    playback_state: PlaybackState,
    search_query: String,
    search_mode: bool,
    edit_mode: bool,
    edit_text: String,
    add_mode: bool,
    add_url: String,
    status_message: Option<String>,
    download_rx: Option<mpsc::Receiver<DownloadUpdate>>,
}

impl Tui {
    pub fn new(config: Config, db: Database) -> Result<Self> {
        let client = DaemonClient::new(config.socket_path());
        let tracks = db.get_all_tracks()?;

        let playback_state = if client.is_daemon_running() {
            client.get_status().unwrap_or_default()
        } else {
            PlaybackState::default()
        };

        let mut library_state = ListState::default();
        if !tracks.is_empty() {
            library_state.select(Some(0));
        }

        Ok(Self {
            config,
            db,
            client,
            tracks,
            library_state,
            playback_state,
            search_query: String::new(),
            search_mode: false,
            edit_mode: false,
            edit_text: String::new(),
            add_mode: false,
            add_url: String::new(),
            status_message: None,
            download_rx: None,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let result = self.main_loop(&mut terminal);

        disable_raw_mode()?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        terminal.show_cursor()?;

        result
    }

    #[allow(clippy::collapsible_if)]
    fn main_loop(&mut self, terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
        loop {
            // Refresh playback state
            if self.client.is_daemon_running() {
                if let Ok(state) = self.client.get_status() {
                    self.playback_state = state;
                }
            }

            // Poll for download progress updates
            if let Some(rx) = &self.download_rx {
                match rx.try_recv() {
                    Ok(DownloadUpdate::Status(msg)) => {
                        self.status_message = Some(msg);
                    }
                    Ok(DownloadUpdate::Progress(phase)) => match phase {
                        DownloadPhase::Downloading {
                            percent,
                            speed,
                            eta,
                        } => {
                            self.status_message = Some(format!(
                                "Downloading: {:.1}% ({}, ETA {})",
                                percent, speed, eta
                            ));
                        }
                        DownloadPhase::Converting => {
                            self.status_message = Some("Converting audio...".to_string());
                        }
                    },
                    Ok(DownloadUpdate::Done(result)) => {
                        match result {
                            Ok(track) => {
                                if self.db.insert_track(&track).is_ok() {
                                    self.status_message =
                                        Some(format!("Added: {}", track.display_name()));
                                    if let Ok(tracks) = self.db.get_all_tracks() {
                                        self.tracks = tracks;
                                        self.library_state.select(Some(0));
                                    }
                                } else {
                                    self.status_message = Some("Failed to save track".to_string());
                                }
                            }
                            Err(e) => {
                                self.status_message = Some(format!("Download failed: {}", e));
                            }
                        }
                        self.download_rx = None;
                    }
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.status_message =
                            Some("Download thread terminated unexpectedly".to_string());
                        self.download_rx = None;
                    }
                }
            }

            terminal.draw(|f| self.ui(f))?;

            if event::poll(Duration::from_millis(250))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    if self.search_mode {
                        match key.code {
                            KeyCode::Esc => {
                                self.search_mode = false;
                                self.search_query.clear();
                            }
                            KeyCode::Enter => {
                                self.search_mode = false;
                                self.apply_search();
                            }
                            KeyCode::Backspace => {
                                self.search_query.pop();
                            }
                            KeyCode::Char(c) => {
                                self.search_query.push(c);
                            }
                            _ => {}
                        }
                    } else if self.edit_mode {
                        match key.code {
                            KeyCode::Esc => {
                                self.edit_mode = false;
                                self.edit_text.clear();
                            }
                            KeyCode::Enter => {
                                self.edit_mode = false;
                                self.apply_edit();
                            }
                            KeyCode::Backspace => {
                                self.edit_text.pop();
                            }
                            KeyCode::Char(c) => {
                                self.edit_text.push(c);
                            }
                            _ => {}
                        }
                    } else if self.add_mode {
                        match key.code {
                            KeyCode::Esc => {
                                self.add_mode = false;
                                self.add_url.clear();
                            }
                            KeyCode::Enter => {
                                self.add_mode = false;
                                self.add_track();
                            }
                            KeyCode::Backspace => {
                                self.add_url.pop();
                            }
                            KeyCode::Char(c) => {
                                self.add_url.push(c);
                            }
                            _ => {}
                        }
                    } else {
                        // Clear status message on any key press
                        self.status_message = None;
                        match key.code {
                            KeyCode::Char('q') => return Ok(()),
                            KeyCode::Char('/') => {
                                self.search_mode = true;
                            }
                            KeyCode::Char('e') => self.start_edit(),
                            KeyCode::Char('a') if self.download_rx.is_none() => {
                                self.add_mode = true;
                            }
                            KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
                            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
                            KeyCode::Left | KeyCode::Char('h') => self.seek_backward(),
                            KeyCode::Right | KeyCode::Char('l') => self.seek_forward(),
                            KeyCode::Enter => self.play_selected(),
                            KeyCode::Char(' ') => self.toggle_or_play(),
                            KeyCode::Char('+') | KeyCode::Char('=') => self.volume_up(),
                            KeyCode::Char('-') => self.volume_down(),
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    fn ui(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(7), // Now playing (larger)
                Constraint::Min(8),    // Main content
                Constraint::Length(1), // Help
            ])
            .split(f.area());

        self.render_now_playing(f, chunks[0]);
        self.render_main_content(f, chunks[1]);
        self.render_help(f, chunks[2]);
    }

    fn format_time(seconds: u64) -> String {
        let mins = seconds / 60;
        let secs = seconds % 60;
        format!("{:02}:{:02}", mins, secs)
    }

    fn render_now_playing(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));

        let inner = block.inner(area);
        f.render_widget(block, area);

        if let Some(track) = &self.playback_state.current_track {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([
                    Constraint::Length(1), // Track title
                    Constraint::Length(1), // Spacer
                    Constraint::Length(1), // Progress bar
                    Constraint::Length(1), // Time + controls
                ])
                .split(inner);

            // Track title (centered, bold)
            let status_icon = if self.playback_state.is_playing {
                "▶ "
            } else {
                "⏸ "
            };
            let title = Paragraph::new(Line::from(vec![
                Span::styled(status_icon, Style::default().fg(Color::Cyan)),
                Span::styled(
                    track.display_name(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]))
            .alignment(Alignment::Center);
            f.render_widget(title, chunks[0]);

            // Progress bar
            let progress = if track.duration > 0 {
                (self.playback_state.position as f64 / track.duration as f64).min(1.0)
            } else {
                0.0
            };

            let gauge = Gauge::default()
                .ratio(progress)
                .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
                .label("");
            f.render_widget(gauge, chunks[2]);

            // Time display and controls
            let current_time = Self::format_time(self.playback_state.position);
            let total_time = Self::format_time(track.duration);

            let time_line = Line::from(vec![
                Span::raw(format!("{}  ", current_time)),
                Span::styled(
                    if self.playback_state.is_playing {
                        "⏸"
                    } else {
                        "▶"
                    },
                    Style::default().fg(Color::Cyan),
                ),
                Span::raw(format!("  {}", total_time)),
                Span::raw(format!("    Vol: {}%", self.playback_state.volume)),
            ]);

            let time_para = Paragraph::new(time_line).alignment(Alignment::Center);
            f.render_widget(time_para, chunks[3]);
        } else {
            // Nothing playing
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(1)
                .constraints([Constraint::Min(1)])
                .split(inner);

            let text = Paragraph::new("No track playing")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(text, chunks[0]);
        }
    }

    fn render_main_content(&self, f: &mut Frame, area: Rect) {
        // Library only
        let library_block = Block::default()
            .title(format!(" Library ({}) ", self.tracks.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Cyan));

        let items: Vec<ListItem> = self
            .tracks
            .iter()
            .map(|t| {
                let is_current = self
                    .playback_state
                    .current_track
                    .as_ref()
                    .map(|ct| ct.id == t.id)
                    .unwrap_or(false);

                let style = if !t.available {
                    Style::default().fg(Color::DarkGray)
                } else if is_current {
                    Style::default().fg(Color::Cyan)
                } else {
                    Style::default()
                };

                let prefix = if is_current { "♪ " } else { "  " };
                ListItem::new(format!(
                    "{}{} - {}",
                    prefix,
                    t.display_name(),
                    t.format_duration()
                ))
                .style(style)
            })
            .collect();

        let list = List::new(items)
            .block(library_block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▸ ");

        f.render_stateful_widget(list, area, &mut self.library_state.clone());
    }

    fn render_help(&self, f: &mut Frame, area: Rect) {
        let (help_text, style) = if self.search_mode {
            (
                format!(
                    " Search: {}▌  (Enter to search, Esc to cancel)",
                    self.search_query
                ),
                Style::default().fg(Color::DarkGray),
            )
        } else if self.edit_mode {
            (
                format!(
                    " Rename: {}▌  (Enter to save, Esc to cancel)",
                    self.edit_text
                ),
                Style::default().fg(Color::DarkGray),
            )
        } else if self.add_mode {
            (
                format!(" Add URL: {}▌  (Enter to add, Esc to cancel)", self.add_url),
                Style::default().fg(Color::DarkGray),
            )
        } else if let Some(ref msg) = self.status_message {
            (format!(" {}", msg), Style::default().fg(Color::Yellow))
        } else {
            (
                " q:Quit  /:Search  a:Add  e:Edit  ↑↓:Nav  ←→:Seek  Space:Play  +/-:Vol"
                    .to_string(),
                Style::default().fg(Color::DarkGray),
            )
        };

        let help = Paragraph::new(help_text).style(style);
        f.render_widget(help, area);
    }

    fn select_next(&mut self) {
        let len = self.tracks.len();
        if len == 0 {
            return;
        }

        let i = match self.library_state.selected() {
            Some(i) => (i + 1) % len,
            None => 0,
        };
        self.library_state.select(Some(i));
    }

    fn select_prev(&mut self) {
        let len = self.tracks.len();
        if len == 0 {
            return;
        }

        let i = match self.library_state.selected() {
            Some(i) => {
                if i == 0 {
                    len - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.library_state.select(Some(i));
    }

    fn play_selected(&mut self) {
        let Some(i) = self.library_state.selected() else {
            return;
        };
        let Some(track) = self.tracks.get(i) else {
            return;
        };
        if track.available {
            let _ = self.client.play(track.clone());
        }
    }

    fn toggle_or_play(&mut self) {
        // If something is playing, toggle pause/resume
        // If nothing is playing, play the selected track
        if self.playback_state.current_track.is_some() {
            if self.playback_state.is_playing {
                let _ = self.client.pause();
            } else {
                let _ = self.client.resume();
            }
        } else {
            self.play_selected();
        }
    }

    fn volume_up(&mut self) {
        let vol = (self.playback_state.volume + 5).min(100);
        let _ = self.client.set_volume(vol);
    }

    fn volume_down(&mut self) {
        let vol = self.playback_state.volume.saturating_sub(5);
        let _ = self.client.set_volume(vol);
    }

    fn seek_forward(&mut self) {
        if self.playback_state.current_track.is_some() {
            let new_pos = self.playback_state.position + 10;
            let _ = self.client.seek(new_pos);
        }
    }

    fn seek_backward(&mut self) {
        if self.playback_state.current_track.is_some() {
            let new_pos = self.playback_state.position.saturating_sub(10);
            let _ = self.client.seek(new_pos);
        }
    }

    fn start_edit(&mut self) {
        let Some(i) = self.library_state.selected() else {
            return;
        };
        let Some(track) = self.tracks.get(i) else {
            return;
        };
        // Pre-fill with current alias or title
        self.edit_text = track.alias.clone().unwrap_or_else(|| track.title.clone());
        self.edit_mode = true;
    }

    fn apply_edit(&mut self) {
        if self.edit_text.is_empty() {
            return;
        }

        let Some(i) = self.library_state.selected() else {
            self.edit_text.clear();
            return;
        };
        let Some(track) = self.tracks.get(i) else {
            self.edit_text.clear();
            return;
        };

        // Save the new alias to the database
        let new_alias = self.edit_text.trim().to_string();
        let alias = if new_alias == track.title {
            None // Clear alias if it matches the title
        } else {
            Some(new_alias.as_str())
        };

        if self.db.update_track_alias(&track.id, alias).is_ok() {
            // Update local track list
            if let Some(t) = self.tracks.get_mut(i) {
                t.alias = alias.map(|s| s.to_string());
            }
        }

        self.edit_text.clear();
    }

    fn add_track(&mut self) {
        let url = self.add_url.trim().to_string();
        self.add_url.clear();

        if url.is_empty() {
            return;
        }

        // Check if it looks like a YouTube URL
        if !url.contains("youtube.com") && !url.contains("youtu.be") {
            self.status_message = Some("Invalid URL - must be a YouTube URL".to_string());
            return;
        }

        let (tx, rx) = mpsc::channel();
        self.download_rx = Some(rx);
        self.status_message = Some("Checking video info...".to_string());

        let config = self.config.clone();
        let db_path = config.db_path();

        std::thread::spawn(move || {
            let downloader = Downloader::new(config);

            // Get video info and check for duplicates
            let canonical_url = match downloader.get_video_info(&url) {
                Ok((title, canonical_url, _)) => {
                    // Open a separate DB connection for the duplicate check
                    if let Ok(db) = Database::open(&db_path)
                        && let Ok(Some(_)) = db.get_track_by_url(&canonical_url)
                    {
                        let _ = tx.send(DownloadUpdate::Done(Err(format!(
                            "Already in library: {}",
                            title
                        ))));
                        return;
                    }
                    let _ = tx.send(DownloadUpdate::Status(format!("Downloading: {}...", title)));
                    canonical_url
                }
                Err(e) => {
                    let _ = tx.send(DownloadUpdate::Done(Err(format!("{}", e))));
                    return;
                }
            };

            let tx_progress = tx.clone();
            match downloader.download(&canonical_url, move |phase| {
                let _ = tx_progress.send(DownloadUpdate::Progress(phase));
            }) {
                Ok(track) => {
                    let _ = tx.send(DownloadUpdate::Done(Ok(track)));
                }
                Err(e) => {
                    let _ = tx.send(DownloadUpdate::Done(Err(format!("{}", e))));
                }
            }
        });
    }

    fn apply_search(&mut self) {
        if self.search_query.is_empty() {
            return;
        }

        use fuzzy_matcher::FuzzyMatcher;
        use fuzzy_matcher::skim::SkimMatcherV2;

        let matcher = SkimMatcherV2::default();
        let query = &self.search_query;

        let mut matches: Vec<_> = self
            .tracks
            .iter()
            .enumerate()
            .filter_map(|(i, track)| {
                let title_score = matcher.fuzzy_match(&track.title, query).unwrap_or(0);
                let alias_score = track
                    .alias
                    .as_ref()
                    .and_then(|a| matcher.fuzzy_match(a, query))
                    .unwrap_or(0);
                let score = title_score.max(alias_score);
                if score > 0 { Some((i, score)) } else { None }
            })
            .collect();

        matches.sort_by_key(|entry| std::cmp::Reverse(entry.1));

        if let Some((index, _)) = matches.first() {
            self.library_state.select(Some(*index));
        }

        self.search_query.clear();
    }
}

pub fn run(config: Config, db: Database) -> Result<()> {
    let mut tui = Tui::new(config, db)?;
    tui.run()
}
