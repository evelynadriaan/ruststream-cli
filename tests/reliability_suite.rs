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

fn cmd(bin: &str, data_home: &str, config_home: &str, runtime_dir: &str) -> Command {
    let mut c = Command::new(bin);
    c.env("XDG_DATA_HOME", data_home)
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_RUNTIME_DIR", runtime_dir);
    c
}

#[test]
fn repeated_daemon_start_stop_cycles_are_stable() {
    let (_temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");

    for _ in 0..3 {
        cmd(bin, &data_home, &config_home, &runtime_dir)
            .args(["daemon", "start"])
            .assert()
            .success()
            .stdout(contains("Daemon started"));

        cmd(bin, &data_home, &config_home, &runtime_dir)
            .args(["daemon", "status"])
            .assert()
            .success()
            .stdout(contains("Daemon is running"));

        cmd(bin, &data_home, &config_home, &runtime_dir)
            .args(["daemon", "stop"])
            .assert()
            .success()
            .stdout(contains("Daemon stopped"));

        cmd(bin, &data_home, &config_home, &runtime_dir)
            .args(["daemon", "status"])
            .assert()
            .success()
            .stdout(contains("Daemon is not running"));
    }
}

#[test]
fn stale_socket_and_pid_are_cleaned_on_start() {
    let (_temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");

    let stale_socket_dir = std::path::Path::new(&runtime_dir).join("clistream");
    fs::create_dir_all(&stale_socket_dir).expect("failed to create stale socket dir");
    let stale_socket = stale_socket_dir.join("clistream.sock");
    fs::write(&stale_socket, b"stale-file").expect("failed to write stale socket placeholder");

    let stale_pid_dir = std::path::Path::new(&data_home).join("clistream");
    fs::create_dir_all(&stale_pid_dir).expect("failed to create stale pid dir");
    let stale_pid = stale_pid_dir.join("clistream.pid");
    fs::write(&stale_pid, "999999").expect("failed to write stale pid file");

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["daemon", "start"])
        .assert()
        .success()
        .stdout(contains("Daemon started"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["daemon", "status"])
        .assert()
        .success()
        .stdout(contains("Daemon is running"));

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["daemon", "stop"])
        .assert()
        .success()
        .stdout(contains("Daemon stopped"));
}

#[test]
fn download_failures_surface_as_typed_errors() {
    let (_temp, data_home, config_home, runtime_dir) = setup_env();
    let bin = env!("CARGO_BIN_EXE_clistream");

    cmd(bin, &data_home, &config_home, &runtime_dir)
        .args(["add", "not-a-url"])
        .assert()
        .failure()
        .stderr(contains("yt-dlp get_video_info failed"));
}
