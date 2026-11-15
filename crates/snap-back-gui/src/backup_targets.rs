//! Backup destinations are independent of the drive being browsed for recovery.
use super::*;
use iced::widget::{column, tooltip};
use iced_shadcn::PopoverProps;

impl App {
    pub(super) fn backup_targets(&self) -> &[String] {
        self.backup_selection.as_deref().unwrap_or_else(|| {
            self.status
                .as_ref()
                .map(|status| status.backup_targets.as_slice())
                .unwrap_or_default()
        })
    }

    pub(super) fn can_choose_backup_targets(&self) -> bool {
        self.can_operate()
            && self.status.as_ref().is_some_and(|status| {
                status.recovery_pending.is_none() && !status.backup_drives.is_empty()
            })
    }

    pub(super) fn reconcile_backup_selection(&mut self, status: &Status) {
        if let Some(selection) = &mut self.backup_selection {
            // Disconnecting a drive keeps its checkmark. Only forgetting a
            // registered drive removes it from a pending selection.
            selection.retain(|uuid| status.backup_drives.iter().any(|d| &d.uuid == uuid));
            if selection.len() == status.backup_targets.len()
                && selection
                    .iter()
                    .all(|uuid| status.backup_targets.contains(uuid))
            {
                self.backup_selection = None;
            }
        }
    }

    pub(super) fn backup_arguments(&self) -> Vec<String> {
        let mut args = vec!["backup".into(), "--save-targets".into()];
        for uuid in self.backup_targets() {
            args.extend(["--drive".into(), uuid.clone()]);
        }
        args
    }

    pub(super) fn backup_button(&self) -> Element<'_, Message> {
        let enabled = self.can_backup();
        let choose_enabled = self.can_choose_backup_targets();
        let primary = button(container(text("Back up now").size(14)).center_y(40))
            .on_press_maybe(enabled.then_some(Message::Backup))
            .padding([0, 18])
            .height(40)
            .style(move |_, state| split_style(enabled, false, state));
        let arrow = tooltip(
            button(container(text("▾").size(17)).center_x(36).center_y(40))
                .on_press_maybe(choose_enabled.then_some(Message::BackupMenu))
                .width(36)
                .height(40)
                .padding(0)
                .style(move |_, state| split_style(choose_enabled, true, state)),
            text("Choose backup drives").size(12),
            tooltip::Position::Bottom,
        )
        .delay(Duration::from_millis(600));
        let mut drives = column![].spacing(12);
        if let Some(status) = &self.status {
            for drive in &status.backup_drives {
                let uuid = drive.uuid.clone();
                let row_uuid = uuid.clone();
                let selected = self.backup_targets().contains(&uuid);
                let checkbox = iced::widget::checkbox(selected)
                    .on_toggle_maybe(
                        choose_enabled
                            .then_some(move |value| Message::BackupTarget(uuid.clone(), value)),
                    )
                    .size(16)
                    .style(move |_, state| iced::widget::checkbox::Style {
                        background: if selected { TEAL } else { RAISED }.into(),
                        icon_color: BG,
                        border: Border {
                            color: if matches!(
                                state,
                                iced::widget::checkbox::Status::Hovered { .. }
                            ) {
                                TEAL
                            } else {
                                LINE
                            },
                            width: 1.0,
                            radius: 4.0.into(),
                        },
                        text_color: None,
                    });
                drives = drives.push(
                    button(
                        row![
                            checkbox,
                            column![
                                text(if drive.label.is_empty() {
                                    "Backup drive"
                                } else {
                                    &drive.label
                                })
                                .size(14),
                                text(if !drive.connected {
                                    "Not connected"
                                } else if drive.mounted {
                                    "Connected"
                                } else {
                                    "Available"
                                })
                                .size(11)
                                .color(if drive.connected { MUTED } else { RED }),
                                text(&drive.mount).size(10).color(MUTED)
                            ]
                            .spacing(4)
                            .width(Fill)
                        ]
                        .spacing(12)
                        .align_y(Alignment::Center),
                    )
                    .on_press_maybe(
                        choose_enabled.then_some(Message::BackupTarget(row_uuid, !selected)),
                    )
                    .padding(8)
                    .width(Fill)
                    .style(move |_, state| button::Style {
                        background: matches!(
                            state,
                            button::Status::Hovered | button::Status::Pressed
                        )
                        .then_some(RAISED.into()),
                        text_color: WHITE,
                        border: Border::default().rounded(6),
                        ..Default::default()
                    }),
                );
            }
        }
        let count = self.backup_targets().len();
        let menu = column![
            text("Back up to").size(15),
            container(scrollable(drives).height(iced::Length::Shrink)).max_height(260),
            rule::horizontal(1),
            text(if count == 0 {
                "Select at least one drive.".to_owned()
            } else {
                format!(
                    "{count} {} selected",
                    if count == 1 { "drive" } else { "drives" }
                )
            })
            .size(12)
            .color(if count == 0 { MUTED } else { TEAL }),
            text("Back up now saves this selection for hourly backups.")
                .size(11)
                .color(MUTED)
        ]
        .spacing(14)
        .width(288);
        // The library owns open/close state, outside-click dismissal, and Escape.
        // Checkbox messages update the selection without dismissing the popover.
        let dropdown = iced_shadcn::popover(
            arrow,
            menu,
            PopoverProps::new()
                .max_width(330)
                .offset(8.0)
                .disabled(!choose_enabled),
            &self.shadcn,
        );
        let split = row![
            primary,
            container(
                container(Space::new().width(1).height(22)).style(|_| container::Style {
                    background: Some(BG.scale_alpha(0.20).into()),
                    ..Default::default()
                })
            )
            .center_y(40)
            .style(move |_| container::Style {
                background: Some(if choose_enabled { TEAL } else { RAISED }.into()),
                ..Default::default()
            }),
            dropdown
        ]
        .spacing(0);
        column![
            split,
            text(if count == 0 {
                "Choose backup drives".into()
            } else {
                format!(
                    "{count} backup {}",
                    if count == 1 { "drive" } else { "drives" }
                )
            })
            .size(11)
            .color(MUTED)
        ]
        .align_x(Alignment::End)
        .spacing(7)
        .into()
    }
}

fn split_style(enabled: bool, right: bool, state: button::Status) -> button::Style {
    let color = if !enabled {
        RAISED
    } else if matches!(state, button::Status::Hovered | button::Status::Pressed) {
        Color::from_rgb(0.38, 0.91, 0.77)
    } else {
        TEAL
    };
    button::Style {
        background: Some(color.into()),
        text_color: if enabled { BG } else { MUTED },
        border: Border {
            radius: iced::border::Radius {
                top_left: if right { 0.0 } else { 6.0 },
                bottom_left: if right { 0.0 } else { 6.0 },
                top_right: if right { 6.0 } else { 0.0 },
                bottom_right: if right { 6.0 } else { 0.0 },
            },
            ..Default::default()
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status {
            drive: Drive {
                connected: true,
                ..Drive::default()
            },
            backup_targets: vec!["a".into()],
            backup_drives: ["a", "b"]
                .map(|uuid| BackupDrive {
                    uuid: uuid.into(),
                    connected: true,
                    mounted: true,
                    ..BackupDrive::default()
                })
                .into(),
            ..Status::default()
        }
    }

    #[test]
    fn polling_preserves_draft_and_successful_save_acknowledges_it() {
        let (mut app, _) = App::new();
        let _ = app.update(Message::StatusLoaded(Ok(status())));
        let _ = app.update(Message::BackupTarget("b".into(), true));
        let _ = app.update(Message::StatusLoaded(Ok(status())));
        assert_eq!(app.backup_targets(), ["a", "b"]);
        assert_eq!(
            app.backup_arguments(),
            ["backup", "--save-targets", "--drive", "a", "--drive", "b"]
        );
        let mut saved = status();
        saved.backup_targets = vec!["b".into(), "a".into()];
        let _ = app.update(Message::StatusLoaded(Ok(saved)));
        assert!(app.backup_selection.is_none());
    }

    #[test]
    fn empty_selection_blocks_backup_without_blocking_recovery_or_picker() {
        let (mut app, _) = App::new();
        app.status = Some(status());
        let _ = app.update(Message::BackupTarget("a".into(), false));
        let _ = app.update(Message::Backup);
        assert!(app.backup_targets().is_empty() && !app.can_backup());
        assert!(app.can_choose_backup_targets() && app.can_recover());
        assert!(app.operation_command.is_none());
    }

    #[test]
    fn disconnected_targets_stay_selected_and_active_restore_drive_is_independent() {
        let (mut app, _) = App::new();
        app.status = Some(status());
        let _ = app.update(Message::BackupTarget("b".into(), true));
        let mut disconnected = status();
        disconnected.drive.connected = false;
        disconnected.backup_drives[1].connected = false;
        let _ = app.update(Message::StatusLoaded(Ok(disconnected)));
        assert_eq!(app.backup_targets(), ["a", "b"]);
        assert!(app.can_backup() && !app.can_recover());
        let mut forgotten = status();
        forgotten.backup_drives.retain(|drive| drive.uuid != "b");
        let _ = app.update(Message::StatusLoaded(Ok(forgotten)));
        assert_eq!(app.backup_targets(), ["a"]);
    }

    #[test]
    fn selection_changes_require_known_drives_and_an_idle_operation() {
        let (mut app, _) = App::new();
        app.status = Some(status());
        let _ = app.update(Message::BackupTarget("unknown".into(), true));
        assert_eq!(app.backup_targets(), ["a"]);
        app.busy = Some("Backing up…".into());
        let _ = app.update(Message::BackupTarget("b".into(), true));
        assert_eq!(app.backup_targets(), ["a"]);
    }
}
