use std::{path::Path, time::Duration};

use iced::{
    Alignment, Background, Border, Color, Element, Fill, Font, Subscription, Task, Theme,
    widget::{
        Space, button, column, container, progress_bar, row, rule, scrollable, text, text_input,
    },
};
use iced_shadcn::{AccentColor, ButtonProps, ButtonSize, ButtonVariant};
use serde::Deserialize;

mod backup_targets;
mod chrome;
mod desktop_theme;
mod drives;
mod equal_height_row;
mod motion;
use drives::{DriveInventory, DrivePlan, DriveSetup};

const BG: Color = Color::from_rgb(0.039, 0.055, 0.071);
const PANEL: Color = Color::from_rgb(0.063, 0.083, 0.102);
const RAISED: Color = Color::from_rgb(0.086, 0.110, 0.129);
const LINE: Color = Color::from_rgb(0.145, 0.18, 0.20);
const WHITE: Color = Color::from_rgb(0.91, 0.95, 0.96);
const MUTED: Color = Color::from_rgb(0.53, 0.63, 0.67);
const TEAL: Color = Color::from_rgb(0.32, 0.87, 0.72);
const RED: Color = Color::from_rgb(0.99, 0.52, 0.52);

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Drive {
    label: String,
    mount: String,
    total_bytes: u64,
    used_bytes: u64,
    available_bytes: u64,
    connected: bool,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct RestorePoint {
    id: String,
    created_at: String,
    label: String,
    bytes: u64,
    subvolumes: Vec<String>,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct SnapperConfig {
    name: String,
    subvolume: String,
    timeline_create: Option<bool>,
    timeline_cleanup: Option<bool>,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Status {
    configured: bool,
    drive: Drive,
    points: Vec<RestorePoint>,
    busy: bool,
    progress: Option<String>,
    backup_progress: Option<BackupProgress>,
    last_error: Option<String>,
    skipped_subvolumes: Vec<String>,
    schedule_enabled: bool,
    configs: Vec<SnapperConfig>,
    recovery_pending: Option<String>,
    backup_targets: Vec<String>,
    backup_drives: Vec<BackupDrive>,
}
#[derive(Debug, Clone, Deserialize)]
struct BackupProgress {
    completed: usize,
    total: usize,
}
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct BackupDrive {
    uuid: String,
    label: String,
    mount: String,
    connected: bool,
    mounted: bool,
    unmounted: bool,
}
#[derive(Debug, Clone, Deserialize)]
struct Entry {
    name: String,
    path: String,
    is_dir: bool,
    bytes: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Overview,
    Restore,
    Settings,
    Drives,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BackupState {
    Loading,
    Setup,
    Ready,
    Unavailable,
}
#[derive(Debug, Clone)]
enum Message {
    Chrome(chrome::Event),
    Tick,
    AnimationFrame(std::time::Instant),
    Refresh,
    StatusLoaded(Result<Status, String>),
    Navigate(Page),
    Backup,
    BackupMenu,
    BackupTarget(String, bool),
    OperationDone(Result<String, String>),
    SelectPoint(String),
    Source(String),
    Browse(String),
    Browsed {
        generation: u64,
        point: String,
        source: String,
        path: String,
        result: Result<Vec<Entry>, String>,
    },
    SelectFile(String),
    ClearSelection,
    Destination(String),
    RestoreFile,
    AskRecovery,
    ConfirmText(String),
    ConfirmRecovery,
    DismissRecovery,
    Reboot,
    CancelRecovery,
    Schedule(bool),
    DismissNotice,
    OpenDrives,
    RefreshDrives,
    DrivesLoaded {
        generation: u64,
        result: Result<DriveInventory, String>,
    },
    SelectNewDrive(String),
    DrivePlanned {
        generation: u64,
        device: String,
        result: Result<DrivePlan, String>,
    },
    DriveConfirmation(String),
    CancelDrivePlan,
    ConfirmEraseDrive,
    UseDrive(String),
    UnmountDrive(String),
    DriveOperationDone(Result<String, String>),
}

struct App {
    page: Page,
    status: Option<Status>,
    fetching: bool,
    status_error: Option<String>,
    browser_error: Option<String>,
    operation_command: Option<String>,
    busy: Option<String>,
    notice: Option<(bool, String)>,
    point: Option<String>,
    source: String,
    directory: String,
    entries: Vec<Entry>,
    displayed_folder: Option<(String, String)>,
    browse_generation: u64,
    selected_file: Option<String>,
    destination: String,
    browsing: bool,
    recovery_dialog: bool,
    confirmation: String,
    drive_setup: DriveSetup,
    drive_return_page: Page,
    backup_selection: Option<Vec<String>>,
    motion: motion::ActivityMotion,
    chrome: chrome::WindowChrome,
    shadcn: iced_shadcn::Theme,
}

fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("Snap Back")
        .theme(|_: &App| Theme::Dark)
        .style(|_: &App, _: &Theme| iced::theme::Style {
            background_color: Color::TRANSPARENT,
            text_color: WHITE,
        })
        .subscription(App::subscription)
        .window_size((1180.0, 800.0))
        .window(iced::window::Settings {
            size: iced::Size::new(1180.0, 800.0),
            min_size: Some(iced::Size::new(940.0, 680.0)),
            decorations: false,
            transparent: true,
            resizable: true,
            ..Default::default()
        })
        .centered()
        .antialiasing(true)
        .run()
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let app = Self {
            page: Page::Overview,
            status: None,
            fetching: true,
            status_error: None,
            browser_error: None,
            operation_command: None,
            busy: None,
            notice: None,
            point: None,
            source: "home".into(),
            directory: String::new(),
            entries: Vec::new(),
            displayed_folder: None,
            browse_generation: 0,
            selected_file: None,
            destination: String::new(),
            browsing: false,
            recovery_dialog: false,
            confirmation: String::new(),
            drive_setup: DriveSetup::default(),
            drive_return_page: Page::Overview,
            backup_selection: None,
            motion: motion::ActivityMotion::default(),
            chrome: chrome::WindowChrome::default(),
            shadcn: iced_shadcn::Theme::dark(),
        };
        (
            app,
            Task::batch([status_task(), chrome::WindowChrome::load_theme()]),
        )
    }

    fn subscription(&self) -> Subscription<Message> {
        let interval = if self.backup_fraction().is_some() {
            1
        } else {
            5
        };
        let polling = iced::time::every(Duration::from_secs(interval)).map(|_| Message::Tick);
        let frames = if self.motion.needs_frames() {
            iced::time::every(Duration::from_millis(16)).map(Message::AnimationFrame)
        } else {
            Subscription::none()
        };
        Subscription::batch([polling, frames, chrome::WindowChrome::subscription()])
    }

    fn operation(&mut self, label: &str, args: Vec<String>) -> Task<Message> {
        if !self.can_operate() {
            return Task::none();
        }
        self.busy = Some(label.into());
        self.operation_command = args.first().cloned();
        self.sync_motion();
        self.notice = None;
        Task::perform(backend(args), |result| {
            Message::OperationDone(result.map(|json| {
                json.get("message")
                    .and_then(|x| x.as_str())
                    .unwrap_or("Backup completed.")
                    .to_owned()
            }))
        })
    }

    fn is_busy(&self) -> bool {
        self.busy.is_some() || self.status.as_ref().is_some_and(|s| s.busy)
    }

    fn backup_fraction(&self) -> Option<f32> {
        if let Some(progress) = self
            .status
            .as_ref()
            .and_then(|s| s.backup_progress.as_ref())
        {
            return Some(progress.completed as f32 / progress.total.max(1) as f32);
        }
        (self.operation_command.as_deref() == Some("backup")).then_some(0.0)
    }

    fn sync_motion(&mut self) {
        self.motion
            .set_activity(self.is_busy(), self.backup_fraction());
    }

    fn reset_browser(&mut self) {
        self.browse_generation = self.browse_generation.wrapping_add(1);
        self.directory.clear();
        self.entries.clear();
        self.displayed_folder = None;
        self.selected_file = None;
        self.destination.clear();
        self.browser_error = None;
        self.browsing = false;
        self.recovery_dialog = false;
        self.confirmation.clear();
    }

    fn backup_state(&self) -> BackupState {
        if self.status.as_ref().is_some_and(|s| s.configured) {
            BackupState::Ready
        } else if self.status_error.is_some() {
            BackupState::Unavailable
        } else if self.status.is_some() {
            BackupState::Setup
        } else {
            BackupState::Loading
        }
    }

    fn can_operate(&self) -> bool {
        self.status.as_ref().is_some_and(|s| s.configured)
            && self.status_error.is_none()
            && !self.is_busy()
            && !self.browsing
    }

    fn can_read_backups(&self) -> bool {
        self.can_choose_backup() && !self.browsing
    }

    fn can_choose_backup(&self) -> bool {
        self.status_error.is_none()
            && !self.is_busy()
            && self
                .status
                .as_ref()
                .is_some_and(|s| s.configured && s.drive.connected)
    }

    fn can_backup(&self) -> bool {
        self.can_choose_backup_targets()
            && self.status.as_ref().is_some_and(|status| {
                status.backup_drives.iter().any(|drive| {
                    !drive.unmounted
                        && drive.connected
                        && self.backup_targets().contains(&drive.uuid)
                })
            })
    }

    fn can_recover(&self) -> bool {
        self.can_read_backups()
            && self
                .status
                .as_ref()
                .is_some_and(|s| s.recovery_pending.is_none())
    }

    fn browse(&mut self, path: String) -> Task<Message> {
        let Some(point) = self.point.clone() else {
            return Task::none();
        };
        self.directory = path.clone();
        self.selected_file = None;
        self.destination.clear();
        self.browser_error = None;
        // Keep the last completed listing on screen until its replacement is
        // ready. Its actions remain disabled while a request is outstanding.
        self.browsing = true;
        self.browse_generation = self.browse_generation.wrapping_add(1);
        let generation = self.browse_generation;
        let source = self.source.clone();
        let requested_point = point.clone();
        let requested_source = source.clone();
        let requested_path = path.clone();
        Task::perform(
            async move {
                let value = backend(vec!["browse".into(), point, source, path]).await?;
                serde_json::from_value(value).map_err(|e| format!("Could not read file list: {e}"))
            },
            move |result| Message::Browsed {
                generation,
                point: requested_point.clone(),
                source: requested_source.clone(),
                path: requested_path.clone(),
                result,
            },
        )
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Chrome(event) => return self.chrome.update(event),
            Message::AnimationFrame(now) => self.motion.tick(now),
            Message::Refresh if self.page == Page::Drives => {
                return self.refresh_drives();
            }
            Message::Tick | Message::Refresh => {
                if !self.fetching && !self.browsing {
                    self.fetching = true;
                    return status_task();
                }
            }
            Message::StatusLoaded(result) => {
                self.fetching = false;
                match result {
                    Ok(status) => {
                        self.status_error = None;
                        self.reconcile_backup_selection(&status);
                        let next_point = self
                            .point
                            .as_ref()
                            .filter(|id| status.points.iter().any(|p| &p.id == *id))
                            .cloned()
                            .or_else(|| status.points.first().map(|p| p.id.clone()));
                        let drive_changed = self
                            .status
                            .as_ref()
                            .is_some_and(|s| s.drive.mount != status.drive.mount);
                        let changed = drive_changed || next_point != self.point;
                        if changed {
                            if self.point.is_some()
                                && next_point.is_some()
                                && self.page == Page::Restore
                            {
                                self.notice = Some((false, "The selected backup is no longer available. Showing the newest backup.".into()));
                            }
                            self.reset_browser();
                            self.point = next_point;
                        }
                        self.status = Some(status);
                        self.sync_motion();
                        if changed
                            && self.page == Page::Restore
                            && self.point.is_some()
                            && self.can_read_backups()
                        {
                            return self.browse(String::new());
                        }
                    }
                    Err(error) => self.status_error = Some(error),
                }
            }
            Message::Navigate(page) => {
                if self
                    .operation_command
                    .as_deref()
                    .is_some_and(|c| c == "setup-drive" || c == "use-drive" || c == "unmount-drive")
                {
                    return Task::none();
                }
                if self.page == Page::Drives && page != Page::Drives {
                    self.clear_drive_plan();
                    self.drive_setup.loading = false;
                }
                if page != Page::Restore {
                    self.recovery_dialog = false;
                    self.confirmation.clear();
                }
                self.page = page;
                if page == Page::Restore
                    && self.point.is_some()
                    && self.entries.is_empty()
                    && self.browser_error.is_none()
                    && self.can_read_backups()
                {
                    return self.browse(String::new());
                }
            }
            Message::Backup => {
                if self.can_backup() {
                    return self.operation("Backing up…", self.backup_arguments());
                }
            }
            // The popover widget handles its own open/close state.
            Message::BackupMenu => {}
            Message::BackupTarget(uuid, selected) => {
                if self.can_choose_backup_targets()
                    && self.status.as_ref().is_some_and(|status| {
                        status.backup_drives.iter().any(|drive| drive.uuid == uuid)
                    })
                {
                    let mut targets = self.backup_targets().to_vec();
                    targets.retain(|target| target != &uuid);
                    if selected {
                        targets.push(uuid);
                    }
                    self.backup_selection = Some(targets);
                }
            }
            Message::OperationDone(result) => {
                self.busy = None;
                if result.is_ok() && self.operation_command.as_deref() == Some("restore-file") {
                    self.selected_file = None;
                    self.destination.clear();
                }
                self.operation_command = None;
                self.sync_motion();
                self.notice = Some(match result {
                    Ok(msg) => (false, msg),
                    Err(error) => (true, error),
                });
                self.fetching = true;
                return status_task();
            }
            Message::SelectPoint(point) => {
                if !self.can_choose_backup()
                    || (self.page == Page::Restore
                        && self.point.as_ref() == Some(&point)
                        && self.browser_error.is_none())
                {
                    return Task::none();
                }
                self.point = Some(point);
                self.page = Page::Restore;
                self.recovery_dialog = false;
                self.confirmation.clear();
                return self.browse(String::new());
            }
            Message::Source(source) => {
                if !self.can_choose_backup() || self.source == source {
                    return Task::none();
                }
                self.source = source;
                return self.browse(String::new());
            }
            Message::Browse(path) => {
                if self.can_read_backups() {
                    return self.browse(path);
                }
            }
            Message::Browsed {
                generation,
                point,
                source,
                path,
                result,
            } => {
                if generation != self.browse_generation
                    || self.point.as_deref() != Some(&point)
                    || self.source != source
                    || self.directory != path
                {
                    return Task::none();
                }
                self.browsing = false;
                match result {
                    Ok(entries) => {
                        self.entries = entries;
                        self.displayed_folder = Some((source, path));
                        self.browser_error = None;
                    }
                    Err(error) => self.browser_error = Some(error),
                }
            }
            Message::SelectFile(path) => {
                if !self.can_read_backups() || self.browser_error.is_some() {
                    return Task::none();
                }
                self.destination = self.default_destination(&path);
                self.selected_file = Some(path);
            }
            Message::ClearSelection => {
                self.selected_file = None;
                self.destination.clear();
            }
            Message::Destination(path) => self.destination = path,
            Message::RestoreFile => {
                if !self.can_read_backups() || self.browser_error.is_some() {
                    return Task::none();
                }
                if let (Some(point), Some(path)) = (self.point.clone(), self.selected_file.clone())
                {
                    if !Path::new(&self.destination).is_absolute() {
                        self.notice = Some((
                            true,
                            "Enter a full path for a new folder, starting with /.".into(),
                        ));
                        return Task::none();
                    }
                    return self.operation(
                        "Copying selected item…",
                        vec![
                            "restore-file".into(),
                            point,
                            self.source.clone(),
                            path,
                            "--to".into(),
                            self.destination.clone(),
                        ],
                    );
                }
            }
            Message::AskRecovery => {
                if !self.can_recover() || self.point.is_none() {
                    return Task::none();
                }
                self.recovery_dialog = true;
                self.confirmation.clear();
            }
            Message::ConfirmText(value) => self.confirmation = value,
            Message::ConfirmRecovery => {
                if self.confirmation == "RESTORE"
                    && self.can_recover()
                    && let Some(point) = self.point.clone()
                {
                    self.recovery_dialog = false;
                    return self.operation(
                        "Preparing system recovery…",
                        vec!["prepare-recovery".into(), point],
                    );
                }
            }
            Message::DismissRecovery => self.recovery_dialog = false,
            Message::Reboot => {
                return self.operation("Restarting into recovery…", vec!["reboot".into()]);
            }
            Message::CancelRecovery => {
                return self.operation("Cancelling recovery…", vec!["cancel-recovery".into()]);
            }
            Message::Schedule(enabled) => {
                return self.operation(
                    "Updating automatic backups…",
                    vec!["schedule".into(), if enabled { "on" } else { "off" }.into()],
                );
            }
            Message::DismissNotice => self.notice = None,
            Message::OpenDrives => {
                if !self.can_open_drives() {
                    return Task::none();
                }
                self.reset_browser();
                self.drive_return_page = self.page;
                self.page = Page::Drives;
                return self.refresh_drives();
            }
            Message::RefreshDrives => {
                if self.page == Page::Drives {
                    return self.refresh_drives();
                }
            }
            Message::DrivesLoaded { generation, result } => {
                if self.page != Page::Drives || generation != self.drive_setup.generation {
                    return Task::none();
                }
                self.drive_setup.loading = false;
                match result {
                    Ok(inventory) => {
                        self.drive_setup.inventory = Some(inventory);
                        self.drive_setup.error = None;
                    }
                    Err(error) => {
                        self.drive_setup.inventory = None;
                        self.drive_setup.error = Some(error);
                    }
                }
            }
            Message::SelectNewDrive(device) => {
                if !self.can_change_drive()
                    || !self.drive_setup.inventory.as_ref().is_some_and(|i| {
                        i.candidates
                            .iter()
                            .any(|d| d.device == device && d.eligible)
                    })
                {
                    return Task::none();
                }
                self.clear_drive_plan();
                self.drive_setup.error = None;
                self.drive_setup.selected = Some(device.clone());
                self.drive_setup.planning = true;
                let generation = self.drive_setup.generation;
                let requested = device.clone();
                return Task::perform(
                    async move {
                        let value =
                            backend(vec!["plan-drive".into(), "--device".into(), requested])
                                .await?;
                        serde_json::from_value(value)
                            .map_err(|e| format!("Could not check the selected drive: {e}"))
                    },
                    move |result| Message::DrivePlanned {
                        generation,
                        device: device.clone(),
                        result,
                    },
                );
            }
            Message::DrivePlanned {
                generation,
                device,
                result,
            } => {
                if self.page != Page::Drives
                    || generation != self.drive_setup.generation
                    || self.drive_setup.selected.as_deref() != Some(device.as_str())
                {
                    return Task::none();
                }
                self.drive_setup.planning = false;
                match result {
                    Ok(plan) if plan.device == device => self.drive_setup.plan = Some(plan),
                    Ok(_) => {
                        self.clear_drive_plan();
                        self.drive_setup.error = Some(
                            "The selected drive changed. Refresh the drive list and try again."
                                .into(),
                        );
                    }
                    Err(error) => {
                        self.clear_drive_plan();
                        self.drive_setup.error = Some(error);
                    }
                }
                return reset_main_scroll();
            }
            Message::DriveConfirmation(value) => self.drive_setup.confirmation = value,
            Message::CancelDrivePlan => {
                if !self.is_busy() {
                    self.clear_drive_plan();
                    self.drive_setup.error = None;
                }
            }
            Message::ConfirmEraseDrive => {
                if !self.erase_is_confirmed() {
                    return Task::none();
                }
                let plan = self.drive_setup.plan.as_ref().unwrap().clone();
                return self.start_drive_operation(
                    "Erasing and preparing backup drive…",
                    vec![
                        "setup-drive".into(),
                        "--token".into(),
                        plan.token,
                        "--confirm".into(),
                        self.drive_setup.confirmation.clone(),
                    ],
                );
            }
            Message::UnmountDrive(uuid) => {
                if self.can_change_drive()
                    && self
                        .drive_setup
                        .inventory
                        .as_ref()
                        .is_some_and(|i| i.registered.iter().any(|d| d.uuid == uuid && d.mounted))
                {
                    return self.start_drive_operation(
                        "Unmounting backup drive…",
                        vec!["unmount-drive".into(), uuid],
                    );
                }
            }
            Message::UseDrive(uuid) => {
                if !self.can_change_drive()
                    || !self.drive_setup.inventory.as_ref().is_some_and(|i| {
                        i.registered.iter().any(|d| {
                            d.uuid == uuid
                                && d.connected
                                && (!d.active || !d.mounted || d.unmounted)
                        })
                    })
                {
                    return Task::none();
                }
                return self.start_drive_operation(
                    "Opening drive backups…",
                    vec!["use-drive".into(), uuid],
                );
            }
            Message::DriveOperationDone(result) => {
                self.busy = None;
                let unmounted = self.operation_command.as_deref() == Some("unmount-drive");
                let added_drive = self.operation_command.as_deref() == Some("setup-drive");
                self.operation_command = None;
                match result {
                    Ok(message) => {
                        if added_drive {
                            self.backup_selection = None;
                        }
                        self.clear_drive_plan();
                        self.drive_setup.inventory = None;
                        self.drive_setup.error = None;
                        self.reset_browser();
                        self.point = None;
                        self.status = None;
                        self.status_error = None;
                        self.page = if unmounted {
                            Page::Drives
                        } else if added_drive {
                            Page::Overview
                        } else {
                            Page::Restore
                        };
                        self.notice = Some((false, message));
                    }
                    Err(error) => {
                        self.clear_drive_plan();
                        if unmounted {
                            self.notice = Some((true, error));
                        } else {
                            self.drive_setup.error = Some(error);
                        }
                    }
                }
                self.sync_motion();
                self.fetching = true;
                return Task::batch([
                    status_task(),
                    reset_main_scroll(),
                    if unmounted {
                        self.refresh_drives()
                    } else {
                        Task::none()
                    },
                ]);
            }
        }
        self.sync_motion();
        Task::none()
    }

    fn default_destination(&self, path: &str) -> String {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let mut name = String::new();
        for character in Path::new(path)
            .file_name()
            .unwrap_or_else(|| std::ffi::OsStr::new("files"))
            .to_string_lossy()
            .chars()
            .filter(|c| !c.is_control())
        {
            if name.len() + character.len_utf8() > 80 {
                break;
            }
            name.push(character);
        }
        let date = self
            .status
            .as_ref()
            .and_then(|s| s.points.iter().find(|p| Some(&p.id) == self.point.as_ref()))
            .and_then(|p| chrono::DateTime::parse_from_rfc3339(&p.created_at).ok())
            .map(|d| {
                d.with_timezone(&chrono::Local)
                    .format("%Y%m%d-%H%M")
                    .to_string()
            })
            .unwrap_or_else(|| "backup".into());
        let base = format!("{home}/Restored-{name}-{date}");
        for suffix in 0..1000 {
            let candidate = if suffix == 0 {
                base.clone()
            } else {
                format!("{base}-{}", suffix + 1)
            };
            if !Path::new(&candidate).exists() {
                return candidate;
            }
        }
        format!("{base}-{}", chrono::Local::now().timestamp_millis())
    }

    fn action<'a>(
        &self,
        label: impl Into<String>,
        message: Message,
        variant: ButtonVariant,
        enabled: bool,
    ) -> Element<'a, Message> {
        iced_shadcn::button(
            label.into(),
            enabled.then_some(message),
            ButtonProps::new()
                .variant(variant)
                .size(ButtonSize::Size3)
                .color(AccentColor::Teal),
            &self.shadcn,
        )
        .into()
    }

    fn view(&self) -> Element<'_, Message> {
        let sidebar = self.sidebar();
        let mut content = column![self.header()].spacing(24);
        // Keep the hero at a stable tree position and screen location while
        // progress/notice cards enter below it. The primary action never jumps.
        if self.page == Page::Overview
            && let Some(status) = &self.status
            && status.configured
        {
            content = content.push(self.overview_hero(status));
        }
        if let Some(error) = &self.status_error
            && self.page != Page::Drives
        {
            content = content.push(card(
                column![
                    text("Couldn’t check backups").size(20),
                    text(error).size(13).color(MUTED),
                    text(if self.status.as_ref().is_some_and(|s| s.configured) {
                        "Showing the last available information. Refresh to try again."
                    } else {
                        "Refresh to try again, or choose a backup drive."
                    })
                    .size(13)
                    .color(MUTED),
                    row![
                        self.action(
                            "Try again",
                            Message::Refresh,
                            ButtonVariant::Default,
                            !self.browsing
                        ),
                        self.action(
                            "Choose backup drive",
                            Message::OpenDrives,
                            ButtonVariant::Outline,
                            self.can_open_drives()
                        )
                    ]
                    .spacing(10)
                    .align_y(Alignment::Center)
                ]
                .spacing(14),
            ));
        }
        if let Some((error, message)) = &self.notice {
            content = content.push(
                container(
                    row![
                        text(if *error { "!" } else { "✓" })
                            .size(20)
                            .color(if *error { RED } else { TEAL }),
                        text(message).size(14).width(Fill),
                        button(text("×").size(20))
                            .on_press(Message::DismissNotice)
                            .style(button::text),
                    ]
                    .spacing(14)
                    .align_y(Alignment::Center),
                )
                .padding(16)
                .style(move |_| container::Style {
                    background: Some(Background::Color(if *error {
                        Color::from_rgb(0.18, 0.08, 0.10)
                    } else {
                        Color::from_rgb(0.05, 0.15, 0.13)
                    })),
                    border: Border {
                        color: if *error {
                            RED.scale_alpha(0.3)
                        } else {
                            TEAL.scale_alpha(0.3)
                        },
                        width: 1.0,
                        radius: 10.0.into(),
                    },
                    ..Default::default()
                }),
            );
        }
        let progress = self.status.as_ref().and_then(|s| s.progress.as_deref());
        let activity = progress
            .or(self.busy.as_deref())
            .or_else(|| self.is_busy().then_some("Operation in progress…"));
        if let Some(operation) = activity {
            content = content.push(card(
                row![
                    text("●")
                        .size(12)
                        .color(TEAL.scale_alpha(0.6 + 0.4 * self.motion.pulse())),
                    text(operation).size(15).color(TEAL)
                ]
                .spacing(10)
                .align_y(Alignment::Center),
            ));
        }
        if matches!(self.page, Page::Overview | Page::Restore)
            && let Some(status) = &self.status
            && !status.skipped_subvolumes.is_empty()
        {
            let mut warning = column![
                text("Subvolumes that will be skipped").size(18),
                text("These subvolumes are outside your backup configuration.")
                    .size(13)
                    .color(MUTED),
            ]
            .spacing(8);
            for subvolume in &status.skipped_subvolumes {
                warning = warning.push(text(subvolume).size(13));
            }
            content = content.push(card(warning));
        }
        if let Some(pending) = self
            .status
            .as_ref()
            .and_then(|s| s.recovery_pending.as_ref())
        {
            content = content.push(card(column![
                text("System recovery is ready").size(20).color(TEAL),
                text(format!("Backup: {}. Restarting will replace the system and home directories. Save your work first.", self.status.as_ref().and_then(|s| s.points.iter().find(|p| &p.id == pending)).map(|p| display_time(&p.created_at)).unwrap_or_else(|| "selected backup".into()))).size(14).color(MUTED),
                row![self.action("Restart and restore", Message::Reboot, ButtonVariant::Default, self.can_operate()), self.action("Cancel recovery", Message::CancelRecovery, ButtonVariant::Outline, self.can_operate())].spacing(10)
            ].spacing(14)));
        }
        if self.page == Page::Drives {
            content = content.push(self.drive_picker());
        } else {
            match self.backup_state() {
                BackupState::Ready => {
                    content = content.push(match self.page {
                        Page::Overview => self.overview(),
                        Page::Restore => self.restore(),
                        Page::Settings => self.settings(),
                        Page::Drives => unreachable!(),
                    });
                }
                BackupState::Setup => content = content.push(self.setup_welcome()),
                BackupState::Loading => {
                    content = content.push(card(text("Checking backups…").size(20).color(MUTED)));
                }
                BackupState::Unavailable => {}
            }
        }
        let main_content = container(
            scrollable(container(content).padding(32).width(Fill))
                .id("main-content")
                .width(Fill)
                .height(Fill),
        )
        // Keep the scrollbar clear of the transparent bottom corner.
        .padding(iced::Padding {
            bottom: self.chrome.corner_radius(),
            ..iced::Padding::ZERO
        })
        .width(Fill)
        .height(Fill);
        let body: Element<'_, Message> = if self.chrome.has_leading_controls() {
            // Leading controls belong at the window edge. Give the caption its
            // own row above both panes so it cannot overlap the sidebar brand.
            column![
                self.chrome.titlebar(),
                row![sidebar, self.chrome.content_pane(main_content)].height(Fill)
            ]
            .width(Fill)
            .height(Fill)
            .into()
        } else {
            row![
                sidebar,
                self.chrome.content_pane(
                    column![self.chrome.titlebar(), main_content]
                        .width(Fill)
                        .height(Fill)
                )
            ]
            .width(Fill)
            .height(Fill)
            .into()
        };
        self.chrome.frame(body)
    }

    fn sidebar(&self) -> Element<'_, Message> {
        let mut nav = column![
            chrome::drag_region(
                row![
                    container(text("↶").size(30).color(TEAL))
                        .padding([3, 10])
                        .style(|_| container::Style {
                            background: Some(Color::from_rgb(0.06, 0.17, 0.15).into()),
                            border: Border::default().rounded(12),
                            ..Default::default()
                        }),
                    column![
                        text("Snap Back").size(21).font(Font {
                            weight: iced::font::Weight::Semibold,
                            ..Font::DEFAULT
                        }),
                        text("System backups").size(11).color(MUTED)
                    ]
                    .spacing(3)
                ]
                .spacing(11)
                .align_y(Alignment::Center)
            ),
            Space::new().height(32),
            text("NAVIGATION").size(10).color(MUTED),
        ]
        .spacing(12);
        for (page, icon, title) in [
            (Page::Overview, "◈", "Overview"),
            (Page::Restore, "↶", "Restore"),
            (Page::Settings, "⚙", "Settings"),
        ] {
            let selected = self.page == page;
            nav = nav.push(
                iced_anim::widget::button(
                    row![text(icon).size(19).width(26), text(title).size(14)]
                        .spacing(9)
                        .align_y(Alignment::Center),
                )
                .padding([13, 14])
                .width(Fill)
                .on_press_maybe(
                    (!self.operation_command.as_deref().is_some_and(|c| {
                        c == "setup-drive" || c == "use-drive" || c == "unmount-drive"
                    }))
                    .then_some(Message::Navigate(page)),
                )
                .style(move |_, status| nav_style(selected, status))
                .animation(
                    iced_anim::transition::Easing::EASE_OUT
                        .with_duration(Duration::from_millis(150)),
                ),
            );
        }
        nav = nav
            .push(Space::new().height(Fill))
            .push(rule::horizontal(1));
        let connected_drives = self
            .status
            .as_ref()
            .map(|status| {
                status
                    .backup_drives
                    .iter()
                    .filter(|drive| drive.connected)
                    .count()
            })
            .unwrap_or(0);
        let connected = connected_drives > 0;
        nav = nav.push(
            row![
                text("●")
                    .size(10)
                    .color(if connected { TEAL } else { MUTED }),
                column![
                    text(if self.status_error.is_some() {
                        "Status unavailable".into()
                    } else if connected {
                        format!(
                            "{connected_drives} backup {} connected",
                            if connected_drives == 1 {
                                "drive"
                            } else {
                                "drives"
                            }
                        )
                    } else if self.status.as_ref().is_some_and(|s| !s.configured) {
                        "Choose a backup drive".into()
                    } else if self.status.is_none() && self.status_error.is_none() {
                        "Checking backup drives…".into()
                    } else {
                        "No backup drives connected".into()
                    })
                    .size(11),
                    text("Btrfs · CachyOS · Snapper").size(10).color(MUTED)
                ]
                .spacing(6)
            ]
            .spacing(9)
            .align_y(Alignment::Center),
        );
        let radius = self.chrome.corner_radius();
        let leading_controls = self.chrome.has_leading_controls();
        container(nav)
            .padding([30, 22])
            .width(238)
            .height(Fill)
            .style(move |_| container::Style {
                background: Some(PANEL.into()),
                border: Border {
                    color: LINE,
                    width: 1.0,
                    radius: iced::border::Radius {
                        top_left: if leading_controls { 0.0 } else { radius },
                        bottom_left: radius,
                        ..Default::default()
                    },
                },
                ..Default::default()
            })
            .into()
    }

    fn header(&self) -> Element<'_, Message> {
        let (title, subtitle) = match self.page {
            Page::Overview => ("Backups", "Back up the system and home directories."),
            Page::Restore => (
                "Restore",
                "Choose a backup to recover files or restore the system.",
            ),
            Page::Settings => ("Settings", "Automatic backups and local snapshots."),
            Page::Drives => ("Backup drive", "Choose a saved drive or set up a new one."),
        };
        row![
            column![
                text(title).size(30).font(Font {
                    weight: iced::font::Weight::Semibold,
                    ..Font::DEFAULT
                }),
                text(subtitle).size(14).color(MUTED)
            ]
            .spacing(9)
            .width(Fill),
            self.action(
                if self.page == Page::Drives && self.drive_setup.loading {
                    "Refreshing…"
                } else {
                    "Refresh"
                },
                Message::Refresh,
                ButtonVariant::Ghost,
                if self.page == Page::Drives {
                    self.can_open_drives() && !self.drive_setup.loading
                } else {
                    !self.browsing
                }
            )
        ]
        .align_y(Alignment::Center)
        .into()
    }

    fn overview_hero(&self, status: &Status) -> Element<'_, Message> {
        let has_backup = !status.points.is_empty();
        let destinations_available = status.backup_drives.iter().any(|drive| {
            !drive.unmounted && drive.connected && self.backup_targets().contains(&drive.uuid)
        });
        let headline = if status.recovery_pending.is_some() {
            "System recovery prepared"
        } else if self.is_busy() {
            "Operation in progress"
        } else if !destinations_available {
            "Choose a backup drive"
        } else if !status.drive.connected {
            "Ready to back up"
        } else if has_backup {
            "Latest backup"
        } else {
            "No backups yet"
        };
        let last = status
            .points
            .first()
            .map(|p| display_time(&p.created_at))
            .unwrap_or_else(|| "No completed backup".into());
        container(
            row![
                self.motion.hero(),
                column![
                    text(if !destinations_available {
                        "NO AVAILABLE DESTINATION"
                    } else if has_backup {
                        "BACKUP AVAILABLE"
                    } else {
                        "NO BACKUPS"
                    })
                    .size(11)
                    .color(TEAL),
                    text(headline).size(24),
                    text(last).size(13).color(MUTED),
                    row![pill("System"), pill("Home directories")].spacing(8)
                ]
                .spacing(13)
                .width(Fill),
                self.backup_button()
            ]
            .spacing(24)
            .align_y(Alignment::Center),
        )
        .padding(26)
        .style(|_| container::Style {
            background: Some(Color::from_rgb(0.052, 0.105, 0.105).into()),
            border: Border {
                color: TEAL.scale_alpha(0.22),
                width: 1.0,
                radius: 16.0.into(),
            },
            ..Default::default()
        })
        .into()
    }

    fn setup_welcome(&self) -> Element<'_, Message> {
        card(
            row![
                self.motion.hero(),
                column![
                    text("Set up your first backup").size(26),
                    text("Choose a drive to keep backups of your system and home directories.")
                        .size(14).color(MUTED),
                    text("Once it’s set up, Snap Back saves a backup every hour while your drive is connected.")
                        .size(13).color(MUTED),
                    row![self.action(
                        "Choose backup drive", Message::OpenDrives,
                        ButtonVariant::Default, self.can_open_drives()
                    )].spacing(10)
                ].spacing(16).width(Fill)
            ].spacing(26).align_y(Alignment::Center)
        )
    }

    fn overview(&self) -> Element<'_, Message> {
        let Some(status) = &self.status else {
            return Space::new().into();
        };
        let used = if status.drive.total_bytes > 0 {
            (status.drive.used_bytes as f32 / status.drive.total_bytes as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let drive_card = card(
            column![
                row![
                    text("VIEWING BACKUPS ON").size(10).color(MUTED),
                    Space::new().width(Fill),
                    text(if status.drive.connected {
                        "● Connected"
                    } else {
                        "○ Disconnected"
                    })
                    .size(11)
                    .color(if status.drive.connected { TEAL } else { RED })
                ],
                text(if status.drive.label.is_empty() {
                    "Snap Back"
                } else {
                    &status.drive.label
                })
                .size(22),
                text(&status.drive.mount).size(11).color(MUTED),
                progress_bar(0.0..=1.0, used)
                    .girth(6)
                    .style(|_: &Theme| progress_bar::Style {
                        background: RAISED.into(),
                        bar: TEAL.into(),
                        border: Border {
                            radius: 3.0.into(),
                            ..Default::default()
                        },
                    }),
                text(if status.drive.connected {
                    format!(
                        "{} free of {}",
                        human_bytes(status.drive.available_bytes),
                        human_bytes(status.drive.total_bytes)
                    )
                } else {
                    "Connect this drive to browse its backups.".into()
                })
                .size(12)
                .color(MUTED),
                self.action(
                    "Manage drives",
                    Message::OpenDrives,
                    ButtonVariant::Outline,
                    self.can_open_drives()
                )
            ]
            .spacing(12),
        );
        let schedule_card = card(
            column![
                text("AUTOMATIC BACKUPS").size(10).color(MUTED),
                text(if status.schedule_enabled {
                    "Every hour"
                } else {
                    "Paused"
                })
                .size(22),
                text("24 hourly  ·  7 daily  ·  4 weekly")
                    .size(12)
                    .color(MUTED),
                text(if status.schedule_enabled {
                    "Runs while the machine is on, using your last backup’s drives."
                } else {
                    "Manual backups remain available."
                })
                .size(12)
                .color(MUTED),
                self.action(
                    "Backup settings",
                    Message::Navigate(Page::Settings),
                    ButtonVariant::Ghost,
                    true
                )
            ]
            .spacing(12),
        );
        let mut history = column![
            row![
                column![
                    text("Restore points").size(21),
                    text("Choose a backup to browse its files.")
                        .size(12)
                        .color(MUTED)
                ]
                .spacing(7)
                .width(Fill),
                pill(format!(
                    "{} restore point{}",
                    status.points.len(),
                    if status.points.len() == 1 { "" } else { "s" }
                ))
            ]
            .align_y(Alignment::Center),
            Space::new().height(6)
        ]
        .spacing(14);
        if status.points.is_empty() {
            history = history.push(card(
                column![
                    text("No completed backups").size(16),
                    text("Connect the backup drive and select Back up now.")
                        .size(13)
                        .color(MUTED)
                ]
                .spacing(10),
            ));
        } else {
            for point in status.points.iter().take(8) {
                history = history.push(self.timeline_row(point));
            }
        }
        if let Some(error) = &status.last_error {
            history = history.push(
                text(format!("Last operation error: {error}"))
                    .size(13)
                    .color(RED),
            );
        }
        column![equal_height_row::cards(drive_card, schedule_card), history]
            .spacing(24)
            .into()
    }

    fn timeline_row<'a>(&'a self, point: &'a RestorePoint) -> Element<'a, Message> {
        let label = if point.label.is_empty() {
            "System and home"
        } else {
            &point.label
        };
        button(
            row![
                container(text("↶").size(23).color(TEAL))
                    .padding(12)
                    .style(|_| container::Style {
                        background: Some(RAISED.into()),
                        border: Border::default().rounded(12),
                        ..Default::default()
                    }),
                column![
                    text(display_time(&point.created_at)).size(15),
                    text(label).size(12).color(MUTED)
                ]
                .spacing(6)
                .width(Fill),
                column![
                    text("System + home").size(11).color(TEAL),
                    text(if point.bytes > 0 {
                        format!("{} contents", human_bytes(point.bytes))
                    } else {
                        point.subvolumes.join(" + ")
                    })
                    .size(11)
                    .color(MUTED)
                ]
                .spacing(6),
                text("Browse").size(12).color(MUTED)
            ]
            .spacing(16)
            .align_y(Alignment::Center),
        )
        .on_press_maybe(
            self.can_read_backups()
                .then(|| Message::SelectPoint(point.id.clone())),
        )
        .padding(14)
        .width(Fill)
        .style(|_, status| nav_style(false, status))
        .into()
    }

    fn restore(&self) -> Element<'_, Message> {
        let Some(status) = &self.status else {
            return text("Backup status is unavailable. Refresh to retry.")
                .color(MUTED)
                .into();
        };
        if !status.drive.connected {
            return card(
                column![
                    text("Backup drive disconnected").size(23),
                    text("Connect the configured backup drive to browse or restore its backups.")
                        .size(14)
                        .color(MUTED),
                    self.action(
                        "Change drive",
                        Message::OpenDrives,
                        ButtonVariant::Outline,
                        self.can_open_drives()
                    )
                ]
                .spacing(16),
            );
        }
        if status.points.is_empty() {
            return card(
                column![
                    text("No backups available").size(23),
                    text("Create a backup before recovering files or the system.").color(MUTED),
                    row![
                        self.backup_button(),
                        self.action(
                            "Browse another drive",
                            Message::OpenDrives,
                            ButtonVariant::Ghost,
                            self.can_open_drives()
                        )
                    ]
                    .spacing(12)
                    .align_y(Alignment::Center)
                ]
                .spacing(16),
            );
        }
        if self.recovery_dialog {
            let when = self
                .status
                .as_ref()
                .and_then(|s| s.points.iter().find(|p| Some(&p.id) == self.point.as_ref()))
                .map(|p| display_time(&p.created_at))
                .unwrap_or_else(|| "the selected backup".into());
            return card(column![
                text("Restore this system?").size(24).color(RED),
                text(format!("Backup: {when}")).size(16),
                text("This replaces the system, all home directories, and other configured volumes. Changes made since this backup will no longer be in the active system. The current volumes are retained for manual recovery.").size(14),
                text("Preparation copies the backup onto the system disk. You can cancel before restarting. Save your work before the recovery restart.").size(13).color(MUTED),
                text("Type RESTORE to prepare. This does not restart the machine.").size(13),
                text_input("RESTORE", &self.confirmation).on_input(Message::ConfirmText).padding(12),
                row![self.action("Prepare system recovery", Message::ConfirmRecovery, ButtonVariant::Destructive, self.confirmation == "RESTORE" && self.can_recover()), self.action("Cancel", Message::DismissRecovery, ButtonVariant::Outline, true)].spacing(10)
            ].spacing(18));
        }
        let mut points = column![text("RESTORE POINTS").size(10).color(MUTED)].spacing(10);
        for point in &status.points {
            let selected = self.point.as_deref() == Some(point.id.as_str());
            points = points.push(
                button(
                    column![
                        text(display_time(&point.created_at)).size(12),
                        text(if selected {
                            "● Selected"
                        } else {
                            "Root + home"
                        })
                        .size(10)
                        .color(if selected { TEAL } else { MUTED })
                    ]
                    .spacing(8),
                )
                .on_press_maybe(
                    self.can_choose_backup()
                        .then(|| Message::SelectPoint(point.id.clone())),
                )
                .padding(13)
                .width(Fill)
                .style(move |_, state| nav_style(selected, state)),
            );
        }
        let (displayed_source, displayed_directory) = if self.browsing {
            self.displayed_folder
                .as_ref()
                .map(|(source, path)| (source.as_str(), path.as_str()))
                .unwrap_or((&self.source, &self.directory))
        } else {
            (self.source.as_str(), self.directory.as_str())
        };
        let title = if displayed_source == "home" {
            "Home directories"
        } else {
            "System"
        };
        let up = Path::new(&self.directory)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string();
        let mut files = column![
            row![
                self.action(
                    "Home",
                    Message::Source("home".into()),
                    if self.source == "home" {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    },
                    self.can_choose_backup()
                ),
                self.action(
                    "Root",
                    Message::Source("root".into()),
                    if self.source == "root" {
                        ButtonVariant::Secondary
                    } else {
                        ButtonVariant::Ghost
                    },
                    self.can_choose_backup()
                ),
                Space::new().width(Fill),
                self.action(
                    "↑ Up",
                    Message::Browse(up),
                    ButtonVariant::Ghost,
                    !self.directory.is_empty() && self.can_read_backups()
                )
            ]
            .spacing(8)
            .align_y(Alignment::Center),
            row![
                text(format!("{title} /{displayed_directory}"))
                    .size(12)
                    .color(MUTED)
                    .width(Fill),
                text(if self.browsing { "Updating…" } else { "" })
                    .size(11)
                    .color(MUTED)
            ]
            .align_y(Alignment::Center),
            rule::horizontal(1)
        ]
        .spacing(12);
        if let Some(path) = &self.selected_file {
            files = files.push(row![text(format!("Selected: /{path}")).size(13).color(TEAL).width(Fill), self.action("Clear", Message::ClearSelection, ButtonVariant::Ghost, self.can_read_backups())].spacing(8).align_y(Alignment::Center))
                .push(text("NEW DESTINATION FOLDER").size(10).color(MUTED))
                .push(text_input("/home/you/Restored-files", &self.destination).on_input(Message::Destination).padding(12).size(12))
                .push(text("A new folder will be created here. The selected item is copied inside it; existing folders are never overwritten.").size(11).color(MUTED))
                .push(self.action("Copy selected item", Message::RestoreFile, ButtonVariant::Default, self.can_read_backups() && !self.destination.trim().is_empty() && self.browser_error.is_none()));
        }

        let listing: Element<'_, Message> = if let Some(error) = &self.browser_error {
            column![text("Could not open this folder").size(15).color(RED)]
                .push(text(error).size(12).color(MUTED))
                .push(self.action(
                    "Retry folder",
                    Message::Browse(self.directory.clone()),
                    ButtonVariant::Outline,
                    self.can_read_backups(),
                ))
                .spacing(12)
                .into()
        } else if self.browsing && self.displayed_folder.is_none() {
            text("Loading folder…").size(14).color(MUTED).into()
        } else if self.entries.is_empty() {
            text("This folder is empty.").size(14).color(MUTED).into()
        } else {
            let mut list = column![].spacing(4);
            for entry in &self.entries {
                let selected = self.selected_file.as_deref() == Some(entry.path.as_str());
                let action = if entry.is_dir {
                    Message::Browse(entry.path.clone())
                } else {
                    Message::SelectFile(entry.path.clone())
                };
                let mut line = row![
                    button(
                        row![
                            text(if entry.is_dir { "▣" } else { "□" })
                                .size(18)
                                .color(if entry.is_dir { TEAL } else { MUTED }),
                            text(&entry.name).size(13).width(Fill),
                            text(if entry.is_dir {
                                "Folder".into()
                            } else {
                                human_bytes(entry.bytes)
                            })
                            .size(11)
                            .color(MUTED)
                        ]
                        .spacing(10)
                        .align_y(Alignment::Center)
                    )
                    .width(Fill)
                    .padding(10)
                    .on_press_maybe(self.can_read_backups().then_some(action))
                    .style(move |_, state| nav_style(selected, state))
                ]
                .spacing(4)
                .align_y(Alignment::Center);
                if entry.is_dir {
                    line = line.push(self.action(
                        "Select",
                        Message::SelectFile(entry.path.clone()),
                        ButtonVariant::Ghost,
                        self.can_read_backups(),
                    ));
                }
                list = list.push(line);
            }
            scrollable(list).height(Fill).into()
        };
        files = files.push(container(listing).height(260).width(Fill));
        if !displayed_directory.is_empty() && self.browser_error.is_none() {
            files = files.push(self.action(
                "Select folder",
                Message::SelectFile(self.directory.clone()),
                ButtonVariant::Outline,
                self.can_read_backups() && self.browser_error.is_none(),
            ));
        }
        if self.selected_file.is_none() && self.browser_error.is_none() {
            files = files.push(
                text("Open a folder to browse, or select a file or folder to copy.")
                    .size(12)
                    .color(MUTED),
            );
        }
        let body = column![
            row![
                text(format!("Backups on {}", status.drive.label))
                    .size(13)
                    .color(MUTED)
                    .width(Fill),
                self.action(
                    "Change drive",
                    Message::OpenDrives,
                    ButtonVariant::Ghost,
                    self.can_open_drives()
                )
            ]
            .align_y(Alignment::Center),
            row![
                container(scrollable(points)).max_height(430).width(210),
                container(card(files)).width(Fill)
            ]
            .spacing(18),
            card(
                column![
                    row![
                        column![
                            text("System recovery").size(19),
                            text(
                                "Replace the system and home directories with the selected backup."
                            )
                            .size(12)
                            .color(MUTED)
                        ]
                        .spacing(8)
                        .width(Fill),
                        self.action(
                            "Restore system…",
                            Message::AskRecovery,
                            ButtonVariant::Outline,
                            self.can_recover() && self.point.is_some()
                        )
                    ]
                    .spacing(14)
                    .align_y(Alignment::Center)
                ]
                .spacing(12)
            )
        ]
        .spacing(20);
        body.into()
    }

    fn settings(&self) -> Element<'_, Message> {
        let Some(status) = &self.status else {
            return card(
                column![
                    text("Choose a drive to set up backups.").color(MUTED),
                    self.action(
                        "Choose backup drive",
                        Message::OpenDrives,
                        ButtonVariant::Default,
                        self.can_open_drives()
                    )
                ]
                .spacing(16),
            );
        };
        let schedule = card(column![
            row![column![text("Automatic backups").size(21), text("Run hourly to your saved selection of connected drives while this machine is on.").size(13).color(MUTED)].spacing(9).width(Fill),
                self.action(if status.schedule_enabled { "Pause backups" } else { "Enable backups" }, Message::Schedule(!status.schedule_enabled), ButtonVariant::Outline, self.can_operate())].spacing(14).align_y(Alignment::Center),
            rule::horizontal(1),
            row![retention("24", "hourly backups"), retention("7", "daily backups"), retention("4", "weekly backups")].spacing(16),
            text("Keeps the newest backup in each retained hour, day, and week. Newer backups in the same period replace older ones.").size(12).color(MUTED)
        ].spacing(22));
        let mut configs = column![text("Local Snapper snapshots").size(21), text("Backups use Snapper’s root and home configurations. Local snapshot schedules and cleanup are separate from backup-drive retention.").size(13).color(MUTED)].spacing(15);
        for config in &status.configs {
            configs = configs.push(
                row![
                    column![
                        text(&config.name).size(15),
                        text(&config.subvolume).size(12).color(MUTED)
                    ]
                    .spacing(6)
                    .width(Fill),
                    column![
                        text(format!(
                            "Local timeline: {}",
                            snapper_setting(config.timeline_create)
                        ))
                        .size(11)
                        .color(MUTED),
                        text(format!(
                            "Local cleanup: {}",
                            snapper_setting(config.timeline_cleanup)
                        ))
                        .size(11)
                        .color(MUTED)
                    ]
                    .spacing(6)
                ]
                .align_y(Alignment::Center),
            );
        }
        if status
            .configs
            .iter()
            .any(|config| config.timeline_create.is_none() || config.timeline_cleanup.is_none())
        {
            configs = configs.push(
                text("Some local settings require administrator access to read.")
                    .size(11)
                    .color(MUTED),
            );
        }
        column![
            schedule,
            card(configs),
            card(
                column![
                    text("Backup storage").size(20),
                    text("Choose a saved backup drive or erase and set up a new drive.")
                        .size(13)
                        .color(MUTED),
                    self.action(
                        "Choose backup drive",
                        Message::OpenDrives,
                        ButtonVariant::Outline,
                        self.can_open_drives()
                    ),
                    text(concat!(
                        "Snap Back ",
                        env!("CARGO_PKG_VERSION"),
                        " · CachyOS prototype"
                    ))
                    .size(11)
                    .color(MUTED)
                ]
                .spacing(14)
            )
        ]
        .spacing(20)
        .into()
    }
}

fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(22)
        .width(Fill)
        .style(|_| container::Style {
            background: Some(PANEL.into()),
            border: Border {
                color: LINE,
                width: 1.0,
                radius: 13.0.into(),
            },
            ..Default::default()
        })
        .into()
}
fn pill<'a>(label: impl Into<String>) -> Element<'a, Message> {
    container(text(label.into()).size(10).color(TEAL))
        .padding([6, 9])
        .style(|_| container::Style {
            background: Some(Color::from_rgb(0.07, 0.16, 0.145).into()),
            border: Border {
                color: TEAL.scale_alpha(0.15),
                width: 1.0,
                radius: 5.0.into(),
            },
            ..Default::default()
        })
        .into()
}
fn retention<'a>(value: &'a str, label: &'a str) -> Element<'a, Message> {
    container(
        column![
            text(value).size(32).color(TEAL),
            text(label).size(12).color(MUTED)
        ]
        .spacing(6),
    )
    .width(Fill)
    .into()
}
fn nav_style(selected: bool, status: button::Status) -> button::Style {
    let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: Some(
            if selected {
                Color::from_rgb(0.07, 0.17, 0.155)
            } else if hover {
                RAISED
            } else {
                PANEL
            }
            .into(),
        ),
        text_color: if selected { TEAL } else { WHITE },
        border: Border {
            color: if selected {
                TEAL.scale_alpha(0.2)
            } else {
                LINE.scale_alpha(0.4)
            },
            width: 1.0,
            radius: 9.0.into(),
        },
        ..Default::default()
    }
}
fn snapper_setting(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "on",
        Some(false) => "off",
        None => "not readable",
    }
}
fn human_bytes(value: u64) -> String {
    let amount = value as f64;
    if value >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", amount / 1_073_741_824.0)
    } else if value >= 1024 * 1024 {
        format!("{:.1} MiB", amount / 1_048_576.0)
    } else if value >= 1024 {
        format!("{:.1} KiB", amount / 1024.0)
    } else {
        format!("{value} B")
    }
}
fn display_time(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%d %b %Y · %H:%M")
                .to_string()
        })
        .unwrap_or_else(|_| value.to_owned())
}
fn reset_main_scroll() -> Task<Message> {
    iced::widget::operation::snap_to(
        "main-content",
        iced::widget::operation::RelativeOffset::START,
    )
}

fn status_task() -> Task<Message> {
    Task::perform(
        async {
            let result = backend(vec!["status".into()]).await?;
            serde_json::from_value(result)
                .map_err(|error| format!("Could not read backup status: {error}"))
        },
        Message::StatusLoaded,
    )
}
async fn backend(args: Vec<String>) -> Result<serde_json::Value, String> {
    // Idle status reads need no authentication. Mutating and file-access operations
    // elevate only the fixed helper, never the graphical process.
    let privileged = args.as_slice() != ["status"];
    let mut command = if !privileged {
        tokio::process::Command::new("/usr/local/bin/snap-back")
    } else {
        let mut command = tokio::process::Command::new("/usr/bin/pkexec");
        command.arg("/usr/local/bin/snap-back");
        command
    };
    let output = command
        .args(args)
        .output()
        .await
        .map_err(|error| format!("Could not start Snap Back: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        if privileged {
            if output.status.code() == Some(126) {
                return Err("Administrator authentication was cancelled.".into());
            }
            if stderr.contains("Not authorized") || stderr.contains("No authentication agent found")
            {
                return Err("Administrator access was not granted. Open Snap Back from your desktop session and try again.".into());
            }
        }
        return Err(if stderr.is_empty() {
            format!("The operation failed ({}). Try again.", output.status)
        } else {
            stderr
        });
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Could not read the operation result: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(id: &str) -> Status {
        Status {
            configured: true,
            backup_targets: vec!["drive-a".into()],
            backup_drives: vec![BackupDrive {
                uuid: "drive-a".into(),
                connected: true,
                mounted: true,
                ..BackupDrive::default()
            }],
            drive: Drive {
                connected: true,
                ..Drive::default()
            },
            points: vec![RestorePoint {
                id: id.into(),
                created_at: "2026-11-14T08:00:00Z".into(),
                ..RestorePoint::default()
            }],
            ..Status::default()
        }
    }
    #[test]
    fn first_launch_is_setup_and_background_polls_keep_it_there() {
        let (mut app, _) = App::new();
        assert_eq!(app.backup_state(), BackupState::Loading);
        let _ = app.update(Message::StatusLoaded(Ok(Status::default())));
        for _ in 0..3 {
            assert_eq!(app.backup_state(), BackupState::Setup);
            let _ = app.update(Message::Tick);
            assert!(app.fetching);
            assert_eq!(app.backup_state(), BackupState::Setup);
            let _ = app.update(Message::StatusLoaded(Ok(Status::default())));
            assert!(app.status_error.is_none());
        }
        assert!(!app.can_operate() && !app.can_backup() && !app.can_recover());
        assert!(app.can_open_drives() && app.can_change_drive());
        let _ = app.update(Message::OpenDrives);
        assert_eq!(app.page, Page::Drives);
        assert!(app.drive_setup.loading);
    }

    #[test]
    fn setup_completion_enables_backups_and_real_status_failures_stay_visible() {
        let (mut app, _) = App::new();
        let _ = app.update(Message::StatusLoaded(Ok(Status::default())));
        let _ = app.update(Message::StatusLoaded(
            Err("Unreadable configuration".into()),
        ));
        assert_eq!(app.backup_state(), BackupState::Unavailable);
        let _ = app.update(Message::Tick);
        assert_eq!(app.backup_state(), BackupState::Unavailable);
        let _ = app.update(Message::StatusLoaded(Ok(status("point"))));
        assert_eq!(app.backup_state(), BackupState::Ready);
        assert!(app.can_backup() && app.status_error.is_none());
        let _ = app.update(Message::StatusLoaded(Err("Read failed".into())));
        assert_eq!(app.backup_state(), BackupState::Ready);
        assert_eq!(app.point.as_deref(), Some("point"));
        assert!(!app.can_backup());
    }

    #[test]
    fn backup_click_starts_motion_before_helper_response() {
        let (mut app, _) = App::new();
        app.status = Some(status("point"));
        assert!(!app.motion.needs_frames());
        let _ = app.update(Message::Backup);
        assert_eq!(app.operation_command.as_deref(), Some("backup"));
        assert!(app.motion.needs_frames());
    }
    #[test]
    fn external_operation_status_starts_and_stops_motion() {
        let (mut app, _) = App::new();
        let mut busy = status("point");
        busy.busy = true;
        let _ = app.update(Message::StatusLoaded(Ok(busy)));
        assert!(app.motion.needs_frames());
        let start = std::time::Instant::now();
        for i in 1..=60 {
            let _ = app.update(Message::AnimationFrame(
                start + Duration::from_millis(i * 16),
            ));
        }
        assert!(app.motion.amount() > 0.9);
        let _ = app.update(Message::StatusLoaded(Ok(status("point"))));
        let stop = std::time::Instant::now();
        for i in 1..=600 {
            let _ = app.update(Message::AnimationFrame(
                stop + Duration::from_millis(i * 16),
            ));
        }
        assert!(!app.motion.needs_frames());
    }
    #[test]
    fn changing_restore_point_clears_selected_item_and_confirmation() {
        let (mut app, _) = App::new();
        app.status = Some(status("old"));
        app.point = Some("old".into());
        app.page = Page::Restore;
        app.selected_file = Some("aneesh/report.txt".into());
        app.destination = "/home/aneesh/Recovered".into();
        app.directory = "aneesh".into();
        app.recovery_dialog = true;
        app.confirmation = "RESTORE".into();
        let _ = app.update(Message::StatusLoaded(Ok(status("new"))));
        assert_eq!(app.point.as_deref(), Some("new"));
        assert!(
            app.selected_file.is_none()
                && app.destination.is_empty()
                && app.confirmation.is_empty()
        );
        assert!(!app.recovery_dialog);
    }
    #[test]
    fn old_folder_response_cannot_replace_new_point_contents() {
        let (mut app, _) = App::new();
        app.status = Some(status("new"));
        app.point = Some("new".into());
        app.browsing = true;
        let _ = app.update(Message::Browsed {
            generation: app.browse_generation,
            point: "old".into(),
            source: "home".into(),
            path: "".into(),
            result: Ok(vec![Entry {
                name: "stale".into(),
                path: "stale".into(),
                is_dir: false,
                bytes: 1,
            }]),
        });
        assert!(app.entries.is_empty() && app.browsing);
    }
    #[test]
    fn status_error_disables_actions_until_a_successful_refresh() {
        let (mut app, _) = App::new();
        app.status = Some(status("point"));
        app.point = Some("point".into());
        assert!(app.can_backup());
        let _ = app.update(Message::StatusLoaded(Err("unavailable".into())));
        assert!(!app.can_backup());
        let _ = app.update(Message::StatusLoaded(Ok(status("point"))));
        assert!(app.status_error.is_none() && app.can_backup());
    }
    #[test]
    fn folder_failure_is_distinct_from_an_empty_folder() {
        let (mut app, _) = App::new();
        app.status = Some(status("point"));
        app.point = Some("point".into());
        app.browsing = true;
        let _ = app.update(Message::Browsed {
            generation: app.browse_generation,
            point: "point".into(),
            source: "home".into(),
            path: "".into(),
            result: Err("Authorization cancelled".into()),
        });
        assert!(!app.browsing && app.browser_error.is_some());
        let _ = app.update(Message::SelectFile("unavailable".into()));
        assert!(app.selected_file.is_none());
    }
    #[test]
    fn leaving_restore_clears_recovery_confirmation() {
        let (mut app, _) = App::new();
        app.page = Page::Restore;
        app.recovery_dialog = true;
        app.confirmation = "RESTORE".into();
        let _ = app.update(Message::Navigate(Page::Overview));
        assert!(!app.recovery_dialog && app.confirmation.is_empty());
    }
    #[test]
    fn switching_points_keeps_last_listing_but_blocks_file_actions() {
        let (mut app, _) = App::new();
        app.status = Some(status("old"));
        app.page = Page::Restore;
        app.point = Some("old".into());
        app.displayed_folder = Some(("home".into(), "aneesh".into()));
        app.entries = vec![Entry {
            name: "report.txt".into(),
            path: "aneesh/report.txt".into(),
            is_dir: false,
            bytes: 1,
        }];
        app.selected_file = Some("aneesh/report.txt".into());
        let _ = app.update(Message::SelectPoint("new".into()));
        assert!(app.browsing && app.can_choose_backup() && !app.can_read_backups());
        assert_eq!(app.entries[0].name, "report.txt");
        assert_eq!(app.displayed_folder, Some(("home".into(), "aneesh".into())));
        assert!(app.selected_file.is_none());
        let _ = app.update(Message::SelectFile("aneesh/report.txt".into()));
        let _ = app.update(Message::RestoreFile);
        assert!(app.selected_file.is_none() && app.operation_command.is_none());
        let _ = app.update(Message::Browsed {
            generation: app.browse_generation,
            point: "new".into(),
            source: "home".into(),
            path: "".into(),
            result: Ok(Vec::new()),
        });
        assert!(!app.browsing && app.entries.is_empty());
        assert_eq!(app.displayed_folder, Some(("home".into(), "".into())));
    }
    #[test]
    fn rapid_switch_back_ignores_an_older_request_for_the_same_point() {
        let (mut app, _) = App::new();
        app.status = Some(status("first"));
        app.page = Page::Restore;
        app.point = Some("first".into());
        let _ = app.browse(String::new());
        let old_generation = app.browse_generation;
        let _ = app.update(Message::SelectPoint("second".into()));
        let _ = app.update(Message::SelectPoint("first".into()));
        assert!(app.browse_generation > old_generation);
        let _ = app.update(Message::Browsed {
            generation: old_generation,
            point: "first".into(),
            source: "home".into(),
            path: "".into(),
            result: Err("Old request failed".into()),
        });
        assert!(app.browsing && app.browser_error.is_none());
        let _ = app.update(Message::Browsed {
            generation: app.browse_generation,
            point: "first".into(),
            source: "home".into(),
            path: "".into(),
            result: Ok(Vec::new()),
        });
        assert!(!app.browsing && app.displayed_folder.is_some());
    }
    #[test]
    fn generated_folder_name_respects_unicode_byte_limits() {
        let (app, _) = App::new();
        let path = app.default_destination(&"文".repeat(200));
        assert!(
            Path::new(&path)
                .file_name()
                .unwrap()
                .as_encoded_bytes()
                .len()
                < 255
        );
    }
}
