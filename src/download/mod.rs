use serde::Deserialize;
use std::io::{BufRead, BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use tracing::warn;

use crate::config::Config;
use crate::models::Track;

const MAX_RETRIES: u8 = 3;
const INITIAL_BACKOFF_MS: u64 = 300;

pub enum DownloadPhase {
    Downloading {
        percent: f64,
        speed: String,
        eta: String,
    },
    Converting,
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("{dependency} is not installed. {hint}")]
    DependencyMissing {
        dependency: &'static str,
        hint: &'static str,
    },
    #[error("Failed to run {command}: {source}")]
    CommandSpawn {
        command: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("yt-dlp {operation} failed: {message}")]
    YtDlpFailed {
        operation: &'static str,
        message: String,
        transient: bool,
    },
    #[error("Failed to parse yt-dlp output for {operation}: {source}")]
    ParseJson {
        operation: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("Failed to read yt-dlp output for {operation}: {source}")]
    ReadOutput {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
    #[error("Download completed but file not found: {path}")]
    FileNotFound { path: String },
    #[error("Invalid output path for yt-dlp template")]
    InvalidOutputPath,
}

impl DownloadError {
    fn is_retryable(&self) -> bool {
        matches!(
            self,
            DownloadError::YtDlpFailed {
                transient: true,
                ..
            } | DownloadError::CommandSpawn { .. }
        )
    }
}

#[derive(Debug, Deserialize)]
struct YtDlpInfo {
    #[allow(dead_code)]
    id: String,
    title: String,
    duration: Option<f64>,
    webpage_url: String,
}

pub struct Downloader {
    config: Config,
}

impl Downloader {
    pub fn new(config: Config) -> Self {
        Self { config }
    }

    pub fn check_dependencies() -> Result<(), DownloadError> {
        let yt_dlp = Command::new("yt-dlp")
            .arg("--version")
            .output()
            .map_err(|source| DownloadError::CommandSpawn {
                command: "yt-dlp",
                source,
            })?;

        if !yt_dlp.status.success() {
            return Err(DownloadError::DependencyMissing {
                dependency: "yt-dlp",
                hint: "Install it: https://github.com/yt-dlp/yt-dlp#installation",
            });
        }

        let ffmpeg = Command::new("ffmpeg")
            .arg("-version")
            .output()
            .map_err(|source| DownloadError::CommandSpawn {
                command: "ffmpeg",
                source,
            })?;

        if !ffmpeg.status.success() {
            return Err(DownloadError::DependencyMissing {
                dependency: "ffmpeg",
                hint: "Install it: https://ffmpeg.org/download.html",
            });
        }

        Ok(())
    }

    pub fn get_video_info(&self, url: &str) -> Result<(String, String, u64), DownloadError> {
        let output = retry_with_backoff("get_video_info", || {
            Command::new("yt-dlp")
                .args(["--dump-json", "--no-download", "--no-playlist", url])
                .output()
                .map_err(|source| DownloadError::CommandSpawn {
                    command: "yt-dlp",
                    source,
                })
                .and_then(|output| {
                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        Err(DownloadError::YtDlpFailed {
                            operation: "get_video_info",
                            transient: is_transient_failure(&stderr),
                            message: stderr,
                        })
                    } else {
                        Ok(output)
                    }
                })
        })?;

        let info: YtDlpInfo =
            serde_json::from_slice(&output.stdout).map_err(|source| DownloadError::ParseJson {
                operation: "get_video_info",
                source,
            })?;

        let duration = info.duration.unwrap_or(0.0) as u64;
        Ok((info.title, info.webpage_url, duration))
    }

    pub fn download(
        &self,
        url: &str,
        on_progress: impl Fn(DownloadPhase),
    ) -> Result<Track, DownloadError> {
        retry_with_backoff("download", || self.download_once(url, &on_progress))
    }

    fn download_once(
        &self,
        url: &str,
        on_progress: &impl Fn(DownloadPhase),
    ) -> Result<Track, DownloadError> {
        let (title, canonical_url, duration) = self.get_video_info(url)?;

        let audio_dir = self.config.audio_dir();
        let format = &self.config.audio.format;

        let safe_title: String = title
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == ' ' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let safe_title = safe_title.trim();

        let output_template = audio_dir.join(format!("{safe_title}.%(ext)s"));
        let output_template_str = output_template
            .to_str()
            .ok_or(DownloadError::InvalidOutputPath)?;

        let mut child = Command::new("yt-dlp")
            .args([
                "-x",
                "--audio-format",
                format,
                "--audio-quality",
                "0",
                "--no-playlist",
                "--progress",
                "--newline",
                "--progress-template",
                "download:PROGRESS:%(progress._percent_str)s:%(progress._speed_str)s:%(progress._eta_str)s",
                "--progress-template",
                "postprocess:POSTPROCESS",
                "-o",
                output_template_str,
                "--print",
                "after_move:filepath",
                &canonical_url,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|source| DownloadError::CommandSpawn {
                command: "yt-dlp",
                source,
            })?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| DownloadError::YtDlpFailed {
                operation: "download",
                transient: false,
                message: "yt-dlp stderr stream unavailable".to_string(),
            })?;
        let reader = BufReader::new(stderr);
        let mut stderr_output = String::new();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };

            if let Some(rest) = line.strip_prefix("PROGRESS:") {
                let parts: Vec<&str> = rest.splitn(3, ':').collect();
                if parts.len() == 3 {
                    let percent = parts[0]
                        .trim()
                        .trim_end_matches('%')
                        .parse::<f64>()
                        .unwrap_or(0.0);
                    let speed = parts[1].trim().to_string();
                    let eta = parts[2].trim().to_string();
                    on_progress(DownloadPhase::Downloading {
                        percent,
                        speed,
                        eta,
                    });
                }
            } else if line.starts_with("POSTPROCESS") {
                on_progress(DownloadPhase::Converting);
            } else {
                stderr_output.push_str(&line);
                stderr_output.push('\n');
            }
        }

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| DownloadError::YtDlpFailed {
                operation: "download",
                transient: false,
                message: "yt-dlp stdout stream unavailable".to_string(),
            })?;
        let mut stdout_str = String::new();
        stdout
            .read_to_string(&mut stdout_str)
            .map_err(|source| DownloadError::ReadOutput {
                operation: "download",
                source,
            })?;

        let status = child.wait().map_err(|source| DownloadError::CommandSpawn {
            command: "yt-dlp",
            source,
        })?;

        if !status.success() {
            return Err(DownloadError::YtDlpFailed {
                operation: "download",
                transient: is_transient_failure(&stderr_output),
                message: stderr_output.trim().to_string(),
            });
        }

        let file_path = stdout_str.trim().to_string();
        if file_path.is_empty() || !Path::new(&file_path).exists() {
            let expected_path = audio_dir.join(format!("{safe_title}.{format}"));
            if expected_path.exists() {
                return Ok(Track::new(
                    canonical_url,
                    title,
                    duration,
                    expected_path.to_string_lossy().to_string(),
                ));
            }
            return Err(DownloadError::FileNotFound {
                path: expected_path.to_string_lossy().to_string(),
            });
        }

        Ok(Track::new(canonical_url, title, duration, file_path))
    }

    pub fn check_availability(&self, url: &str) -> Result<bool, DownloadError> {
        let output = retry_with_backoff("check_availability", || {
            Command::new("yt-dlp")
                .args(["--simulate", "--no-playlist", url])
                .output()
                .map_err(|source| DownloadError::CommandSpawn {
                    command: "yt-dlp",
                    source,
                })
        })?;

        Ok(output.status.success())
    }

    #[allow(dead_code)]
    pub fn audio_dir(&self) -> PathBuf {
        self.config.audio_dir()
    }
}

fn retry_with_backoff<T>(
    operation: &'static str,
    mut f: impl FnMut() -> Result<T, DownloadError>,
) -> Result<T, DownloadError> {
    for attempt in 1..=MAX_RETRIES {
        match f() {
            Ok(result) => return Ok(result),
            Err(err) if err.is_retryable() && attempt < MAX_RETRIES => {
                let delay = backoff_delay(attempt);
                warn!(
                    operation,
                    attempt,
                    max_attempts = MAX_RETRIES,
                    retry_in_ms = delay.as_millis(),
                    error = %err,
                    "Transient downloader failure, retrying"
                );
                thread::sleep(delay);
            }
            Err(err) => {
                warn!(
                    operation,
                    attempt,
                    max_attempts = MAX_RETRIES,
                    error = %err,
                    "Downloader operation failed"
                );
                return Err(err);
            }
        }
    }

    Err(DownloadError::YtDlpFailed {
        operation,
        message: "Retry loop exhausted unexpectedly".to_string(),
        transient: false,
    })
}

fn backoff_delay(attempt: u8) -> Duration {
    let scale = 1_u64 << (attempt.saturating_sub(1) as u32);
    Duration::from_millis(INITIAL_BACKOFF_MS * scale)
}

fn is_transient_failure(stderr: &str) -> bool {
    let lower = stderr.to_lowercase();
    let indicators = [
        "timed out",
        "timeout",
        "temporarily unavailable",
        "try again",
        "connection reset",
        "connection refused",
        "network is unreachable",
        "429",
        "502",
        "503",
        "504",
        "http error 5",
        "remote end closed connection",
    ];
    indicators.iter().any(|token| lower.contains(token))
}

#[allow(dead_code)]
pub fn extract_video_id(url: &str) -> Option<String> {
    if url.contains("youtu.be/") {
        url.split("youtu.be/")
            .nth(1)
            .and_then(|s| s.split(['?', '&']).next())
            .map(|s| s.to_string())
    } else if url.contains("youtube.com") {
        url.split(['?', '&'])
            .find(|s| s.starts_with("v="))
            .map(|s| s[2..].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_video_id() {
        assert_eq!(
            extract_video_id("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            extract_video_id("https://youtu.be/dQw4w9WgXcQ"),
            Some("dQw4w9WgXcQ".to_string())
        );
        assert_eq!(
            extract_video_id("https://youtube.com/watch?v=abc123&t=10"),
            Some("abc123".to_string())
        );
    }

    #[test]
    fn transient_failure_classifier_detects_network_errors() {
        assert!(is_transient_failure(
            "ERROR: HTTP Error 503: Service Unavailable"
        ));
        assert!(is_transient_failure("Connection timed out"));
        assert!(!is_transient_failure("Video unavailable"));
    }

    #[test]
    fn invalid_url_returns_typed_download_failure() {
        let downloader = Downloader::new(Config::default());
        let err = downloader.get_video_info("not-a-url").unwrap_err();
        assert!(matches!(err, DownloadError::YtDlpFailed { .. }));
    }
}
