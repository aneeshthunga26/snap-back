# Changelog

## 0.1.0

First public prototype for CachyOS.

- Rust/Iced desktop interface with backup status, restore-point browsing, and drive setup.
- Btrfs send/receive backups of root, home, configured system subvolumes, and boot files.
- Multiple backup destinations, independent incremental history, and hourly retention.
- File and folder recovery into new directories.
- Offline system recovery through Limine and mkinitcpio, retaining the previous volumes.
- Explicit whole-disk erase confirmation when setting up a new backup drive.
- Linux x86_64 release archives, matching source, dependency notices, and checksums.

This release targets the tested CachyOS layout described in the README. It does not provision a blank replacement system disk.
