use assert_cmd::Command;
use chrono::Utc;
use predicates::str::contains;
use rusqlite::Connection;
use std::fs;
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

#[test]
fn no_subcommand_fails_in_non_interactive_mode() {
    let (_temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");

    let mut cmd = Command::new(bin);
    cmd.env("XDG_DATA_HOME", &data_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .assert()
        .failure()
        .stderr(contains("No subcommand provided in non-interactive mode"));
}

#[test]
fn remove_command_deletes_track_file_and_metadata() {
    let (_temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");

    // Initialize storage + DB schema.
    let mut list_cmd = Command::new(bin);
    list_cmd
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .arg("list")
        .assert()
        .success();

    let app_data = std::path::Path::new(&data_home).join("clistream");
    let audio_dir = app_data.join("audio");
    let db_path = app_data.join("clistream.db");
    fs::create_dir_all(&audio_dir).expect("failed to create audio dir");

    let file_path = audio_dir.join("remove-me.mp3");
    fs::write(&file_path, b"dummy-audio").expect("failed to create test audio file");

    let conn = Connection::open(&db_path).expect("failed to open db");
    conn.execute(
        "INSERT INTO tracks (id, url, title, alias, duration, added_at, file_path, available)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            Uuid::new_v4().to_string(),
            "https://youtube.com/watch?v=phase2remove",
            "Phase2 Remove Track",
            Option::<String>::None,
            12_i64,
            Utc::now().to_rfc3339(),
            file_path.to_string_lossy().to_string(),
            1_i64
        ],
    )
    .expect("failed to insert test track");

    let mut remove_cmd = Command::new(bin);
    remove_cmd
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .args(["remove", "Phase2 Remove Track"])
        .assert()
        .success()
        .stdout(contains("Removed: Phase2 Remove Track"));

    assert!(
        !file_path.exists(),
        "remove command should delete the audio file"
    );

    let remaining: i64 = conn
        .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
        .expect("failed to count tracks");
    assert_eq!(remaining, 0, "remove command should delete db metadata");
}
