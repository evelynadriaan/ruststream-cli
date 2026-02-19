use assert_cmd::Command;
use clistream::ipc::{DaemonClient, DaemonResponse};
use clistream::models::{PlaybackState, StreamEntry};
use clistream::tui::fetch_playlist_entries_with_command;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const YOUTUBE_TEST_URL: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";

struct TestHarness {
    _temp: TempDir,
    data_home: String,
    config_home: String,
    runtime_dir: String,
    bin: &'static str,
}

impl TestHarness {
    fn new() -> Self {
        let temp = tempfile::tempdir().expect("failed to create temp dir");
        let data_home = temp.path().join("data");
        let config_home = temp.path().join("config");
        let runtime_dir = temp.path().join("run");

        fs::create_dir_all(&data_home).expect("failed to create data dir");
        fs::create_dir_all(&config_home).expect("failed to create config dir");
        fs::create_dir_all(&runtime_dir).expect("failed to create runtime dir");

        Self {
            _temp: temp,
            data_home: data_home.to_string_lossy().to_string(),
            config_home: config_home.to_string_lossy().to_string(),
            runtime_dir: runtime_dir.to_string_lossy().to_string(),
            bin: env!("CARGO_BIN_EXE_clistream"),
        }
    }

    fn cmd(&self) -> Command {
        let mut c = Command::new(self.bin);
        c.env("XDG_DATA_HOME", &self.data_home)
            .env("XDG_CONFIG_HOME", &self.config_home)
            .env("XDG_RUNTIME_DIR", &self.runtime_dir);
        c
    }

    fn start_daemon(&self) {
        self.cmd().args(["daemon", "start"]).assert().success();
    }

    fn stop_daemon(&self) {
        let _ = self.cmd().args(["daemon", "stop"]).output();
    }

    fn socket_path(&self) -> PathBuf {
        Path::new(&self.runtime_dir)
            .join("clistream")
            .join("clistream.sock")
    }

    fn client(&self) -> DaemonClient {
        DaemonClient::new(self.socket_path())
    }
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        self.stop_daemon();
    }
}

fn has_binary(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn wait_for_status<F>(client: &DaemonClient, timeout: Duration, predicate: F) -> PlaybackState
where
    F: Fn(&PlaybackState) -> bool,
{
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(status) = client.get_status()
            && predicate(&status)
        {
            return status;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for daemon status predicate"
        );
        thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn stream_single_url_sends_correct_ipc_command() {
    if !has_binary("mpv") || !has_binary("yt-dlp") {
        return;
    }

    let harness = TestHarness::new();
    harness.start_daemon();
    let client = harness.client();

    let response = client
        .stream(YOUTUBE_TEST_URL.to_string())
        .expect("stream command IPC failed");

    assert!(
        matches!(response, DaemonResponse::Ok),
        "expected Ok response, got: {:?}",
        response
    );

    let status = wait_for_status(&client, Duration::from_secs(5), |s| s.is_streaming);
    assert!(status.is_streaming);
}

#[test]
fn stream_queue_load_populates_state() {
    if !has_binary("mpv") {
        return;
    }

    let harness = TestHarness::new();
    harness.start_daemon();
    let client = harness.client();

    let entries = vec![
        StreamEntry {
            title: "Fake 1".to_string(),
            url: "file:///fake1".to_string(),
        },
        StreamEntry {
            title: "Fake 2".to_string(),
            url: "file:///fake2".to_string(),
        },
        StreamEntry {
            title: "Fake 3".to_string(),
            url: "file:///fake3".to_string(),
        },
    ];

    let response = client
        .stream_queue_load(entries)
        .expect("stream_queue_load IPC failed");
    assert!(matches!(
        response,
        DaemonResponse::Ok | DaemonResponse::Error { .. }
    ));

    let status = wait_for_status(&client, Duration::from_secs(5), |s| {
        s.is_streaming && s.stream_queue.len() == 3
    });

    assert_eq!(status.stream_queue.len(), 3);
    assert_eq!(status.stream_queue_index, 0);
    assert!(status.is_streaming);
}

#[test]
fn next_advances_stream_queue_index() {
    if !has_binary("mpv") {
        return;
    }

    let harness = TestHarness::new();
    harness.start_daemon();
    let client = harness.client();

    let entries = vec![
        StreamEntry {
            title: "Fake 1".to_string(),
            url: "file:///fake1".to_string(),
        },
        StreamEntry {
            title: "Fake 2".to_string(),
            url: "file:///fake2".to_string(),
        },
        StreamEntry {
            title: "Fake 3".to_string(),
            url: "file:///fake3".to_string(),
        },
    ];

    let _ = client
        .stream_queue_load(entries)
        .expect("stream_queue_load IPC failed");

    let initial = wait_for_status(&client, Duration::from_secs(5), |s| {
        s.is_streaming && s.stream_queue.len() == 3
    });
    assert_eq!(initial.stream_queue_index, 0);

    let response = client.next().expect("next IPC failed");
    assert!(matches!(
        response,
        DaemonResponse::Ok | DaemonResponse::Error { .. }
    ));

    let status = wait_for_status(&client, Duration::from_secs(5), |s| {
        s.stream_queue_index == 1
    });
    assert_eq!(status.stream_queue_index, 1);
}

#[test]
fn previous_wraps_stream_queue_index() {
    if !has_binary("mpv") {
        return;
    }

    let harness = TestHarness::new();
    harness.start_daemon();
    let client = harness.client();

    let entries = vec![
        StreamEntry {
            title: "Fake 1".to_string(),
            url: "file:///fake1".to_string(),
        },
        StreamEntry {
            title: "Fake 2".to_string(),
            url: "file:///fake2".to_string(),
        },
        StreamEntry {
            title: "Fake 3".to_string(),
            url: "file:///fake3".to_string(),
        },
    ];

    let _ = client
        .stream_queue_load(entries)
        .expect("stream_queue_load IPC failed");

    let initial = wait_for_status(&client, Duration::from_secs(5), |s| {
        s.is_streaming && s.stream_queue.len() == 3
    });
    assert_eq!(initial.stream_queue_index, 0);

    let response = client.previous().expect("previous IPC failed");
    assert!(matches!(
        response,
        DaemonResponse::Ok | DaemonResponse::Error { .. }
    ));

    let status = wait_for_status(&client, Duration::from_secs(5), |s| {
        s.stream_queue_index == 2
    });
    assert_eq!(status.stream_queue_index, 2);
}

#[test]
fn stop_clears_streaming_state() {
    if !has_binary("mpv") {
        return;
    }

    let harness = TestHarness::new();
    harness.start_daemon();
    let client = harness.client();

    let entries = vec![
        StreamEntry {
            title: "Fake 1".to_string(),
            url: "file:///fake1".to_string(),
        },
        StreamEntry {
            title: "Fake 2".to_string(),
            url: "file:///fake2".to_string(),
        },
        StreamEntry {
            title: "Fake 3".to_string(),
            url: "file:///fake3".to_string(),
        },
    ];

    let _ = client
        .stream_queue_load(entries)
        .expect("stream_queue_load IPC failed");

    let _ = wait_for_status(&client, Duration::from_secs(5), |s| {
        s.is_streaming && s.stream_queue.len() == 3
    });

    let response = client.stop().expect("stop IPC failed");
    assert!(matches!(response, DaemonResponse::Ok));

    let status = wait_for_status(&client, Duration::from_secs(5), |s| {
        !s.is_streaming && s.stream_queue.is_empty()
    });

    assert!(!status.is_streaming);
    assert!(status.stream_queue.is_empty());
    assert_eq!(status.stream_queue_index, 0);
}

#[test]
fn fetch_playlist_entries_parses_yt_dlp_output() {
    let temp = tempfile::tempdir().expect("failed to create temp dir");
    let fake_yt_dlp = temp.path().join("yt-dlp");

    fs::write(
        &fake_yt_dlp,
        r#"#!/bin/sh
set -eu
cat <<'JSON'
{"title":"Track One","url":"https://example.com/one"}
{"title":"Track Two","url":"https://example.com/two"}
{"title":"Track Three","url":"dQw4w9WgXcQ"}
JSON
"#,
    )
    .expect("failed to write fake yt-dlp");

    let mut perms = fs::metadata(&fake_yt_dlp)
        .expect("failed to stat fake yt-dlp")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&fake_yt_dlp, perms).expect("failed to make fake yt-dlp executable");

    let entries = fetch_playlist_entries_with_command(
        fake_yt_dlp
            .to_str()
            .expect("fake yt-dlp path should be valid UTF-8"),
        "https://www.youtube.com/playlist?list=PL123",
    )
    .expect("expected mocked yt-dlp output to parse");

    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].title, "Track One");
    assert_eq!(entries[0].url, "https://example.com/one");
    assert_eq!(entries[1].title, "Track Two");
    assert_eq!(entries[1].url, "https://example.com/two");
    assert_eq!(entries[2].title, "Track Three");
    assert_eq!(
        entries[2].url,
        "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
    );
}
