use std::sync::mpsc::Receiver;

use discord_rich_presence::{DiscordIpc, DiscordIpcClient, activity};
use tracing::warn;

#[derive(Debug, Clone)]
pub enum PresenceUpdate {
    Playing {
        title: String,
        artist: Option<String>,
    },
    Paused {
        title: String,
    },
    Stopped,
}

pub fn run(rx: Receiver<PresenceUpdate>) {
    let mut client = match DiscordIpcClient::new("1234567890123456789") {
        Ok(client) => client,
        Err(err) => {
            warn!(error = %err, "Failed to initialize Discord IPC client");
            return;
        }
    };

    if let Err(err) = client.connect() {
        warn!(
            error = %err,
            "Discord Rich Presence unavailable (Discord may not be running)"
        );
        return;
    }

    while let Ok(update) = rx.recv() {
        match update {
            PresenceUpdate::Playing { title, artist } => {
                let mut assets = activity::Assets::new();
                if let Some(artist) = artist.as_deref() {
                    assets = assets.small_text(artist);
                }

                if let Err(err) = client.set_activity(
                    activity::Activity::new()
                        .state("Playing")
                        .details(&title)
                        .assets(assets),
                ) {
                    warn!(error = %err, "Failed to set Discord activity to playing");
                }
            }
            PresenceUpdate::Paused { title } => {
                if let Err(err) =
                    client.set_activity(activity::Activity::new().state("Paused").details(&title))
                {
                    warn!(error = %err, "Failed to set Discord activity to paused");
                }
            }
            PresenceUpdate::Stopped => {
                if let Err(err) = client.clear_activity() {
                    warn!(error = %err, "Failed to clear Discord activity");
                }
            }
        }
    }

    if let Err(err) = client.close() {
        warn!(error = %err, "Failed to close Discord IPC client");
    }
}
