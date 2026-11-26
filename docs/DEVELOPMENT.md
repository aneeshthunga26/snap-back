# Snap Back

A Rust desktop backup prototype for CachyOS. A quiet, dark interface built with Iced and iced-shadcn, backed by real Snapper snapshots and Btrfs send/receive.

Open **Snap Back** from the application launcher inside `cachy-vm`.

- **Back up now**, live operation status, disk capacity, and a restore-point timeline.
- A radiating band travels from the fixed backup symbol toward a rotating outer ring during operations, with subtle navigation and activity feedback using `iced_anim`. Motion settles at rest.
- Integrated window controls use the desktop's symbolic icon theme and KDE/GNOME button layout, with subtle rounded corners and no separate system title bar.
- A graphical backup-drive picker with saved-drive switching, automatic mounting, and explicit whole-disk erase confirmation for new drives.
- A split **Back up now** button with a checkbox menu for backing up to several saved drives in one run. The last used selection also receives hourly backups.
- Root and home, all configured mounted system subvolumes, and nested machine/portable subvolumes. Backups continue when the source layout changes and list any detected subvolumes outside the backup configuration.
- First full transfer, then incremental transfers to an independently mounted Btrfs drive. Completed points are read-only and verified against the source snapshot UUID.
- Automatic hourly backups while running; retention keeps the newest point in 24 occupied hourly, 7 daily, and 4 weekly buckets. Multiple backups within a bucket coalesce. Local Snapper configuration and cleanup remain separate from backup-disk retention.
- Browse root/home snapshots and recover a file or folder into a new directory in your home. The previous listing stays visible while another folder or restore point loads. Existing destinations are refused.
- Prepare a full system recovery, restart into a dedicated Limine recovery entry, and restore the complete configured subvolume set before mounting root. The previous system remains in rollback subvolumes.

## Installed VM

The dedicated 160 GiB sparse virtual disk is attached persistently as `vdb`, formatted Btrfs with label `SNAPBACK`, and mounted by UUID at `/backup/snap-back`. A second 64 GiB backup disk is attached as `vdc`. The VM system disk was expanded to 100 GiB to provide room for recovery staging. The VM disk images are on the host's `/backup/snap-back-vm/` filesystem, which has sufficient free space; the original host image was preserved.

| Component | Location inside VM |
| --- | --- |
| GUI | `/usr/local/bin/snap-back-gui` |
| Privileged Rust helper | `/usr/local/bin/snap-back` |
| Configuration | `/etc/snap-back/config.json` |
| Restore points | Each saved drive's `points/` directory |
| Local incremental parents | `/var/lib/snap-back/anchors/by-drive/UUID/` |
| Scheduler | `snap-back.timer` / `snap-back.service` |
| Recovery control and last journal | `/boot/snap-back/` |
| VM validation evidence | `/backup/snap-back/.validation/` |

The GUI runs as your user and uses PolicyKit for administrator operations. It never runs a shell with entered file paths. A reboot is a separate action after recovery preparation; preparation can be cancelled.

## Build and install

All development verification for this prototype takes place **inside `cachy-vm`**. No host builds or tests are required.

```sh
# Inside the VM, from this checkout:
export CARGO_TARGET_DIR=/backup/snap-back/.build
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
sudo env CARGO_TARGET_DIR="$CARGO_TARGET_DIR" SNAP_BACK_PROFILE=debug \
  bash packaging/install.sh /backup/snap-back
```

## Choose or add a backup drive

In **Settings → Backup storage → Choose backup drive** (also available from Overview), choose either:

- **Browse backups** for a saved backup drive. This mounts it if needed and switches the restore browser to its backups. It keeps the backup destination selection unchanged. No data is erased.
- **Set up drive** for a new, unmounted whole disk. Review its device path, size, model, and serial. The red warning explains that **all existing data, including every partition, will be erased**. Type the exact device-specific phrase, such as `ERASE /dev/vdc`, then choose **Erase and use for backups**. Cancel before that action leaves the disk unchanged.

Setup formats the selected disk as Btrfs, mounts it by UUID, adds automatic mounting, saves the drive, and adds it to the selected backup destinations. Other selected drives and their existing points are kept. System disks, saved backup disks, read-only/in-use disks, and disks with mounted partitions cannot be erased through this flow. Unmount a new disk in your file manager before setup. Administrator authentication is required.

This performs filesystem formatting, not a secure wipe. The confirmation expires after ten minutes and the helper checks physical disk identity again before writing, so a stale device selection must be reviewed again.

Both VM backup drives are selected as backup destinations. The 64 GiB drive is currently shown in the restore browser.

Use the arrow beside **Back up now** to check the saved drives to include. Clicking **Back up now** remembers that selection for hourly backups. Clearing every checkbox disables the backup action. Disconnected drives keep their checkmarks; if one selected drive fails, the other selected drives are still attempted and the result names the failures and completed drives.

Drives are processed one after another, each with its own snapshot checkpoint, incremental parent, and retention. Browsing or recovering from another drive does not change where backups are written.

For command-line installation on an already formatted disk, `snap-back configure --destination /mount/path` still discovers sources and reuses Snapper configs. It refuses to overwrite an existing configuration. The graphical setup flow handles formatting and mounting; `packaging/install.sh` itself never formats a disk.

Runtime dependencies: `btrfs-progs`, `snapper`, `rsync`, GNU coreutils, util-linux, systemd, polkit. Full recovery currently targets the CachyOS layout with Btrfs root `@`, a separate home subvolume, a FAT EFI partition mounted at `/boot`, Limine, and mkinitcpio. This is a working prototype for that layout.

## CLI

```sh
sudo snap-back status
sudo snap-back backup --label 'Before an upgrade'
sudo snap-back backup --drive DRIVE_UUID_A --drive DRIVE_UUID_B --save-targets
sudo snap-back set-backup-drives --drive DRIVE_UUID_A --drive DRIVE_UUID_B
sudo snap-back browse POINT home aneesh/Documents
sudo snap-back restore-file POINT home aneesh/Documents/report.txt \
  --to /home/aneesh/Recovered-report
sudo snap-back prepare-recovery POINT
sudo snap-back cancel-recovery
sudo snap-back schedule on
```

CLI results are JSON; errors go to stderr. File recovery copies the selected item inside the newly created destination directory. Full recovery staging requires free space on the source filesystem for received copies. The first backup after a full system restore can be a full transfer while local incremental parents are rebuilt. Old subvolumes remain under `.snap-back-restore-POINT/before/` at filesystem top level, and are intentionally not automatically deleted. Back up and inspect them before manual cleanup.

Without `--drive`, backup uses the saved destinations. Repeat `--drive` for a one-run selection; add `--save-targets` to remember it. `set-backup-drives` changes the hourly defaults without starting a backup. `use-drive UUID` chooses the drive whose points are browsed or restored and keeps those defaults unchanged.

## Recovery behavior and scope

Snapshots are filesystem-consistent and taken sequentially across subvolumes. Quiesce databases or other transactional applications when an application-consistent checkpoint is required. Root and home contents are backed up, including dotfiles and permissions; Snapper's historical `.snapshots` trees are managed locally and preserved across recovery rather than recursively backed up. `/boot` is copied with each restore point to keep kernels and modules aligned. Kernel and bootloader installation to a completely blank replacement disk is outside this prototype's recovery flow.

A backup disk disconnected or replaced with another UUID is refused. Failed transfers stay invisible to the timeline and are cleaned up safely. Mutating operations share a lock. File recovery rejects traversal and symlink escapes, does not recreate device nodes, and strips setuid/setgid permissions when returning files to the user.

The VM disks currently reside on the same host storage filesystem; they isolate VM disk contents but do not protect against loss of that host filesystem. Encryption was not enabled, as requested.

## Source layout

- `crates/snap-back-core`: backup, Snapper discovery, retention, safe file recovery, offline recovery.
- `crates/snap-back-cli`: JSON command interface and privileged helper.
- `crates/snap-back-gui`: Iced/shadcn desktop interface.
- `packaging`: desktop entry, PolicyKit action, systemd timer, installer, recovery initcpio hooks.
- `tests/vm_smoke.py`: VM-only real Btrfs integration checks; does not reboot by itself.
- `tests/vm_verify_recovery.py`: VM-only verification after the destructive recovery reboot.
- `docs/VALIDATION.md`: verified results and recovery evidence.

The final product and copy review is recorded in [docs/UX-REVIEW.md](docs/UX-REVIEW.md), with refreshed VM screenshots in [docs/screenshots](docs/screenshots).

Drive setup safety and VM tests are recorded in [docs/DRIVE-SETUP.md](docs/DRIVE-SETUP.md).

Desktop window integration and its scope are described in [docs/WINDOW-CONTROLS.md](docs/WINDOW-CONTROLS.md).
