use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::io::{BufRead, BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::config::Config;
use crate::models::Track;

pub enum DownloadPhase {
    Downloading {
        percent: f64,
        speed: String,
        eta: String,
    },
    Converting,
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

    pub fn check_dependencies() -> Result<()> {
        // Check yt-dlp
        let yt_dlp = Command::new("yt-dlp").arg("--version").output();

        if yt_dlp.is_err() {
            bail!(
                "yt-dlp is not installed. Please install it: https://github.com/yt-dlp/yt-dlp#installation"
            );
        }

        // Check ffmpeg
        let ffmpeg = Command::new("ffmpeg").arg("-version").output();

        if ffmpeg.is_err() {
            bail!("ffmpeg is not installed. Please install it: https://ffmpeg.org/download.html");
        }

        Ok(())
    }

    pub fn get_video_info(&self, url: &str) -> Result<(String, String, u64)> {
        let output = Command::new("yt-dlp")
            .args(["--dump-json", "--no-download", "--no-playlist", url])
            .output()
            .with_context(|| "Failed to run yt-dlp")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("yt-dlp failed: {stderr}");
        }

        let info: YtDlpInfo = serde_json::from_slice(&output.stdout)
            .with_context(|| "Failed to parse yt-dlp output")?;

        let duration = info.duration.unwrap_or(0.0) as u64;
        Ok((info.title, info.webpage_url, duration))
    }

    pub fn download(&self, url: &str, on_progress: impl Fn(DownloadPhase)) -> Result<Track> {
        let (title, canonical_url, duration) = self.get_video_info(url)?;

        let audio_dir = self.config.audio_dir();
        let format = &self.config.audio.format;

        // Generate a safe filename
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

        let mut child = Command::new("yt-dlp")
            .args([
                "-x", // Extract audio
                "--audio-format",
                format,
                "--audio-quality",
                "0", // Best quality
                "--no-playlist",
                "--progress",
                "--newline",
                "--progress-template",
                "download:PROGRESS:%(progress._percent_str)s:%(progress._speed_str)s:%(progress._eta_str)s",
                "--progress-template",
                "postprocess:POSTPROCESS",
                "-o",
                output_template.to_str().unwrap(),
                "--print",
                "after_move:filepath",
                &canonical_url,
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| "Failed to run yt-dlp")?;

        let stderr = child.stderr.take().unwrap();
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

        // stderr EOF — process has finished writing, read stdout and wait
        let mut stdout = child.stdout.take().unwrap();
        let mut stdout_str = String::new();
        stdout
            .read_to_string(&mut stdout_str)
            .with_context(|| "Failed to read yt-dlp output")?;

        let status = child.wait().with_context(|| "yt-dlp process failed")?;

        if !status.success() {
            bail!("Download failed: {}", stderr_output.trim());
        }

        let file_path = stdout_str.trim().to_string();

        if file_path.is_empty() || !Path::new(&file_path).exists() {
            // Try to find the file
            let expected_path = audio_dir.join(format!("{safe_title}.{format}"));
            if expected_path.exists() {
                return Ok(Track::new(
                    canonical_url,
                    title,
                    duration,
                    expected_path.to_string_lossy().to_string(),
                ));
            }
            bail!("Download completed but file not found");
        }

        Ok(Track::new(canonical_url, title, duration, file_path))
    }

    pub fn check_availability(&self, url: &str) -> Result<bool> {
        let output = Command::new("yt-dlp")
            .args(["--simulate", "--no-playlist", url])
            .output()
            .with_context(|| "Failed to check video availability")?;

        Ok(output.status.success())
    }

    #[allow(dead_code)]
    pub fn audio_dir(&self) -> PathBuf {
        self.config.audio_dir()
    }
}

#[allow(dead_code)]
pub fn extract_video_id(url: &str) -> Option<String> {
    // Handle various YouTube URL formats
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
}
