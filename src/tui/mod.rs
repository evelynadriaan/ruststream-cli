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
use serde_json::Value;
use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::db::Database;
use crate::download::{DownloadPhase, Downloader};
use crate::ipc::{DaemonClient, DaemonResponse};
use crate::models::{PlaybackState, StreamEntry, Track};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum AppMode {
    #[default]
    Normal,
    Search,
    YtSearch,
    Edit,
    AddUrl,
    StreamUrl,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum LibraryView {
    #[default]
    Local,
    StreamQueue,
    YtResults,
}

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
    stream_queue_state: ListState,
    yt_results: Vec<StreamEntry>,
    yt_results_state: ListState,
    playback_state: PlaybackState,
    mode: AppMode,
    input_buf: String,
    library_view: LibraryView,
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

        let library_view = if playback_state.is_streaming {
            LibraryView::StreamQueue
        } else {
            LibraryView::Local
        };

        let mut stream_queue_state = ListState::default();
        if !playback_state.stream_queue.is_empty() {
            let idx = playback_state
                .stream_queue_index
                .min(playback_state.stream_queue.len() - 1);
            stream_queue_state.select(Some(idx));
        }

        Ok(Self {
            config,
            db,
            client,
            tracks,
            library_state,
            stream_queue_state,
            yt_results: Vec::new(),
            yt_results_state: ListState::default(),
            playback_state,
            mode: AppMode::Normal,
            input_buf: String::new(),
            library_view,
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
            let was_streaming = self.playback_state.is_streaming;

            // Refresh playback state
            if self.client.is_daemon_running() {
                if let Ok(state) = self.client.get_status() {
                    self.playback_state = state;
                }
            }

            self.sync_library_view_with_streaming(was_streaming);

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

                    match self.mode {
                        AppMode::Search => match key.code {
                            KeyCode::Esc => {
                                self.mode = AppMode::Normal;
                                self.input_buf.clear();
                            }
                            KeyCode::Enter => {
                                self.mode = AppMode::Normal;
                                self.apply_search();
                            }
                            KeyCode::Backspace => {
                                self.input_buf.pop();
                            }
                            KeyCode::Char(c) => {
                                self.input_buf.push(c);
                            }
                            _ => {}
                        },
                        AppMode::YtSearch => match key.code {
                            KeyCode::Esc => {
                                self.mode = AppMode::Normal;
                                self.input_buf.clear();
                                self.yt_results.clear();
                                self.yt_results_state.select(None);
                            }
                            KeyCode::Enter => {
                                self.mode = AppMode::Normal;
                                self.search_youtube();
                            }
                            KeyCode::Backspace => {
                                self.input_buf.pop();
                            }
                            KeyCode::Char(c) => {
                                self.input_buf.push(c);
                            }
                            _ => {}
                        },
                        AppMode::Edit => match key.code {
                            KeyCode::Esc => {
                                self.mode = AppMode::Normal;
                                self.input_buf.clear();
                            }
                            KeyCode::Enter => {
                                self.mode = AppMode::Normal;
                                self.apply_edit();
                            }
                            KeyCode::Backspace => {
                                self.input_buf.pop();
                            }
                            KeyCode::Char(c) => {
                                self.input_buf.push(c);
                            }
                            _ => {}
                        },
                        AppMode::AddUrl => match key.code {
                            KeyCode::Esc => {
                                self.mode = AppMode::Normal;
                                self.input_buf.clear();
                            }
                            KeyCode::Enter => {
                                self.mode = AppMode::Normal;
                                self.add_track();
                            }
                            KeyCode::Backspace => {
                                self.input_buf.pop();
                            }
                            KeyCode::Char(c) => {
                                self.input_buf.push(c);
                            }
                            _ => {}
                        },
                        AppMode::StreamUrl => match key.code {
                            KeyCode::Esc => {
                                self.mode = AppMode::Normal;
                                self.input_buf.clear();
                            }
                            KeyCode::Enter => {
                                self.mode = AppMode::Normal;
                                self.start_stream();
                            }
                            KeyCode::Backspace => {
                                self.input_buf.pop();
                            }
                            KeyCode::Char(c) => {
                                self.input_buf.push(c);
                            }
                            _ => {}
                        },
                        AppMode::Normal => {
                            // Clear status message on any normal-mode key press
                            self.status_message = None;
                            match key.code {
                                KeyCode::Char('q') => return Ok(()),
                                KeyCode::Char('/') => {
                                    self.library_view = LibraryView::Local;
                                    self.mode = AppMode::Search;
                                    self.input_buf.clear();
                                }
                                KeyCode::Char('?') => {
                                    self.mode = AppMode::YtSearch;
                                    self.input_buf.clear();
                                }
                                KeyCode::Esc if self.library_view == LibraryView::YtResults => {
                                    self.library_view = LibraryView::Local;
                                }
                                KeyCode::Char('e') => self.start_edit(),
                                KeyCode::Char('a')
                                    if self.library_view == LibraryView::YtResults
                                        && self.download_rx.is_none() =>
                                {
                                    self.add_selected_yt_result_to_library();
                                }
                                KeyCode::Char('a') if self.download_rx.is_none() => {
                                    self.library_view = LibraryView::Local;
                                    self.mode = AppMode::AddUrl;
                                    self.input_buf.clear();
                                }
                                KeyCode::Char('s') => {
                                    self.mode = AppMode::StreamUrl;
                                    self.input_buf.clear();
                                }
                                KeyCode::Char('l') => {
                                    self.library_view = LibraryView::Local;
                                }
                                KeyCode::Char('d') | KeyCode::Delete => {
                                    self.remove_selected_track();
                                }
                                KeyCode::Char('S') if self.playback_state.is_streaming => {
                                    self.save_current_stream_shortcut();
                                }
                                KeyCode::Up | KeyCode::Char('k') => self.select_prev(),
                                KeyCode::Down | KeyCode::Char('j') => self.select_next(),
                                KeyCode::Left | KeyCode::Char('h') => self.prev_or_seek_backward(),
                                KeyCode::Right => self.next_or_seek_forward(),
                                KeyCode::Enter if self.library_view == LibraryView::YtResults => {
                                    self.stream_selected_yt_result();
                                }
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
    }

    fn sync_library_view_with_streaming(&mut self, was_streaming: bool) {
        if !was_streaming
            && self.playback_state.is_streaming
            && !self.playback_state.stream_queue.is_empty()
        {
            self.library_view = LibraryView::StreamQueue;
            let idx = self
                .playback_state
                .stream_queue_index
                .min(self.playback_state.stream_queue.len() - 1);
            self.stream_queue_state.select(Some(idx));
        }

        if was_streaming && !self.playback_state.is_streaming {
            self.library_view = LibraryView::Local;
        }

        if self.playback_state.stream_queue.is_empty() {
            self.stream_queue_state.select(None);
            return;
        }

        let len = self.playback_state.stream_queue.len();
        match self.stream_queue_state.selected() {
            Some(i) if i < len => {}
            _ => {
                let idx = self.playback_state.stream_queue_index.min(len - 1);
                self.stream_queue_state.select(Some(idx));
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

            let display_title = if self.playback_state.is_streaming {
                self.playback_state
                    .stream_queue
                    .get(self.playback_state.stream_queue_index)
                    .map(|entry| entry.title.as_str())
                    .unwrap_or_else(|| track.display_name())
            } else {
                track.display_name()
            };

            // Track title (centered, bold)
            let status_icon = if self.playback_state.is_playing {
                "▶ "
            } else {
                "⏸ "
            };
            let mut title_spans = vec![
                Span::styled(status_icon, Style::default().fg(Color::Cyan)),
                Span::styled(display_title, Style::default().add_modifier(Modifier::BOLD)),
            ];
            if self.playback_state.is_streaming {
                title_spans.push(Span::raw(" "));
                title_spans.push(Span::styled(
                    "[STREAMING]",
                    Style::default().fg(Color::Yellow),
                ));
            }

            let title = Paragraph::new(Line::from(title_spans)).alignment(Alignment::Center);
            f.render_widget(title, chunks[0]);

            // Progress bar
            let gauge = if self.playback_state.is_streaming && track.duration == 0 {
                Gauge::default()
                    .ratio(1.0)
                    .gauge_style(Style::default().fg(Color::Yellow).bg(Color::DarkGray))
                    .label(Span::styled(
                        "▶ LIVE",
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ))
            } else {
                let progress = if track.duration > 0 {
                    (self.playback_state.position as f64 / track.duration as f64).min(1.0)
                } else {
                    0.0
                };

                Gauge::default()
                    .ratio(progress)
                    .gauge_style(Style::default().fg(Color::Cyan).bg(Color::DarkGray))
                    .label("")
            };
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
        match self.library_view {
            LibraryView::Local => self.render_local_library(f, area),
            LibraryView::StreamQueue => self.render_stream_queue(f, area),
            LibraryView::YtResults => self.render_yt_results(f, area),
        }
    }

    fn render_local_library(&self, f: &mut Frame, area: Rect) {
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

    fn render_stream_queue(&self, f: &mut Frame, area: Rect) {
        let queue_len = self.playback_state.stream_queue.len();
        let current_idx = self
            .playback_state
            .stream_queue_index
            .min(queue_len.saturating_sub(1));

        let block = Block::default()
            .title(format!(
                " Stream Queue ({}) - press l for Library ",
                queue_len
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow));

        let items: Vec<ListItem> = self
            .playback_state
            .stream_queue
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let is_current = i == current_idx;
                let prefix = if is_current { "♪ " } else { "  " };
                let style = if is_current {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default()
                };
                ListItem::new(format!("{}{}", prefix, entry.title)).style(style)
            })
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▸ ");

        f.render_stateful_widget(list, area, &mut self.stream_queue_state.clone());
    }

    fn render_yt_results(&self, f: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(" YouTube Results — Enter:Stream  a:Add to library  Esc:Back ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Red));

        let items: Vec<ListItem> = self
            .yt_results
            .iter()
            .enumerate()
            .map(|(i, entry)| ListItem::new(format!("{:>2}. {}", i + 1, entry.title)))
            .collect();

        let list = List::new(items)
            .block(block)
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
            .highlight_symbol("▸ ");

        f.render_stateful_widget(list, area, &mut self.yt_results_state.clone());
    }

    fn render_help(&self, f: &mut Frame, area: Rect) {
        let (help_text, style) = match self.mode {
            AppMode::Search => (
                format!(
                    " Search: {}▌  (Enter to search, Esc to cancel)",
                    self.input_buf
                ),
                Style::default().fg(Color::DarkGray),
            ),
            AppMode::YtSearch => (
                format!(
                    " Search YouTube: {}▌  Enter to search  Esc to cancel ",
                    self.input_buf
                ),
                Style::default().fg(Color::DarkGray),
            ),
            AppMode::Edit => (
                format!(
                    " Rename: {}▌  (Enter to save, Esc to cancel)",
                    self.input_buf
                ),
                Style::default().fg(Color::DarkGray),
            ),
            AppMode::AddUrl => (
                format!(
                    " Add URL: {}▌  (Enter to add, Esc to cancel)",
                    self.input_buf
                ),
                Style::default().fg(Color::DarkGray),
            ),
            AppMode::StreamUrl => (
                format!(
                    " Stream URL: {}▌  (Enter to stream, Esc to cancel)",
                    self.input_buf
                ),
                Style::default().fg(Color::DarkGray),
            ),
            AppMode::Normal => {
                if let Some(ref msg) = self.status_message {
                    (format!(" {}", msg), Style::default().fg(Color::Yellow))
                } else {
                    let save_stream_hint = if self.playback_state.is_streaming {
                        "  S:Save stream"
                    } else {
                        ""
                    };
                    (
                        format!(
                            " q:Quit  /:Search  a:Add  s:Stream  e:Edit  d:Remove  l:Library{}  ↑↓:Nav  ←/h:Prev  →:Next/Seek  Space:Play  +/-:Vol",
                            save_stream_hint
                        ),
                        Style::default().fg(Color::DarkGray),
                    )
                }
            }
        };

        let help = Paragraph::new(help_text).style(style);
        f.render_widget(help, area);
    }

    fn select_next(&mut self) {
        match self.library_view {
            LibraryView::Local => {
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
            LibraryView::StreamQueue => {
                let len = self.playback_state.stream_queue.len();
                if len == 0 {
                    return;
                }

                let i = match self.stream_queue_state.selected() {
                    Some(i) => (i + 1) % len,
                    None => 0,
                };
                self.stream_queue_state.select(Some(i));
            }
            LibraryView::YtResults => {
                let len = self.yt_results.len();
                if len == 0 {
                    return;
                }

                let i = match self.yt_results_state.selected() {
                    Some(i) => (i + 1) % len,
                    None => 0,
                };
                self.yt_results_state.select(Some(i));
            }
        }
    }

    fn select_prev(&mut self) {
        match self.library_view {
            LibraryView::Local => {
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
            LibraryView::StreamQueue => {
                let len = self.playback_state.stream_queue.len();
                if len == 0 {
                    return;
                }

                let i = match self.stream_queue_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            len - 1
                        } else {
                            i - 1
                        }
                    }
                    None => 0,
                };
                self.stream_queue_state.select(Some(i));
            }
            LibraryView::YtResults => {
                let len = self.yt_results.len();
                if len == 0 {
                    return;
                }

                let i = match self.yt_results_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            len - 1
                        } else {
                            i - 1
                        }
                    }
                    None => 0,
                };
                self.yt_results_state.select(Some(i));
            }
        }
    }

    fn play_selected(&mut self) {
        if self.library_view != LibraryView::Local {
            return;
        }

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

    fn next_or_seek_forward(&mut self) {
        if self.playback_state.is_streaming {
            let _ = self.client.next();
        } else {
            self.seek_forward();
        }
    }

    fn prev_or_seek_backward(&mut self) {
        if self.playback_state.is_streaming {
            let _ = self.client.previous();
        } else {
            self.seek_backward();
        }
    }

    fn start_edit(&mut self) {
        if self.library_view != LibraryView::Local {
            return;
        }

        let Some(i) = self.library_state.selected() else {
            return;
        };
        let Some(track) = self.tracks.get(i) else {
            return;
        };
        // Pre-fill with current alias or title
        self.input_buf = track.alias.clone().unwrap_or_else(|| track.title.clone());
        self.mode = AppMode::Edit;
    }

    fn apply_edit(&mut self) {
        if self.input_buf.is_empty() {
            return;
        }

        if self.library_view != LibraryView::Local {
            self.input_buf.clear();
            return;
        }

        let Some(i) = self.library_state.selected() else {
            self.input_buf.clear();
            return;
        };
        let Some(track) = self.tracks.get(i) else {
            self.input_buf.clear();
            return;
        };

        // Save the new alias to the database
        let new_alias = self.input_buf.trim().to_string();
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

        self.input_buf.clear();
    }

    fn remove_selected_track(&mut self) {
        if self.library_view != LibraryView::Local {
            return;
        }

        let Some(i) = self.library_state.selected() else {
            return;
        };
        let Some(track) = self.tracks.get(i).cloned() else {
            return;
        };

        if let Err(e) = self.db.delete_track(&track.id) {
            self.status_message = Some(format!("Failed to remove: {}", e));
            return;
        }

        let deleted_was_playing = self
            .playback_state
            .current_track
            .as_ref()
            .map(|current| current.id == track.id)
            .unwrap_or(false);
        if deleted_was_playing {
            let _ = self.client.stop();
        }

        let removed_track = self.tracks.remove(i);
        if self.tracks.is_empty() {
            self.library_state.select(None);
        } else {
            let next_index = i.min(self.tracks.len() - 1);
            self.library_state.select(Some(next_index));
        }

        self.status_message = Some(format!("Removed: {}", removed_track.display_name()));
    }

    fn save_current_stream_shortcut(&mut self) {
        if !self.playback_state.is_streaming {
            return;
        }

        self.status_message = match self.client.save_current_stream() {
            Ok(DaemonResponse::Ok) => Some("Saving stream to library…".to_string()),
            Ok(DaemonResponse::Error { message, .. }) => Some(format!("Save failed: {}", message)),
            Ok(_) => Some("Unexpected daemon response".to_string()),
            Err(e) => Some(format!("Save failed: {}", e)),
        };
    }

    fn add_track(&mut self) {
        let url = self.input_buf.trim().to_string();
        self.input_buf.clear();

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

    fn start_stream(&mut self) {
        let url = self.input_buf.trim().to_string();
        self.input_buf.clear();

        if url.is_empty() {
            self.status_message = Some("Enter a YouTube URL".to_string());
            return;
        }

        if !url.contains("youtube.com") && !url.contains("youtu.be") {
            self.status_message = Some("Invalid URL - must be a YouTube URL".to_string());
            return;
        }

        if url.contains("list=") {
            let entries = match fetch_playlist_entries(&url) {
                Ok(entries) => entries,
                Err(e) => {
                    self.status_message = Some(format!("Playlist fetch failed: {}", e));
                    return;
                }
            };

            let entry_count = entries.len();
            self.status_message =
                Some(format!("Loading stream queue ({} entries)...", entry_count));

            self.status_message = match self.client.stream_queue_load(entries) {
                Ok(DaemonResponse::Ok) => {
                    self.library_view = LibraryView::StreamQueue;
                    Some(format!("Streaming playlist ({} entries)", entry_count))
                }
                Ok(DaemonResponse::Error { message, .. }) => {
                    Some(format!("Stream queue failed: {}", message))
                }
                Ok(_) => Some("Unexpected daemon response".to_string()),
                Err(e) => Some(format!("Stream queue failed: {}", e)),
            };
        } else {
            self.status_message = match self.client.stream(url.clone()) {
                Ok(DaemonResponse::Ok) => Some(format!("Streaming: {}", url)),
                Ok(DaemonResponse::Error { message, .. }) => {
                    Some(format!("Stream failed: {}", message))
                }
                Ok(_) => Some("Unexpected daemon response".to_string()),
                Err(e) => Some(format!("Stream failed: {}", e)),
            };
        }
    }

    fn search_youtube(&mut self) {
        let query = self.input_buf.trim().to_string();
        self.input_buf.clear();

        if query.is_empty() {
            self.status_message = Some("Enter a search query".to_string());
            return;
        }

        self.status_message = Some(format!("Searching YouTube: {}", query));

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(fetch_yt_search_results(&query));
        });

        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(results)) => {
                self.yt_results = results;
                if self.yt_results.is_empty() {
                    self.yt_results_state.select(None);
                    self.status_message = Some("No YouTube results found".to_string());
                    return;
                }
                self.yt_results_state.select(Some(0));
                self.library_view = LibraryView::YtResults;
                self.status_message =
                    Some(format!("Found {} YouTube results", self.yt_results.len()));
            }
            Ok(Err(e)) => {
                self.status_message = Some(format!("YouTube search failed: {}", e));
            }
            Err(_) => {
                self.status_message = Some("YouTube search timed out".to_string());
            }
        }
    }

    fn stream_selected_yt_result(&mut self) {
        if self.library_view != LibraryView::YtResults {
            return;
        }

        let Some(i) = self.yt_results_state.selected() else {
            return;
        };
        let Some(entry) = self.yt_results.get(i) else {
            return;
        };

        self.status_message = match self.client.stream(entry.url.clone()) {
            Ok(DaemonResponse::Ok) => Some(format!("Streaming: {}", entry.title)),
            Ok(DaemonResponse::Error { message, .. }) => {
                Some(format!("Stream failed: {}", message))
            }
            Ok(_) => Some("Unexpected daemon response".to_string()),
            Err(e) => Some(format!("Stream failed: {}", e)),
        };
    }

    fn add_selected_yt_result_to_library(&mut self) {
        if self.library_view != LibraryView::YtResults {
            return;
        }

        let Some(i) = self.yt_results_state.selected() else {
            return;
        };
        let Some(entry) = self.yt_results.get(i) else {
            return;
        };

        self.input_buf = entry.url.clone();
        self.add_track();
    }

    fn apply_search(&mut self) {
        if self.input_buf.is_empty() {
            return;
        }

        use fuzzy_matcher::FuzzyMatcher;
        use fuzzy_matcher::skim::SkimMatcherV2;

        let matcher = SkimMatcherV2::default();
        let query = &self.input_buf;

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

        self.input_buf.clear();
    }
}

pub fn fetch_playlist_entries(url: &str) -> std::result::Result<Vec<StreamEntry>, String> {
    fetch_playlist_entries_with_command("yt-dlp", url)
}

pub fn fetch_yt_search_results(query: &str) -> std::result::Result<Vec<StreamEntry>, String> {
    fetch_yt_search_results_with_command("yt-dlp", query)
}

fn fetch_yt_search_results_with_command(
    command: &str,
    query: &str,
) -> std::result::Result<Vec<StreamEntry>, String> {
    let yt_query = format!("ytsearch10:{}", query);
    let mut child = Command::new(command)
        .args([yt_query.as_str(), "--flat-playlist", "--dump-json"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run yt-dlp: {}", e))?;

    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                if let Some(mut out) = child.stdout.take() {
                    out.read_to_string(&mut stdout)
                        .map_err(|e| format!("Failed to read yt-dlp output: {}", e))?;
                }

                let mut stderr = String::new();
                if let Some(mut err) = child.stderr.take() {
                    err.read_to_string(&mut stderr)
                        .map_err(|e| format!("Failed to read yt-dlp stderr: {}", e))?;
                }

                if !status.success() {
                    let err = stderr.trim();
                    return Err(if err.is_empty() {
                        "yt-dlp search failed".to_string()
                    } else {
                        format!("yt-dlp failed: {}", err)
                    });
                }

                let mut entries = parse_playlist_entries_lines(&stdout)?;
                if entries.len() > 10 {
                    entries.truncate(10);
                }
                return Ok(entries);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Timed out fetching YouTube search results".to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(format!("Failed while waiting for yt-dlp: {}", e));
            }
        }
    }
}

pub fn fetch_playlist_entries_with_command(
    command: &str,
    url: &str,
) -> std::result::Result<Vec<StreamEntry>, String> {
    let mut child = Command::new(command)
        .args(["--flat-playlist", "--dump-json", url])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to run yt-dlp: {}", e))?;

    let deadline = Instant::now() + Duration::from_secs(30);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                if let Some(mut out) = child.stdout.take() {
                    out.read_to_string(&mut stdout)
                        .map_err(|e| format!("Failed to read yt-dlp output: {}", e))?;
                }

                let mut stderr = String::new();
                if let Some(mut err) = child.stderr.take() {
                    err.read_to_string(&mut stderr)
                        .map_err(|e| format!("Failed to read yt-dlp stderr: {}", e))?;
                }

                if !status.success() {
                    let err = stderr.trim();
                    return Err(if err.is_empty() {
                        "yt-dlp failed to resolve playlist".to_string()
                    } else {
                        format!("yt-dlp failed: {}", err)
                    });
                }

                return parse_playlist_entries_lines(&stdout);
            }
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Timed out fetching playlist entries".to_string());
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                return Err(format!("Failed while waiting for yt-dlp: {}", e));
            }
        }
    }
}

fn parse_playlist_entries_lines(stdout: &str) -> std::result::Result<Vec<StreamEntry>, String> {
    let mut entries = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let json: Value = serde_json::from_str(line)
            .map_err(|e| format!("Failed to parse yt-dlp JSON line: {}", e))?;

        let title = json
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("Untitled")
            .trim();

        let raw_url = json
            .get("webpage_url")
            .and_then(Value::as_str)
            .or_else(|| json.get("url").and_then(Value::as_str))
            .unwrap_or("")
            .trim();

        if raw_url.is_empty() {
            continue;
        }

        let resolved_url = if raw_url.starts_with("http://") || raw_url.starts_with("https://") {
            raw_url.to_string()
        } else {
            format!("https://www.youtube.com/watch?v={}", raw_url)
        };

        entries.push(StreamEntry {
            title: if title.is_empty() {
                "Untitled".to_string()
            } else {
                title.to_string()
            },
            url: resolved_url,
        });
    }

    if entries.is_empty() {
        return Err("Playlist contains no playable entries".to_string());
    }

    Ok(entries)
}

pub fn run(config: Config, db: Database) -> Result<()> {
    let mut tui = Tui::new(config, db)?;
    tui.run()
}
