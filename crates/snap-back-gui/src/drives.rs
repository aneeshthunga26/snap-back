use super::*;
use iced::widget::column;

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct DriveInventory {
    pub registered: Vec<SavedDrive>,
    pub candidates: Vec<NewDrive>,
}
#[derive(Debug, Clone, Deserialize)]
pub(super) struct SavedDrive {
    pub uuid: String,
    pub mount: String,
    pub label: String,
    pub connected: bool,
    pub active: bool,
    pub mounted: bool,
    pub unmounted: bool,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub size_bytes: u64,
}
#[derive(Debug, Clone, Deserialize)]
pub(super) struct NewDrive {
    pub device: String,
    pub name: String,
    pub model: String,
    pub serial: String,
    pub size_bytes: u64,
    pub fstype: Option<String>,
    pub label: Option<String>,
    pub eligible: bool,
    pub reason: Option<String>,
}
#[derive(Debug, Clone, Deserialize)]
pub(super) struct DrivePlan {
    pub token: String,
    pub device: String,
    pub name: String,
    pub model: String,
    pub serial: String,
    pub size_bytes: u64,
    pub confirmation: String,
    pub expires_at: String,
}
#[derive(Default)]
pub(super) struct DriveSetup {
    pub inventory: Option<DriveInventory>,
    pub error: Option<String>,
    pub loading: bool,
    pub planning: bool,
    pub generation: u64,
    pub selected: Option<String>,
    pub plan: Option<DrivePlan>,
    pub confirmation: String,
}

impl App {
    pub(super) fn can_open_drives(&self) -> bool {
        !self.is_busy() && !self.browsing
    }
    pub(super) fn can_change_drive(&self) -> bool {
        self.can_open_drives()
            && !self.drive_setup.loading
            && !self.drive_setup.planning
            && self
                .status
                .as_ref()
                .is_none_or(|s| s.recovery_pending.is_none())
            && (self.status.is_none() || self.status_error.is_none())
    }
    pub(super) fn clear_drive_plan(&mut self) {
        self.drive_setup.generation = self.drive_setup.generation.wrapping_add(1);
        self.drive_setup.plan = None;
        self.drive_setup.selected = None;
        self.drive_setup.confirmation.clear();
        self.drive_setup.planning = false;
    }
    pub(super) fn refresh_drives(&mut self) -> Task<Message> {
        if !self.can_open_drives() {
            return Task::none();
        }
        self.clear_drive_plan();
        self.drive_setup.loading = true;
        self.drive_setup.error = None;
        let generation = self.drive_setup.generation;
        Task::perform(
            async {
                let value = backend(vec!["drives".into()]).await?;
                serde_json::from_value(value).map_err(|e| format!("Could not read drives: {e}"))
            },
            move |result| Message::DrivesLoaded { generation, result },
        )
    }
    pub(super) fn erase_is_confirmed(&self) -> bool {
        let Some(plan) = &self.drive_setup.plan else {
            return false;
        };
        let current = self.drive_setup.inventory.as_ref().is_some_and(|i| {
            i.candidates
                .iter()
                .any(|d| d.device == plan.device && d.eligible)
        });
        let unexpired = chrono::DateTime::parse_from_rfc3339(&plan.expires_at)
            .is_ok_and(|date| date > chrono::Utc::now());
        self.page == Page::Drives
            && self.can_change_drive()
            && current
            && unexpired
            && self.drive_setup.selected.as_deref() == Some(plan.device.as_str())
            && self.drive_setup.confirmation == plan.confirmation
            && plan.confirmation == format!("ERASE {}", plan.device)
            && !plan.token.is_empty()
    }
    pub(super) fn start_drive_operation(
        &mut self,
        label: &str,
        args: Vec<String>,
    ) -> Task<Message> {
        if !self.can_change_drive() {
            return Task::none();
        }
        self.busy = Some(label.into());
        self.motion.set_active(true);
        self.operation_command = args.first().cloned();
        self.notice = None;
        Task::perform(backend(args), |result| {
            Message::DriveOperationDone(result.map(|j| {
                j.get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Backup drive updated.")
                    .to_owned()
            }))
        })
    }
    pub(super) fn drive_picker(&self) -> Element<'_, Message> {
        if self
            .operation_command
            .as_deref()
            .is_some_and(|c| c == "setup-drive" || c == "use-drive" || c == "unmount-drive")
        {
            return card(column![text(if self.operation_command.as_deref()==Some("setup-drive"){"Preparing backup drive…"}else if self.operation_command.as_deref()==Some("unmount-drive"){"Unmounting backup drive…"}else{"Opening drive backups…"}).size(23),text("Keep the drive connected. This operation must finish before another drive can be opened.").size(14).color(MUTED)].spacing(14));
        }
        if self.drive_setup.planning {
            return card(
                column![
                    text("Checking selected drive…").size(23),
                    text("No data has been erased.").size(14).color(MUTED),
                    self.action(
                        "Cancel",
                        Message::CancelDrivePlan,
                        ButtonVariant::Outline,
                        true
                    )
                ]
                .spacing(14),
            );
        }
        if let Some(plan) = &self.drive_setup.plan {
            let expired = chrono::DateTime::parse_from_rfc3339(&plan.expires_at)
                .map(|date| date <= chrono::Utc::now())
                .unwrap_or(true);
            let mut body=column![
                text("Erase and set up backup drive").size(24),
                disk_identity(&plan.name,&plan.device,&plan.model,&plan.serial,plan.size_bytes),
                container(column![text("All existing data will be erased").size(21).color(RED),text(format!("This will erase all existing data on {}, including all partitions. This cannot be undone.",plan.device)).size(15),text("The drive will be formatted as Btrfs and added to your backup destinations. Backups on other drives are kept.").size(13).color(MUTED)].spacing(12)).padding(18).style(|_|container::Style{background:Some(Color::from_rgb(0.18,0.08,0.10).into()),border:Border{color:RED.scale_alpha(0.4),width:1.0,radius:9.0.into()},..Default::default()}),
                text(format!("To confirm, type {}",plan.confirmation)).size(14),
                text_input(&plan.confirmation,&self.drive_setup.confirmation).on_input(Message::DriveConfirmation).padding(12),
                row![self.action("Erase and use for backups",Message::ConfirmEraseDrive,ButtonVariant::Destructive,self.erase_is_confirmed()),self.action("Cancel",Message::CancelDrivePlan,ButtonVariant::Outline,true)].spacing(12)
            ].spacing(18);
            if expired {
                body = body.push(
                    text("This confirmation has expired. Cancel and select the drive again.")
                        .size(13)
                        .color(RED),
                );
            }
            if let Some(error) = &self.drive_setup.error {
                body = body.push(text(error).size(13).color(RED));
            }
            return card(body);
        }
        let mut content=column![
            text("Choose a backup drive").size(24),
            text("Browse backups on a saved drive, or erase a new drive to set it up. Choose one or more backup destinations from the arrow beside Back up now.").size(14).color(MUTED),
            row![self.action(if self.drive_setup.loading{"Checking drives…"}else{"Refresh drives"},Message::RefreshDrives,ButtonVariant::Outline,self.can_open_drives() && !self.drive_setup.loading),self.action("Back",Message::Navigate(self.drive_return_page),ButtonVariant::Ghost,self.can_open_drives())].spacing(10)
        ].spacing(18);
        if let Some(error) = &self.drive_setup.error {
            content = content.push(text(error).size(13).color(RED));
        }
        if self
            .status
            .as_ref()
            .is_some_and(|s| s.recovery_pending.is_some())
        {
            content = content.push(
                text("Complete or cancel the prepared system recovery before changing drives.")
                    .size(13)
                    .color(RED),
            );
        }
        let Some(inventory) = &self.drive_setup.inventory else {
            return card(content);
        };
        content = content
            .push(rule::horizontal(1))
            .push(text("Saved backup drives").size(20));
        if inventory.registered.is_empty() {
            content = content.push(text("No saved backup drives.").size(13).color(MUTED));
        }
        for drive in &inventory.registered {
            content = content.push(
                row![
                    column![
                        text(if drive.label.is_empty() {
                            "Backup drive"
                        } else {
                            &drive.label
                        })
                        .size(16),
                        text(&drive.mount).size(12).color(MUTED),
                        text(
                            drive
                                .device
                                .as_ref()
                                .map(|device| format!(
                                    "{device} · {}",
                                    human_bytes(drive.size_bytes)
                                ))
                                .unwrap_or_default()
                        )
                        .size(12)
                        .color(MUTED),
                        text(if drive.unmounted {
                            "Unmounted · backups paused until Use drive or restart"
                        } else if drive.active {
                            "Browsing this drive"
                        } else if drive.connected {
                            "Connected · existing backups are kept"
                        } else {
                            "Not connected"
                        })
                        .size(12)
                        .color(if drive.active { TEAL } else { MUTED })
                    ]
                    .spacing(6)
                    .width(Fill),
                    row![
                        self.action(
                            if !drive.mounted || drive.unmounted {
                                "Use drive"
                            } else if drive.active {
                                "Browsing"
                            } else {
                                "Browse backups"
                            },
                            Message::UseDrive(drive.uuid.clone()),
                            ButtonVariant::Outline,
                            self.can_change_drive()
                                && (!drive.active || !drive.mounted || drive.unmounted)
                                && drive.connected
                        ),
                        self.action(
                            "Unmount",
                            Message::UnmountDrive(drive.uuid.clone()),
                            ButtonVariant::Outline,
                            self.can_change_drive() && drive.mounted
                        )
                    ]
                    .spacing(10)
                ]
                .spacing(14)
                .align_y(Alignment::Center),
            );
        }
        content=content.push(rule::horizontal(1)).push(text("Set up a new drive").size(20)).push(text("New drive setup erases the entire selected disk. System drives, saved backup drives, and drives in use cannot be erased here.").size(13).color(MUTED));
        if inventory.candidates.is_empty() {
            content = content.push(
                text("No disks found. Connect a drive and refresh this list.")
                    .size(13)
                    .color(MUTED),
            );
        }
        for drive in &inventory.candidates {
            let mut details = column![disk_identity(
                &drive.name,
                &drive.device,
                &drive.model,
                &drive.serial,
                drive.size_bytes
            )]
            .spacing(7);
            let existing = match (&drive.fstype, &drive.label) {
                (Some(fs), Some(label)) if !label.is_empty() => {
                    format!("Existing format: {fs} · {label}")
                }
                (Some(fs), _) => format!("Existing format: {fs}"),
                _ => "No filesystem information available".into(),
            };
            details = details.push(text(existing).size(12).color(MUTED));
            if let Some(reason) = &drive.reason {
                details = details.push(text(reason).size(12).color(MUTED));
            }
            content = content.push(card(
                row![
                    details.width(Fill),
                    self.action(
                        "Set up…",
                        Message::SelectNewDrive(drive.device.clone()),
                        ButtonVariant::Outline,
                        drive.eligible && self.can_change_drive()
                    )
                ]
                .spacing(14)
                .align_y(Alignment::Center),
            ));
        }
        card(content)
    }
}
fn disk_identity<'a>(
    name: &'a str,
    device: &'a str,
    model: &'a str,
    serial: &'a str,
    size: u64,
) -> Element<'a, Message> {
    let mut body = column![
        row![
            text(if name.is_empty() { device } else { name }).size(17),
            text(human_bytes(size)).size(15).color(TEAL)
        ]
        .spacing(16),
        text(device).size(13).color(MUTED)
    ]
    .spacing(6);
    if !model.is_empty() {
        body = body.push(text(format!("Model: {model}")).size(12).color(MUTED));
    }
    if !serial.is_empty() {
        body = body.push(text(format!("Serial: {serial}")).size(12).color(MUTED));
    }
    body.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn planned_app() -> App {
        let (mut app, _) = App::new();
        app.page = Page::Drives;
        app.fetching = false;
        app.status = Some(Status::default());
        app.drive_setup.inventory = Some(DriveInventory {
            registered: vec![],
            candidates: vec![NewDrive {
                device: "/dev/vdc".into(),
                name: "Test disk".into(),
                model: String::new(),
                serial: String::new(),
                size_bytes: 1024 * 1024 * 1024,
                fstype: None,
                label: None,
                eligible: true,
                reason: None,
            }],
        });
        app.drive_setup.selected = Some("/dev/vdc".into());
        app.drive_setup.plan = Some(DrivePlan {
            token: "test-plan".into(),
            device: "/dev/vdc".into(),
            name: "Test disk".into(),
            model: String::new(),
            serial: String::new(),
            size_bytes: 1024 * 1024 * 1024,
            confirmation: "ERASE /dev/vdc".into(),
            expires_at: (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339(),
        });
        app
    }
    #[test]
    fn drives_are_not_preselected_and_exact_phrase_is_required() {
        let (app, _) = App::new();
        assert!(app.drive_setup.selected.is_none());
        assert!(!app.erase_is_confirmed());
        let mut app = planned_app();
        for phrase in ["", "ERASE", "ERASE /dev/vdb", "ERASE /dev/vdc "] {
            app.drive_setup.confirmation = phrase.into();
            assert!(!app.erase_is_confirmed());
        }
        app.drive_setup.confirmation = "ERASE /dev/vdc".into();
        assert!(app.erase_is_confirmed());
    }
    #[test]
    fn stale_or_changed_drive_plan_cannot_erase() {
        let mut app = planned_app();
        app.drive_setup.confirmation = "ERASE /dev/vdc".into();
        app.drive_setup.plan.as_mut().unwrap().expires_at =
            (chrono::Utc::now() - chrono::Duration::seconds(1)).to_rfc3339();
        assert!(!app.erase_is_confirmed());
        let mut app = planned_app();
        app.drive_setup.confirmation = "ERASE /dev/vdc".into();
        app.drive_setup.selected = Some("/dev/vdd".into());
        assert!(!app.erase_is_confirmed());
    }
    #[test]
    fn cancelling_plan_invalidates_late_response_and_clears_confirmation() {
        let mut app = planned_app();
        app.drive_setup.confirmation = "ERASE /dev/vdc".into();
        let plan = app.drive_setup.plan.clone().unwrap();
        let generation = app.drive_setup.generation;
        let _ = app.update(Message::CancelDrivePlan);
        let _ = app.update(Message::DrivePlanned {
            generation,
            device: "/dev/vdc".into(),
            result: Ok(plan),
        });
        assert!(
            app.drive_setup.plan.is_none()
                && app.drive_setup.confirmation.is_empty()
                && app.drive_setup.selected.is_none()
        );
    }
    #[test]
    fn existing_recovery_or_busy_operation_blocks_drive_change() {
        let mut app = planned_app();
        app.drive_setup.confirmation = "ERASE /dev/vdc".into();
        app.status.as_mut().unwrap().recovery_pending = Some("point".into());
        assert!(!app.erase_is_confirmed());
        app.status.as_mut().unwrap().recovery_pending = None;
        app.busy = Some("Working".into());
        assert!(!app.erase_is_confirmed());
    }
    #[test]
    fn unconfigured_app_can_open_drive_setup_and_success_resets_restore_state() {
        let (mut app, _) = App::new();
        app.status_error = Some("Not configured".into());
        let _ = app.update(Message::OpenDrives);
        assert_eq!(app.page, Page::Drives);
        assert!(app.drive_setup.loading);
        app.drive_setup.loading = false;
        assert!(app.can_change_drive());
        app.point = Some("old-point".into());
        app.selected_file = Some("old-file".into());
        app.destination = "/home/aneesh/old".into();
        app.confirmation = "RESTORE".into();
        app.recovery_dialog = true;
        app.operation_command = Some("setup-drive".into());
        let _ = app.update(Message::DriveOperationDone(Ok("Drive ready".into())));
        assert!(
            app.point.is_none()
                && app.selected_file.is_none()
                && app.destination.is_empty()
                && !app.recovery_dialog
                && app.confirmation.is_empty()
        );
        assert_eq!(app.page, Page::Overview);
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    #[test]
    fn setup_failure_keeps_current_drive_and_requires_a_new_plan() {
        let (mut app, _) = App::new();
        app.page = Page::Drives;
        app.status = Some(Status {
            drive: Drive {
                mount: "/backup/original".into(),
                connected: true,
                ..Drive::default()
            },
            ..Status::default()
        });
        app.busy = Some("Preparing drive".into());
        app.operation_command = Some("setup-drive".into());
        app.drive_setup.selected = Some("/dev/vdc".into());
        app.drive_setup.confirmation = "ERASE /dev/vdc".into();
        let _ = app.update(Message::DriveOperationDone(
            Err("Drive disconnected".into()),
        ));
        assert_eq!(app.status.as_ref().unwrap().drive.mount, "/backup/original");
        assert!(
            app.busy.is_none()
                && app.drive_setup.plan.is_none()
                && app.drive_setup.confirmation.is_empty()
        );
        assert_eq!(app.drive_setup.error.as_deref(), Some("Drive disconnected"));
        assert_eq!(app.page, Page::Drives);
    }
}
