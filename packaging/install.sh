#!/usr/bin/env bash
# Run INSIDE CachyOS after building there. Never formats a device.
set -euo pipefail
[[ $EUID == 0 ]] || { echo 'Run this installer as root.' >&2; exit 1; }
cd "$(dirname "$0")/.."
: "${CARGO_TARGET_DIR:=target}"
: "${SNAP_BACK_PROFILE:=release}"
install -m755 "$CARGO_TARGET_DIR/$SNAP_BACK_PROFILE/snap-back" /usr/local/bin/snap-back
install -m755 "$CARGO_TARGET_DIR/$SNAP_BACK_PROFILE/snap-back-gui" /usr/local/bin/snap-back-gui
install -m644 packaging/snap-back.service packaging/snap-back.timer /etc/systemd/system/
install -m644 packaging/org.snapback.manage.policy /usr/share/polkit-1/actions/
install -m644 packaging/snap-back.desktop /usr/share/applications/
systemctl daemon-reload
if [[ ! -f /etc/snap-back/config.json ]]; then
    /usr/local/bin/snap-back configure --destination "${1:?Supply an already mounted Btrfs backup drive path}"
fi
systemctl enable --now snapper-cleanup.timer
systemctl enable --now snap-back.timer
printf 'Snap Back installed. Open it from the application launcher.\n'
