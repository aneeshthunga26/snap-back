# Drive setup validation

All builds, formatting, linting, unit tests, graphical interaction, filesystem changes, and backup tests ran inside `cachy-vm`. The host only supplied source editing and libvirt disk attachment.

The VM received a disposable 64 GiB disk with a GPT partition table, ext4 partition, and an old-data marker. The real GUI showed its serial, capacity, and device path and required the exact phrase `ERASE /dev/vdc` after a prominent all-data/all-partitions erase warning. Incorrect text kept the destructive button disabled. Cancelling the review left the data intact.

The actual GUI confirmation succeeded: the old partitions and filesystem were replaced with whole-device Btrfs, the marker disappeared, a new UUID was mounted, a matching persistent fstab entry was saved, both drives were registered, and the original source-subvolume configuration was unchanged. A full backup to this new drive included all nine subvolumes, with root and home contents readable. Original backup manifests retained matching checksums.

The real GUI switched back to the original drive, switched to the second drive after it was unmounted (automatically mounting it), and returned to the original. Each displayed the correct timeline. Switching did not format either drive.

Refusal tests passed for:

- System and current/saved backup disks.
- Partitions, regular files, arbitrary paths, and mounted child partitions.
- Incorrect device confirmation, malformed tokens, and expired plans.
- A disk detached and reattached under the same `/dev/vdc` path after planning; its changed kernel disk sequence invalidated the old confirmation.
- An existing backup/restore lock and pending system recovery.

The original ext4 marker remained readable after every refusal test.

A separate disposable 2 GiB disk exercised a real failure after formatting. Making the configuration file temporarily unwritable forced the save to fail: the original config and fstab stayed byte-identical and the newly mounted disk was unmounted. A non-destructive failed saved-drive switch verified the same cleanup and absence of leftover temporary configuration files. The failure-test disk was then detached.

Destructive commands operate through a retained block-device descriptor with disk-sequence checks; they do not take an arbitrary device path from the UI. Cooperative device locks are released for udev event processing to avoid deadlocking discovery. Filesystem formatting cannot be undone by the app; a setup failure after erasure reports that clearly.

The original backup destination and automatic hourly schedule are restored at handoff. Screenshots are in `screenshots/drive-setup-*.png`.
