# VM validation

All compilation, formatting, linting, automated tests, graphical interaction, and destructive recovery tests ran inside `cachy-vm` on 14 November 2026. The host was used for source editing, SSH/libvirt control, disk provisioning, and viewing VM-produced evidence.

## Rust checks

- `cargo build --workspace`: passed.
- `cargo test --workspace`: passed, including retention tier selection, same-hour coalescing, and path traversal rejection.
- `cargo clippy --workspace --all-targets -- -D warnings`: passed.

## Real backup and file recovery

The VM-only smoke suite passed 17 checks. See `evidence/smoke-results.json`.

- Full Btrfs transfers for root, home, root's home, srv, cache, temporary data, logs, and nested machine/portable subvolumes.
- Root, home, srv, and nested-subvolume marker files present on the backup disk.
- Backed-up home browsable through the Rust helper.
- File recovery preserved content and assigned the invoking user's ownership.
- Existing recovery destinations and path traversal refused.
- The next backup used incremental Btrfs send and contained newly added content.
- Same-hour retention pruned the previous point while keeping the new point readable.
- Hourly scheduling could be enabled and paused.
- Offline recovery refused invocation from a running system.
- A newly created, unconfigured nested Btrfs subvolume caused backup refusal instead of silent omission.

Additional checks confirmed that a disconnected backup disk is shown as disconnected, backup refuses to write into its unmounted directory, competing operations are locked out, and unprivileged status reads correctly report a running operation. Fresh configuration discovered all nine source subvolumes.

## Graphical verification

The real Iced/iced-shadcn application ran under Xvfb inside the VM, using its installed Rust helper. Overview, Restore, and Settings rendered at 1180 × 800 without clipping. Root/home browsing worked, live backup progress appeared, and controls were disabled during an active operation. A real click on file recovery recovered the home marker and displayed the result. Idle status reads require no authentication; privileged operations go through PolicyKit.

Screenshots in `screenshots/` were generated in the VM.

## Full system recovery

Recovery checkpoint: `20261114T075907160810303Z`.

The test overwrote marker files in root, home, srv, and `/var/lib/machines` and added files after the checkpoint. Recovery then booted its dedicated initramfs, switched all nine configured subvolumes before root was mounted, restored the matching boot files, and rebooted normally.

All 16 post-reboot checks passed. See `evidence/recovery-results.json`:

- The original checkpoint contents returned in root, home, srv, and the nested machines subvolume.
- Files created after the checkpoint disappeared from root and home.
- The VM boot ID changed; the journal committed `phase: completed`; the recovery entry disarmed itself.
- The booted kernel had matching installed modules.
- Home file ownership and both root/home Snapper snapshot histories survived.
- The previous root, home, and auxiliary contents remained accessible in rollback subvolumes.

Recovery preparation cancellation also passed: it restored the original Limine configuration and removed the pending journal.

An initial recovery attempt hit an overly broad anchor-cache guard. Its rollback restored all original subvolumes and boot files successfully, recorded `rollback: Ok(())`, and the original system booted again. See `evidence/rollback-confirmed.json`. The cache-preservation optimization was removed; the corrected recovery then passed all checks. The error path also now reboots automatically when rollback and boot restoration complete safely. That new automatic error-path reboot was code-reviewed and compiled; the actual rollback was exercised before that UX improvement.

## Practical limits

This is an installed prototype for CachyOS with Limine, mkinitcpio, Btrfs `@` root, separate home, and an EFI partition at `/boot`. It restores an existing system filesystem; it does not provision a blank replacement system disk. Snapshots across subvolumes are sequential and filesystem-consistent. Old system volumes remain for manual inspection/cleanup and consume disk space. Incremental source caches can be lost by a full system restore; the next backup falls back to a full transfer.

## Installed service after recovery

The installed `snap-back.service` completed successfully after full recovery (`Result=success`, exit status 0), rebuilding its local parents with a full backup as expected. File recovery with the final hardened helper preserved content and user ownership. A subsequent incremental backup also passed. Temporary live-system test markers and recovered test folders were removed; the hourly timer was left enabled and active, with the next run scheduled normally.

## Final UI/UX pass

After all implementation and recovery work finished, a separate product/UI review removed promotional copy and corrected stale selections, folder error handling, copy control visibility, confirmation placement, destination naming, and authorization messages. Six GUI regression tests passed. The final workspace test run passed all nine tests and strict workspace Clippy. See `UX-REVIEW.md` for the changes and VM graphical checks.

## Drive setup and animation update

A later update added graphical drive setup and saved-drive selection, including an explicit all-data/all-partitions erase warning and an exact device-specific typed confirmation. Real GUI setup replaced a disposable GPT/ext4 drive with Btrfs, saved its mount/configuration, and completed a full backup. Switching, automatic remounting, refusal guards, hotplug identity rejection, and post-format configuration-failure cleanup all passed inside the VM. See `DRIVE-SETUP.md`.

The requested `iced_anim` motion now gently pulses the backup symbol and rotates an outer arc while a backup runs, with restrained sidebar/activity feedback. A single cached SVG handle and native container borders avoid repeated image allocation. Real backup-click tests showed changing busy frames and identical idle frames after settling. Scrolling/navigation were checked for rendering artifacts; the initial canvas implementation was replaced after it failed that check. Memory warmed up and then plateaued during the recording.

The final GUI has 16 passing regression tests; with the three core tests, the workspace has 19 passing tests. Strict workspace Clippy and the workspace build pass. Tests, builds, formatting, GUI runs, screenshots, and animation recordings all ran inside `cachy-vm`.

## Theme and motion refinements — 15 November 2026

The Overview usage bar now explicitly uses the app's teal accent. The hero's inner disk and logo stay fixed: only a transparent band expands from just outside the disk toward the rotating outer ring. A VM recording of 30 busy frames showed identical pixels inside radius 40, with changes confined to the outer annulus; settled idle frames were identical. The usage-bar fill sampled as `rgb(82, 222, 184)`.

The app now requests client-side decorations and draws desktop-aware caption controls. All four symbolic icons resolve from the VM's installed Breeze Dark theme. KDE and GTK/GNOME layout parsing, explicit empty groups, duplicate suppression, and recursive icon inheritance have regression coverage. See `WINDOW-CONTROLS.md` for source references and integration limits.

The final VM workspace run passed 23 tests (20 GUI and 3 core), `cargo fmt --all --check`, strict workspace Clippy, and the workspace build. No build, test, formatter, or application execution was performed on the host.

An isolated KWin compositor inside the VM passed nine real input checks on each of native Wayland and X11: minimize, maximize, restore, compositor restore after minimizing, caption double-click in both states, dragging, corner resizing, and closing. Results are in `evidence/window-controls-wayland.json` and `evidence/window-controls-x11.json`. The Wayland protocol trace showed `zxdg_toplevel_decoration_v1.set_mode(1)` followed by `configure(1)`, confirming client-side decorations. The default right-side and a temporary left-side KDE button layout were visually inspected; screenshots are `screenshots/window-controls.png` and `screenshots/window-controls-left.png`.

The test compositor must use `KWIN_COMPOSE=O2`: this KWin version no longer supports the old QPainter compositor. Selecting `Q` left it running without delivering frame callbacks, which initially made otherwise successful actions appear stuck. The supported compositor passed all checks without an application change.

## Restore browser continuity

The restore browser keeps its last completed listing visible while another restore point, source, or folder is loading. A small “Updating…” label identifies the pending read; file selection, copying, and system recovery remain disabled until the new listing arrives. The displayed folder path stays with the visible rows. Restore-point and source controls remain available for rapid switches, and clicking an already selected restore point does not reload it.

Each browse request has a generation number, so an older response cannot replace the newest selection, including a rapid switch from A to B and back to A. Regression tests cover preserving the old listing while blocking file actions and rejecting a late response for the same point.

The VM graphical check introduced a 1.5-second browse delay using a private mount namespace; it did not change the installed helper. File-row pixels were identical before and during the pending read, the new listing completed, and overlapping restore-point switches remained responsive. See `evidence/restore-loading/results.json` and the VM-only harness `tools/vm-restore-loading-qa.py`.

## Rounded window refinement

Restored windows now use a transparent surface with native antialiased 8 px corners. Sidebar corners match the outer background, and an 8 px bottom inset keeps the main scrollbar clear of the corner. Maximizing makes the surface square. Both tiny-skia/X11 and WGPU/native Wayland were checked inside the VM. All four corner pixels were transparent when restored, opaque when maximized, and transparent after restoring again; boundary pixels had partial alpha for smooth edges.

The final implementation draws each widget once. Sidebar colors use a bounded 150 ms eased transition, which settled correctly after delayed software-rendered frames. The active hero and fixed inner logo remained visually correct. In the isolated VM session WGPU used 0% of one CPU core at idle and about 15% during animation; the software renderer used a full core during animation. These are measurements of this VM, not a performance guarantee. See `evidence/rounded-window.json` and `screenshots/rounded-window.png`.

The combined update passed all 25 workspace tests (22 GUI and 3 core), formatting, strict workspace Clippy, and the workspace build inside the VM. The latest GUI was installed and reopened as the normal desktop user. The backup-drive selection and hourly timer were preserved.

## Multiple backup destinations and restore toolbar

Home, Root, and Up now share one toolbar row. The Back up now split button opens a checkbox menu of saved drives. Checkboxes and drive labels toggle without dismissing it; outside clicks and Escape close it. Clearing the selection disables the backup action. A manual backup remembers the selected destinations for hourly backups, independently of the drive whose restore points are being viewed.

Each drive has its own immutable incremental parent under `anchors/by-drive/UUID/`, point history, and retention. The previous single-drive configuration retains its existing destination selection until changed. Switching the restore browser preserves the selected backup destinations, and setting up another drive adds it without deselecting the others. A failed selected destination is named in the result while other selected destinations are still attempted.

All 22 real backend integration checks passed in the VM: both disks received all nine subvolumes and boot files, root/home marker contents matched, a second scheduled run was incremental on both disks, hourly retention applied separately, and file recovery from each disk returned current content with the user's ownership. A missing saved destination produced a named partial-failure result while the healthy drive completed; the unselected disk stayed unchanged and no point was written into an unmounted directory. Invalid target IDs were refused before changing the configuration or restore points. See `evidence/multi-drive/results.json` and `tests/vm_multi_drive.py`.

Eleven real GUI checks passed, including one Back up now click creating fresh restore points on both selected disks, empty-selection guards, checkbox/label behavior, menu dismissal, and OCR verification that Home, Root, and Up share a row. Native Wayland with the default renderer also passed the dropdown checks with responsive updates. See `evidence/multi-drive/gui.json`, `evidence/multi-drive/native-gui.json`, and `screenshots/multi-drive-menu.png`.

The combined VM workspace run passed 33 tests (26 GUI and 7 core), formatting, strict workspace Clippy, and the workspace build. Both backup drives remain selected; the 64 GiB drive remains the restore browser's drive. The hourly timer is active.

The final product review clarified the menu's save timing and caught trails from externally rotating SVGs in the software renderer. The arc now selects among 360 prebuilt, fixed-size SVG frames with rotation inside the SVG. This keeps pixels inside the renderer's damage bounds and avoids generating image handles during animation. All six VM graphical regression checks passed: the inner logo, sidebar brand, and outside-hero region were pixel-identical across five busy frames, the annulus changed, and no trails remained after stopping or navigating to Restore. See `evidence/multi-drive/motion-regression.json` and `tools/vm-motion-regression.py`.

## First launch (0.1.1, 16 November 2026)

All 38 workspace tests, formatting checks, strict Clippy, and the release build passed inside `cachy-vm`. Three core regressions distinguish missing configuration from malformed JSON and read failures; GUI regressions cover setup, repeated polling, and the transition to configured backups.

`tools/vm-first-launch-qa.py` ran the actual GUI and helper in a private mount namespace, hiding the configuration only from that test. All ten graphical checks passed. The first-launch view offered drive setup, and 24 frames stayed pixel-identical across three status calls, each delayed by 700 ms. Clicking the primary action opened the actual drive inventory. A malformed test configuration showed one error card with horizontal recovery actions; removing it returned to setup, and providing a valid configuration opened the normal backup overview. The real configuration and disks were unchanged. See `evidence/first-launch.json` and `screenshots/first-launch.png`.

## Caption alignment (0.1.2, 16 November 2026)

VM screenshots reproduced icons anchored toward the top-left of their button circles. Using centered, down-only SVG fitting corrected the offset while retaining native icon sizes and full-circle hover behavior. Close, maximize, and minimize were checked with the installed Adwaita and Breeze symbolic icons at 100% and 125% scaling. Native glyph placement is preserved, including Adwaita’s low minimize stroke.

## Safe unmount (16 November 2026)

All 39 workspace tests, strict Clippy, and the release build passed in the VM. Actual secondary-drive checks covered unknown-drive refusal, the operation lock, kernel busy-mount refusal with suppression rollback, successful unmount, scheduled-backup suppression, and remount through Use drive. Configuration and target selections were unchanged. The real GUI Unmount action also unmounted that secondary drive; both saved drives were left mounted afterward. Controls fit at 940 × 680 and 1180 × 800. Pending recovery is refused by the existing backend guard; this pass did not stage a recovery. See `evidence/unmount.json` and `screenshots/unmount-drives.png`.
