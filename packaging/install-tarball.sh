#!/usr/bin/env bash
# Install an extracted Snap Back binary release. DESTDIR stages files only.
set -euo pipefail

release_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
: "${DESTDIR:=}"

if [[ -n "$DESTDIR" ]]; then
    [[ "$DESTDIR" == /* ]] || {
        echo 'DESTDIR must be an absolute staging directory other than /.' >&2
        exit 1
    }
    DESTDIR=$(realpath -m -- "$DESTDIR")
    [[ "$DESTDIR" != / ]] || { echo 'DESTDIR cannot resolve to /.' >&2; exit 1; }
elif [[ $EUID != 0 ]]; then
    echo 'Run sudo ./install.sh, or set DESTDIR to stage the installation.' >&2
    exit 1
fi

for binary in snap-back snap-back-gui; do
    [[ -x "$release_root/bin/$binary" ]] || {
        echo 'Run install.sh from an extracted Snap Back binary release.' >&2
        exit 1
    }
done

install -Dm755 "$release_root/bin/snap-back" "$DESTDIR/usr/local/bin/snap-back"
install -Dm755 "$release_root/bin/snap-back-gui" "$DESTDIR/usr/local/bin/snap-back-gui"
for unit in snap-back.service snap-back.timer; do
    install -Dm644 "$release_root/packaging/$unit" "$DESTDIR/etc/systemd/system/$unit"
done
install -Dm644 "$release_root/packaging/org.snapback.manage.policy" \
    "$DESTDIR/usr/share/polkit-1/actions/org.snapback.manage.policy"
install -Dm644 "$release_root/packaging/snap-back.desktop" \
    "$DESTDIR/usr/share/applications/snap-back.desktop"
install -Dm755 "$release_root/packaging/initcpio/snap-back" \
    "$DESTDIR/usr/lib/initcpio/install/snap-back"
install -Dm644 "$release_root/packaging/initcpio/snap-back-recovery.service" \
    "$DESTDIR/usr/lib/systemd/system/snap-back-recovery.service"
install -Dm644 "$release_root/packaging/initcpio/recovery.conf" \
    "$DESTDIR/usr/lib/snap-back/sysroot.mount.d/recovery.conf"
install -Dm644 "$release_root/LICENSE" "$DESTDIR/usr/share/licenses/snap-back/LICENSE"
install -Dm644 "$release_root/THIRD-PARTY-NOTICES.txt" \
    "$DESTDIR/usr/share/licenses/snap-back/THIRD-PARTY-NOTICES.txt"

if [[ -n "$DESTDIR" ]]; then
    printf 'Snap Back installation staged in %s\n' "$DESTDIR"
else
    systemctl daemon-reload
    systemctl enable --now snap-back.timer
    printf 'Snap Back installed. Open it from your application launcher to choose backup drives.\n'
fi
