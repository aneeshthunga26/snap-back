# Contributing

Snap Back is still a prototype, and reports from other CachyOS setups are useful. Start with an issue if a change affects backup layout, retention, drive setup, or system recovery; those choices affect existing backups.

## Development environment

Use a disposable CachyOS VM with Btrfs root and home subvolumes. Add a separate virtual disk for backups. Full recovery currently needs Limine, mkinitcpio, and an EFI partition mounted at `/boot`.

The workspace has three crates:

- `snap-back-core`: snapshots, transfers, drive setup, retention, and recovery.
- `snap-back-cli`: the JSON command interface and privileged helper.
- `snap-back-gui`: the Iced desktop application.

Install the build dependencies in [docs/INSTALL.md](docs/INSTALL.md), then run these checks **inside the VM**:

```sh
cargo fmt --all --check
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo build --locked --release --workspace
```

Keep destructive integration testing separate from unit tests. The scripts in `tests/` create real snapshots, marker files, and backup transfers; some recovery checks are intended to follow a destructive reboot. They are guarded for the development VM and require root. Read each script before running it. Never point them at a system containing data you need to keep.

[docs/VALIDATION.md](docs/VALIDATION.md) records the checks already run. [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) contains the original VM setup notes and CLI examples.

## Changes and reviews

Keep a change focused and explain the behavior it fixes. For GUI changes, include a screenshot from the VM. For backup or restore changes, describe what happens if a drive disappears, a transfer fails, or the process is interrupted.

Tests should cover behavior or a failure case, rather than repeat how the implementation is written. Do not add automatic formatting or destructive restore tests to ordinary CI jobs.

CI checks pull requests on GitHub-hosted runners using an Arch Linux container. It runs unit tests and packaging checks; destructive backup and recovery testing stays in a disposable VM. See [docs/CI.md](docs/CI.md).

By contributing, you agree that your changes are licensed under GPL-3.0-only, the same license as the project.
