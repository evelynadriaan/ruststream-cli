# Contributing

Thanks for your interest. This is a personal project but contributions are welcome.

## Build locally

**Requirements:** Rust nightly, `ffmpeg`, `yt-dlp`, `mpv`, `pkg-config`, `libasound2-dev`, `libdbus-1-dev`

```bash
git clone https://github.com/evelynadriaan/ruststream-cli
cd ruststream-cli
cargo build
```

Run the binary:
```bash
./target/debug/clistream
```

## Before opening a PR

```bash
cargo fmt
cargo clippy -- -D warnings
cargo test
```

All three must pass. CI will enforce this.

## Scope

Check the [ROADMAP](ROADMAP.md) and open issues before starting large work — avoid building something already in progress or explicitly listed as a non-goal.

For bugs: open an issue first with reproduction steps.
For features: open an issue first to discuss scope.

## Commit style

Plain English, imperative mood: `fix: ...`, `feat: ...`, `docs: ...`, `chore: ...`

## License

By contributing you agree your changes are licensed under MIT.
