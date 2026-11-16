# Install Snap Back

Snap Back currently targets CachyOS on x86_64, with Btrfs root and home subvolumes, Snapper, systemd, and PolicyKit. Full system recovery also needs Limine, mkinitcpio, and an EFI partition mounted at `/boot`. Other distributions and boot layouts have not been verified.

## Binary release

Download the Linux x86_64 archive and `SHA256SUMS` from the project's GitHub Releases page. The matching source archive is available beside it, including the locked Rust dependency sources for an offline build.

```sh
sha256sum --ignore-missing --check SHA256SUMS
tar -xzf snap-back-0.1.2-linux-x86_64.tar.gz
cd snap-back-0.1.2-linux-x86_64
sudo ./install.sh
```

The installer copies the application, PolicyKit action, desktop launcher, recovery files, and hourly systemd timer. It keeps an existing configuration. On a fresh installation, open **Snap Back** from your application launcher and choose a backup drive in Settings. Setting up a new drive erases its existing contents; the application asks for the exact device-specific confirmation before formatting it.

Install the required system tools first:

```sh
sudo pacman -S --needed btrfs-progs snapper rsync util-linux polkit
```

The desktop also needs a working OpenGL or Vulkan driver, a PolicyKit authentication agent, and the normal Wayland or X11 libraries. Snap Back uses desktop portals when available. The binary archive is built for CachyOS/Arch Linux with glibc; it is not a static binary for every Linux distribution. `BUILD-INFO` records the compiler, libc, and source revision used for that archive.

The timer starts automatically. Until a drive is configured, systemd skips the backup service. Subsequent hourly backups use the drive selection saved by your last **Back up now** action. You can pause automatic backups in Settings.

## Build from source

Install Rust and the development tools used by the GUI:

```sh
sudo pacman -S --needed rust base-devel pkgconf libxkbcommon wayland
cargo build --locked --release --workspace
```

Use the binary installer from a generated archive, or use the existing source installer with an already mounted Btrfs backup destination:

```sh
sudo bash packaging/install.sh /path/to/mounted-backup-drive
```

The source installer configures that destination on a fresh installation. If you want to choose or format a drive in the GUI instead, generate a binary archive and run its `install.sh`.

## Build release archives

```sh
packaging/release.sh v0.1.2
```

This builds both Rust binaries with `--locked --release`, checks the version against all workspace packages, and writes binary/source tarballs and `SHA256SUMS` into `dist/`. It respects `CARGO_TARGET_DIR`. To package binaries you just built, add `--skip-build`. Packaging also requires Python 3.11 or newer, Git, GNU tar, and gzip.

The source tarball includes `vendor/` and `.cargo/config.toml`, so it can be built with `cargo build --offline --locked --release --workspace`. Both archives contain `THIRD-PARTY-NOTICES.txt` with the bundled dependencies' license notices.

Archive file ordering, owner IDs, timestamps, and gzip headers are normalized. `SOURCE_DATE_EPOCH` overrides the archive timestamp; otherwise the latest commit's timestamp is used. `SOURCE_REVISION` can supply a commit ID when packaging a checkout without Git metadata. `BUILD-INFO` identifies an unversioned or modified source tree. Published releases should be built from the matching clean tag.

For a package staging directory, use:

```sh
DESTDIR=/absolute/staging/root ./install.sh
```

Staging copies files without invoking systemd. Development builds, tests, archive checks, and installer staging for this project are performed inside `cachy-vm`.
