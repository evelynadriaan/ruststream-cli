use anyhow::Result;

/// Run the macOS media key listener using MediaPlayer framework.
/// macOS support is best-effort compile only, not a release target.
#[allow(dead_code)]
pub fn run_media_key_listener() -> Result<()> {
    // Note: Full implementation requires Objective-C runtime interaction
    // via the MediaPlayer framework (MPRemoteCommandCenter, MPNowPlayingInfoCenter)
    //
    // For a complete implementation, we would need to:
    // 1. Register with MPRemoteCommandCenter for play/pause/next/prev commands
    // 2. Update MPNowPlayingInfoCenter with track metadata
    // 3. Handle the command callbacks
    //
    // This is a placeholder that can be expanded with full objc2 bindings
    // The complexity of the MediaPlayer framework integration is significant

    tracing::info!("macOS media key listener initialized (best-effort compile path)");

    // Keep thread alive
    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

/// Update Now Playing info on macOS (best-effort compile path)
#[allow(dead_code)]
pub fn update_now_playing(_title: &str, _artist: Option<&str>, _duration: u64) {
    // Would update MPNowPlayingInfoCenter here
}
