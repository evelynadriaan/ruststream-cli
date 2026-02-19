use assert_cmd::Command;
use predicates::str::contains;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use tempfile::TempDir;

fn setup_env() -> (TempDir, String, String, String) {
    let temp = tempfile::tempdir().expect("failed to create temp dir");
    let data_home = temp.path().join("data");
    let config_home = temp.path().join("config");
    let runtime_dir = temp.path().join("run");

    fs::create_dir_all(&data_home).expect("failed to create data dir");
    fs::create_dir_all(&config_home).expect("failed to create config dir");
    fs::create_dir_all(&runtime_dir).expect("failed to create runtime dir");

    (
        temp,
        data_home.to_string_lossy().to_string(),
        config_home.to_string_lossy().to_string(),
        runtime_dir.to_string_lossy().to_string(),
    )
}

fn cmd(
    bin: &str,
    data_home: &str,
    config_home: &str,
    runtime_dir: &str,
    path_override: Option<&str>,
) -> Command {
    let mut c = Command::new(bin);
    c.env("XDG_DATA_HOME", data_home)
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_RUNTIME_DIR", runtime_dir);
    if let Some(path_override) = path_override {
        c.env("PATH", path_override);
    }
    c
}

fn has_binary(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn initialize_library(bin: &str, data_home: &str, config_home: &str, runtime_dir: &str) {
    cmd(bin, data_home, config_home, runtime_dir, None)
        .arg("list")
        .assert()
        .success();
}

fn write_executable(path: &Path, contents: &str) {
    fs::write(path, contents).expect("failed to write executable");
    let mut perms = fs::metadata(path)
        .expect("failed to stat executable")
        .permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).expect("failed to set executable permissions");
}

fn setup_fake_downloader(temp: &TempDir) -> String {
    let fake_bin = temp.path().join("fake-bin");
    fs::create_dir_all(&fake_bin).expect("failed to create fake-bin directory");

    let yt_dlp_path = fake_bin.join("yt-dlp");
    write_executable(
        &yt_dlp_path,
        r#"#!/bin/sh
set -eu

last=""
for arg in "$@"; do
  last="$arg"
done

for arg in "$@"; do
  if [ "$arg" = "--version" ]; then
    echo "yt-dlp 2026.01.01"
    exit 0
  fi
done

for arg in "$@"; do
  if [ "$arg" = "--dump-json" ]; then
    printf '{"id":"mock","title":"Mock Stream Track","duration":19,"webpage_url":"%s"}\n' "$last"
    exit 0
  fi
done

for arg in "$@"; do
  if [ "$arg" = "--simulate" ]; then
    exit 0
  fi
done

output_template=""
prev=""
for arg in "$@"; do
  if [ "$prev" = "-o" ]; then
    output_template="$arg"
  fi
  prev="$arg"
done

if [ -z "$output_template" ]; then
  echo "missing output template" >&2
  exit 1
fi

out_file="$(printf '%s' "$output_template" | sed 's/%(ext)s/mp3/g')"
mkdir -p "$(dirname "$out_file")"
printf 'fake-audio' > "$out_file"
echo "POSTPROCESS" >&2
printf '%s\n' "$out_file"
"#,
    );

    let ffmpeg_path = fake_bin.join("ffmpeg");
    write_executable(
        &ffmpeg_path,
        r#"#!/bin/sh
set -eu
if [ "${1:-}" = "-version" ]; then
  echo "ffmpeg version fake"
fi
exit 0
"#,
    );

    let system_path = std::env::var("PATH").unwrap_or_default();
    format!("{}:{}", fake_bin.display(), system_path)
}

fn generate_tone_wav(path: &Path, duration_secs: u32) {
    let status = std::process::Command::new("ffmpeg")
        .args([
            "-f",
            "lavfi",
            "-i",
            &format!("sine=frequency=1000:duration={duration_secs}"),
            "-acodec",
            "pcm_s16le",
            "-y",
            "-loglevel",
            "error",
        ])
        .arg(path)
        .status()
        .expect("failed to spawn ffmpeg for test audio generation");

    assert!(status.success(), "ffmpeg failed to generate test audio");
}

#[test]
fn stream_and_save_downloads_in_background() {
    if !has_binary("mpv") || !has_binary("ffmpeg") {
        return;
    }

    let (temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");
    initialize_library(bin, &data_home, &config_home, &runtime_dir);

    let stream_file = temp.path().join("stream.wav");
    generate_tone_wav(&stream_file, 12);

    let path_env = setup_fake_downloader(&temp);

    cmd(bin, &data_home, &config_home, &runtime_dir, Some(&path_env))
        .args(["daemon", "start"])
        .assert()
        .success();

    cmd(bin, &data_home, &config_home, &runtime_dir, Some(&path_env))
        .arg("stream")
        .arg(stream_file.to_string_lossy().to_string())
        .assert()
        .success()
        .stdout(contains("Streaming:"));

    cmd(bin, &data_home, &config_home, &runtime_dir, Some(&path_env))
        .arg("status")
        .assert()
        .success()
        .stdout(contains("Playing:"));

    cmd(bin, &data_home, &config_home, &runtime_dir, Some(&path_env))
        .arg("save")
        .assert()
        .success()
        .stdout(contains("Saving current stream in background."));

    cmd(bin, &data_home, &config_home, &runtime_dir, Some(&path_env))
        .arg("status")
        .assert()
        .success()
        .stdout(contains("Playing:"));

    let mut saved = false;
    for _ in 0..40 {
        let output = cmd(bin, &data_home, &config_home, &runtime_dir, Some(&path_env))
            .arg("list")
            .output()
            .expect("failed to run list command");
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("Mock Stream Track") {
            saved = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    assert!(saved, "expected saved stream track to appear in library");

    cmd(bin, &data_home, &config_home, &runtime_dir, Some(&path_env))
        .args(["daemon", "stop"])
        .assert()
        .success();
}

#[test]
fn stream_playlist_command_uses_mpv_backend() {
    if !has_binary("mpv") || !has_binary("ffmpeg") {
        return;
    }

    let (temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");
    initialize_library(bin, &data_home, &config_home, &runtime_dir);

    let tone1 = temp.path().join("p1.wav");
    let tone2 = temp.path().join("p2.wav");
    generate_tone_wav(&tone1, 4);
    generate_tone_wav(&tone2, 4);

    let playlist_file = temp.path().join("playlist.m3u");
    let playlist = format!("{}\n{}\n", tone1.to_string_lossy(), tone2.to_string_lossy());
    fs::write(&playlist_file, playlist).expect("failed to write test playlist");

    cmd(bin, &data_home, &config_home, &runtime_dir, None)
        .args(["daemon", "start"])
        .assert()
        .success();

    cmd(bin, &data_home, &config_home, &runtime_dir, None)
        .arg("stream")
        .arg(playlist_file.to_string_lossy().to_string())
        .assert()
        .success()
        .stdout(contains("Streaming playlist:"));

    cmd(bin, &data_home, &config_home, &runtime_dir, None)
        .arg("status")
        .assert()
        .success()
        .stdout(contains("Playing:"));

    cmd(bin, &data_home, &config_home, &runtime_dir, None)
        .args(["daemon", "stop"])
        .assert()
        .success();
}

#[test]
fn stream_and_save_fail_with_mpv_unavailable() {
    let (temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");
    initialize_library(bin, &data_home, &config_home, &runtime_dir);

    let no_mpv_path = temp.path().join("no-mpv-bin");
    fs::create_dir_all(&no_mpv_path).expect("failed to create no-mpv path");
    let no_mpv_path = no_mpv_path.to_string_lossy().to_string();

    cmd(
        bin,
        &data_home,
        &config_home,
        &runtime_dir,
        Some(&no_mpv_path),
    )
    .arg("stream")
    .arg("https://www.youtube.com/watch?v=jNQXAC9IVRw")
    .assert()
    .failure()
    .stderr(contains("MPV_UNAVAILABLE"));

    cmd(
        bin,
        &data_home,
        &config_home,
        &runtime_dir,
        Some(&no_mpv_path),
    )
    .arg("save")
    .assert()
    .failure()
    .stderr(contains("MPV_UNAVAILABLE"));

    let _ = cmd(
        bin,
        &data_home,
        &config_home,
        &runtime_dir,
        Some(&no_mpv_path),
    )
    .args(["daemon", "stop"])
    .output();
}
