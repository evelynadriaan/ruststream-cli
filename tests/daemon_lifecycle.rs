use assert_cmd::Command;
use predicates::str::contains;
use std::fs;
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

#[test]
fn daemon_start_status_stop_cycle() {
    let (_temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");

    let mut start = Command::new(bin);
    start
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .args(["daemon", "start"])
        .assert()
        .success()
        .stdout(contains("Daemon started"));

    let mut status_running = Command::new(bin);
    status_running
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .args(["daemon", "status"])
        .assert()
        .success()
        .stdout(contains("Daemon is running"));

    let mut stop = Command::new(bin);
    stop.env("XDG_DATA_HOME", &data_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .args(["daemon", "stop"])
        .assert()
        .success()
        .stdout(contains("Daemon stopped"));

    let mut status_stopped = Command::new(bin);
    status_stopped
        .env("XDG_DATA_HOME", &data_home)
        .env("XDG_CONFIG_HOME", &config_home)
        .env("XDG_RUNTIME_DIR", &runtime_dir)
        .args(["daemon", "status"])
        .assert()
        .success()
        .stdout(contains("Daemon is not running"));
}
