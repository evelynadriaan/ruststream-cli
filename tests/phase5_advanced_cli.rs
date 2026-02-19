use assert_cmd::Command;
use chrono::Utc;
use predicates::str::contains;
use rusqlite::{Connection, params};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;
use uuid::Uuid;

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

fn cmd(bin: &str, data_home: &str, config_home: &str, runtime_dir: &str) -> Command {
    let mut c = Command::new(bin);
    c.env("XDG_DATA_HOME", data_home)
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_RUNTIME_DIR", runtime_dir);
    c
}

fn initialize_library(bin: &str, data_home: &str, config_home: &str, runtime_dir: &str) -> PathBuf {
    cmd(bin, data_home, config_home, runtime_dir)
        .arg("list")
        .assert()
        .success();

    Path::new(data_home).join("clistream").join("clistream.db")
}

fn seed_tracks(db_path: &Path, titles: &[&str]) {
    let conn = Connection::open(db_path).expect("failed to open db");
    for (i, title) in titles.iter().enumerate() {
        conn.execute(
            "INSERT INTO tracks (id, url, title, alias, duration, added_at, file_path, available)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                Uuid::new_v4().to_string(),
                format!("https://youtube.com/watch?v=phase5{i}"),
                title,
                Option::<String>::None,
                120_i64 + i as i64,
                Utc::now().to_rfc3339(),
                format!("/tmp/phase5-{i}.mp3"),
                1_i64
            ],
        )
        .expect("failed to insert test track");
    }
}

#[test]
fn help_documents_phase5_commands() {
    let bin = env!("CARGO_BIN_EXE_clistream");
    Command::new(bin)
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("next"))
        .stdout(contains("prev"))
        .stdout(contains("queue"))
        .stdout(contains("shuffle"))
        .stdout(contains("repeat"))
        .stdout(contains("playlist"));
}

#[test]
fn next_prev_commands_work_with_library_queue() {
    let (_temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");
    let db_path = initialize_library(bin, &data_home, &config_home, &runtime_dir);
    seed_tracks(&db_path, &["Track One", "Track Two", "Track Three"]);

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["daemon", "start"])
        .assert()
        .success();

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["play", "Track Two"])
        .assert()
        .success()
        .stdout(contains("Playing: Track Two"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .arg("next")
        .assert()
        .success()
        .stdout(contains("Skipped to next track."));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .arg("prev")
        .assert()
        .success()
        .stdout(contains("Returned to previous track."));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["daemon", "stop"])
        .assert()
        .success();
}

#[test]
fn queue_commands_work() {
    let (_temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");
    let db_path = initialize_library(bin, &data_home, &config_home, &runtime_dir);
    seed_tracks(&db_path, &["Queue Alpha", "Queue Beta"]);

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["daemon", "start"])
        .assert()
        .success();

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["queue", "add", "Queue Alpha"])
        .assert()
        .success()
        .stdout(contains("Added to queue: Queue Alpha"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["queue", "add", "Queue Beta"])
        .assert()
        .success()
        .stdout(contains("Added to queue: Queue Beta"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["queue", "list"])
        .assert()
        .success()
        .stdout(contains("Queue (2 tracks):"))
        .stdout(contains("Queue Alpha"))
        .stdout(contains("Queue Beta"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["queue", "clear"])
        .assert()
        .success()
        .stdout(contains("Queue cleared."));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["queue", "list"])
        .assert()
        .success()
        .stdout(contains("Queue is empty."));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["daemon", "stop"])
        .assert()
        .success();
}

#[test]
fn shuffle_commands_work() {
    let (_temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");
    let db_path = initialize_library(bin, &data_home, &config_home, &runtime_dir);
    seed_tracks(&db_path, &["State Track"]);

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["daemon", "start"])
        .assert()
        .success();

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["shuffle", "on"])
        .assert()
        .success()
        .stdout(contains("Shuffle: on"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .arg("shuffle")
        .assert()
        .success()
        .stdout(contains("Shuffle: on"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["shuffle", "off"])
        .assert()
        .success()
        .stdout(contains("Shuffle: off"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .arg("shuffle")
        .assert()
        .success()
        .stdout(contains("Shuffle: off"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["daemon", "stop"])
        .assert()
        .success();
}

#[test]
fn repeat_commands_work() {
    let (_temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");
    let db_path = initialize_library(bin, &data_home, &config_home, &runtime_dir);
    seed_tracks(&db_path, &["Repeat Track"]);

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["daemon", "start"])
        .assert()
        .success();

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["repeat", "one"])
        .assert()
        .success()
        .stdout(contains("Repeat: one"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .arg("repeat")
        .assert()
        .success()
        .stdout(contains("Repeat: one"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["repeat", "all"])
        .assert()
        .success()
        .stdout(contains("Repeat: all"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["repeat", "off"])
        .assert()
        .success()
        .stdout(contains("Repeat: off"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .arg("repeat")
        .assert()
        .success()
        .stdout(contains("Repeat: off"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["daemon", "stop"])
        .assert()
        .success();
}

#[test]
fn playlist_commands_work() {
    let (_temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");
    let db_path = initialize_library(bin, &data_home, &config_home, &runtime_dir);
    seed_tracks(&db_path, &["Playlist Song"]);

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["playlist", "create", "Roadtrip"])
        .assert()
        .success()
        .stdout(contains("Created playlist: Roadtrip"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["playlist", "add", "Roadtrip", "Playlist Song"])
        .assert()
        .success()
        .stdout(contains("Added 'Playlist Song' to playlist 'Roadtrip'."));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["playlist", "list"])
        .assert()
        .success()
        .stdout(contains("Roadtrip (1 tracks)"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["playlist", "remove", "Roadtrip", "Playlist Song"])
        .assert()
        .success()
        .stdout(contains(
            "Removed 'Playlist Song' from playlist 'Roadtrip'.",
        ));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["playlist", "list"])
        .assert()
        .success()
        .stdout(contains("Roadtrip (0 tracks)"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["playlist", "delete", "Roadtrip"])
        .assert()
        .success()
        .stdout(contains("Deleted playlist: Roadtrip"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["playlist", "list"])
        .assert()
        .success()
        .stdout(contains("No playlists found."));
}
