# Security

## Data & privacy

- All audio files are stored locally at `~/.local/share/clistream/audio/`
- All metadata (tracks, playlists, listen history) is stored in `~/.local/share/clistream/clistream.db`
- Config is stored at `~/.config/clistream/config.toml`
- **Nothing ever leaves your machine** except outbound requests to YouTube (via yt-dlp) and lrclib.net (lyrics). No telemetry, no analytics, no accounts.
- The daemon communicates via a local Unix socket at `$XDG_RUNTIME_DIR/clistream/clistream.sock`

## Reporting a vulnerability

If you find a security issue, please open a [GitHub Issue](https://github.com/evelynadriaan/ruststream-cli/issues) marked with the `security` label, or contact the maintainer directly via GitHub.

Please do not publicly disclose the details until it has been acknowledged.
