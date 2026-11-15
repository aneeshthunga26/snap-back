# Final product and UI review

This pass ran after backup, recovery, installation, and VM verification were complete. It changed the Rust GUI only; the backup engine and recovery flow were left intact.

## Copy

Replaced slogans and reassurance with task labels and observed facts: Backups, Restore, Settings, latest backup date, drive state, and schedule. Removed promises of an “always” available way back or a “complete history.” Retention now explains that newer backups replace older backups in the same period. Full recovery uses a readable backup date and explicitly describes replacement, preparation, cancellation, and restart.

## Behavior fixes

- Invalidated restore points clear the selected file, destination, and recovery confirmation. Responses from an older folder request cannot populate a different backup.
- Failed folder loads show an error and Retry action instead of appearing empty. Authentication failures use a clear explanation instead of PolicyKit's alarming boilerplate.
- Failed status refreshes mark displayed data as stale and disable operations until refresh succeeds. Automatic status polling continues during operations.
- File selection places the destination and Copy action above the browser, where they remain visible. Successful copies clear the selection; failures preserve it for correction.
- Default recovery folders have readable dates, bounded Unicode filenames, and unused-name suffixes. The interface explains that the selected item is copied inside a new folder.
- System recovery confirmation occupies the main panel rather than appearing below the fold. Leaving Restore clears the confirmation. Preparation and restart remain separate actions.
- Pending recovery, disconnection, loading, and unreadable Snapper settings have distinct states. Hidden capacity is not shown as a misleading zero-sized drive.

## VM verification

All builds, formatting, linting, tests, graphical interaction, and screenshots ran inside `cachy-vm`.

- GUI build and strict Clippy passed.
- Six regression tests passed: restore-point invalidation, stale folder responses, folder errors, status errors, confirmation navigation, and Unicode path limits.
- Real GUI file recovery copied backed-up `.bashrc`; content matched its snapshot. A repeated selection chose an unused folder name. An explicitly existing destination was refused, with the selection retained.
- Confirmation was visible, disabled without the required text, and cancelled without preparing recovery.
- Overview, Settings, and confirmation were inspected at the 940 × 680 minimum window size; the normal 1180 × 800 pages were also inspected.
- An unprivileged GUI loaded status without authentication. Its real authorization failure while browsing displayed the intended error and retry state.

The rebuilt GUI is installed in the VM. Screenshots were refreshed under `docs/screenshots/`. Test recovery folders and Xvfb/GUI processes were removed. The hourly timer remained active; no recovery, reboot, scheduler change, or backup deletion occurred during this pass.

## Follow-up review — 15 November 2026

After the drive picker, motion changes, and desktop window controls were finished, a separate product review checked the current interface and copy again. The Overview action “Change schedule” implied an editable backup interval, while Settings only enables or pauses the fixed hourly schedule. It now reads “Backup settings.” The drive erase warning, exact confirmation phrase, recovery preparation and restart steps, file-copy destination explanation, and loading/error states remain direct and describe their actual effects.

The copy-only change was built and installed inside `cachy-vm`, and the installed app was reopened for testing. No backup drive, scheduler, recovery, or desktop preference was changed during this review.

## Rounded corners and restore loading review

After implementation and VM verification finished, a focused product review inspected the rounded-window, pending-read, and completed-read screenshots alongside the restore browser's request handling. The small “Updating…” label is sufficient, the previous rows and their folder path remain together, and file/recovery actions stay unavailable until the current read completes. Restore-point selection remains responsive during loading, and late responses cannot silently replace it. The rounded window treatment stays consistent with the existing cards and does not crowd the caption controls.

No further copy or interaction changes were warranted. This review changed documentation only; the verified build remains installed and open in the VM.

## Multi-drive selection and restore toolbar review

The final review inspected the split button, checkbox menu, backup result, and restore toolbar after the multi-drive implementation and VM tests. The menu hint now reads “Back up now saves this selection for hourly backups.” This states both when the checkboxes are remembered and which later backups use them. The selected-drive count, full-row checkbox targets, empty-selection guard, and separate restore-drive label make the destination and browsing choices clear. Home, Root, and Up share one toolbar row.

The software-rendered restore-toolbar capture showed teal arc trails after navigating away from a completed backup. This visual artifact was reported for final verification in the normal renderer; no animation code was changed during this focused product review.

The implementation follow-up resolved the artifact by embedding rotation in a bounded set of fixed-size SVG frames. A VM graphical regression check confirmed identical inner-logo, sidebar-brand, and outside-hero pixels across five busy frames while the outer annulus continued moving. Stopping the animation and navigating to Restore also left no trails. The restore-toolbar screenshot was refreshed; see `evidence/multi-drive/motion-regression.json`.
