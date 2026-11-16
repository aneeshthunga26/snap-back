use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Datelike, Timelike, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{MetadataExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

pub mod drives;
pub mod recovery;
pub mod setup;
pub const CONFIG: &str = "/etc/snap-back/config.json";
const STATE: &str = "/var/lib/snap-back";
const RUNTIME: &str = "/run/snap-back";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub name: String,
    pub mount: PathBuf,
    pub subvol: String,
    pub snapper_config: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupTarget {
    pub uuid: String,
    pub mount: PathBuf,
    pub label: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub destination: PathBuf,
    pub backup_uuid: String,
    pub source_uuid: String,
    pub sources: Vec<Source>,
    #[serde(default)]
    pub targets: Vec<BackupTarget>,
    /// None preserves the single-drive default of older configurations.
    #[serde(default)]
    pub backup_targets: Option<Vec<String>>,
}
impl Config {
    pub fn registered_targets(&self) -> Result<Vec<BackupTarget>> {
        let mut targets = self.targets.clone();
        if !targets.iter().any(|target| target.uuid == self.backup_uuid) {
            targets.insert(
                0,
                BackupTarget {
                    uuid: self.backup_uuid.clone(),
                    mount: self.destination.clone(),
                    label: "Snap Back".into(),
                },
            );
        }
        let mut seen = BTreeSet::new();
        let mut mounts = BTreeSet::new();
        for target in &targets {
            valid_id(&target.uuid)?;
            ensure!(
                target.uuid != self.source_uuid,
                "System filesystem cannot be a backup drive"
            );
            ensure!(
                target.mount.is_absolute() && target.mount != Path::new("/"),
                "Invalid backup mount"
            );
            ensure!(
                seen.insert(target.uuid.clone()),
                "Duplicate saved backup drive"
            );
            ensure!(
                mounts.insert(target.mount.clone()),
                "Saved backup drives cannot share a mount directory"
            );
        }
        Ok(targets)
    }

    pub fn selected_targets(&self, requested: Option<&[String]>) -> Result<Vec<BackupTarget>> {
        let legacy = vec![self.backup_uuid.clone()];
        let selected =
            requested.unwrap_or_else(|| self.backup_targets.as_deref().unwrap_or(&legacy));
        ensure!(!selected.is_empty(), "Choose at least one backup drive");
        let saved = self.registered_targets()?;
        let mut seen = BTreeSet::new();
        let mut targets = Vec::new();
        for uuid in selected {
            valid_id(uuid)?;
            let target = saved
                .iter()
                .find(|target| &target.uuid == uuid)
                .with_context(|| format!("Backup drive {uuid} is not a saved drive"))?;
            if seen.insert(uuid.clone()) {
                targets.push(target.clone());
            }
        }
        Ok(targets)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Point {
    pub id: String,
    pub created_at: String,
    pub label: String,
    pub kind: String,
    pub bytes: u64,
    pub subvolumes: Vec<String>,
    pub snapshots: BTreeMap<String, String>,
    pub boot: bool,
}
#[derive(Debug, Serialize)]
pub struct Drive {
    pub label: String,
    pub mount: PathBuf,
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub connected: bool,
}
#[derive(Debug, Serialize)]
pub struct BackupDriveStatus {
    pub uuid: String,
    pub label: String,
    pub mount: PathBuf,
    pub connected: bool,
    pub mounted: bool,
}
#[derive(Debug, Serialize)]
pub struct SnapperConfig {
    pub name: String,
    pub subvolume: String,
    pub timeline_create: Option<bool>,
    pub timeline_cleanup: Option<bool>,
}
#[derive(Debug, Serialize)]
pub struct Status {
    pub configured: bool,
    pub drive: Drive,
    pub points: Vec<Point>,
    pub busy: bool,
    pub progress: Option<String>,
    pub last_error: Option<String>,
    pub schedule_enabled: bool,
    pub configs: Vec<SnapperConfig>,
    pub recovery_pending: Option<String>,
    pub backup_targets: Vec<String>,
    pub backup_drives: Vec<BackupDriveStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub bytes: u64,
}

pub fn command(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .with_context(|| format!("Starting {program}"))?;
    ensure!(
        output.status.success(),
        "{program}: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
fn ps(p: &Path) -> Result<&str> {
    p.to_str().context("Path is not UTF-8")
}
pub fn root() -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "Administrator authorization is required"
    );
    Ok(())
}
pub fn valid_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        "Invalid identifier"
    );
    Ok(())
}
pub fn load_config() -> Result<Config> {
    load_config_at(Path::new(CONFIG))?
        .context("Choose a backup drive in Snap Back to set up backups")
}
fn load_config_at(path: &Path) -> Result<Option<Config>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Could not read Snap Back configuration at {}",
                    path.display()
                )
            });
        }
    };
    let c: Config = serde_json::from_slice(&bytes)
        .with_context(|| format!("Invalid Snap Back configuration at {}", path.display()))?;
    ensure!(
        c.destination.is_absolute() && c.destination != Path::new("/"),
        "Invalid backup mount"
    );
    ensure!(!c.sources.is_empty(), "No source subvolumes configured");
    for s in &c.sources {
        valid_id(&s.name)?;
        ensure!(s.mount.is_absolute(), "Invalid source mount");
    }
    Ok(Some(c))
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let temp = path.with_extension("tmp");
    let mut file = File::create(&temp)?;
    use std::io::Write;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    fs::rename(&temp, path)?;
    File::open(path.parent().context("No parent")?)?.sync_all()?;
    Ok(())
}
fn write_progress(message: &str) {
    let _ = fs::create_dir_all(RUNTIME);
    let _ = fs::write(format!("{RUNTIME}/progress"), message);
}
pub struct Operation {
    _lock: File,
}
impl Operation {
    pub fn start() -> Result<Self> {
        root()?;
        fs::create_dir_all(RUNTIME)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(format!("{RUNTIME}/operation.lock"))?;
        lock.try_lock_exclusive()
            .context("Another backup or restore operation is already running")?;
        Ok(Self { _lock: lock })
    }
}
impl Drop for Operation {
    fn drop(&mut self) {
        let _ = fs::remove_file(format!("{RUNTIME}/progress"));
    }
}
fn busy() -> bool {
    let Ok(f) = OpenOptions::new()
        .read(true)
        .open(format!("{RUNTIME}/operation.lock"))
    else {
        return false;
    };
    f.try_lock_exclusive().is_err()
}
fn mounted_uuid(mount: &Path) -> Result<String> {
    command("findmnt", &["-n", "-M", ps(mount)?, "-o", "UUID"])
}
fn validate_drive(c: &Config) -> Result<()> {
    ensure!(
        mounted_uuid(&c.destination)
            .context("Backup disk is disconnected; mount the configured backup drive first")?
            == c.backup_uuid,
        "Backup disk is disconnected or a different disk is mounted at {}",
        c.destination.display()
    );
    ensure!(
        command(
            "findmnt",
            &["-n", "-M", ps(&c.destination)?, "-o", "FSTYPE"]
        )? == "btrfs",
        "Backup drive must use Btrfs"
    );
    ensure!(
        c.backup_uuid != c.source_uuid,
        "Backup must be on a different filesystem"
    );
    Ok(())
}
pub fn points(c: &Config) -> Result<Vec<Point>> {
    let mut points = Vec::new();
    if !c.destination.join("points").exists() {
        return Ok(points);
    }
    for e in fs::read_dir(c.destination.join("points"))? {
        let e = e?;
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let p = e.path().join("manifest.json");
        if p.exists() {
            let point: Point = serde_json::from_slice(&fs::read(&p)?)
                .with_context(|| format!("Invalid backup manifest {}", p.display()))?;
            valid_id(&point.id)?;
            points.push(point);
        }
    }
    points.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(points)
}
pub fn status() -> Result<Status> {
    status_at(Path::new(CONFIG))
}
fn status_at(config_path: &Path) -> Result<Status> {
    let Some(c) = load_config_at(config_path)? else {
        return Ok(Status {
            configured: false,
            drive: Drive {
                label: String::new(),
                mount: PathBuf::new(),
                total_bytes: 0,
                used_bytes: 0,
                available_bytes: 0,
                connected: false,
            },
            points: vec![],
            busy: busy(),
            progress: fs::read_to_string(format!("{RUNTIME}/progress")).ok(),
            last_error: None,
            schedule_enabled: false,
            configs: vec![],
            recovery_pending: None,
            backup_targets: vec![],
            backup_drives: vec![],
            message: None,
        });
    };
    let backup_targets = c
        .selected_targets(None)?
        .into_iter()
        .map(|target| target.uuid)
        .collect();
    let backup_drives = c
        .registered_targets()?
        .into_iter()
        .map(|target| {
            let mut config = c.clone();
            config.destination = target.mount.clone();
            config.backup_uuid = target.uuid.clone();
            let mounted = validate_drive(&config).is_ok();
            BackupDriveStatus {
                connected: mounted || Path::new("/dev/disk/by-uuid").join(&target.uuid).exists(),
                mounted,
                uuid: target.uuid,
                label: target.label,
                mount: target.mount,
            }
        })
        .collect();
    let connected = validate_drive(&c).is_ok();
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let mut total = 0;
    let mut available = 0;
    let mut used = 0;
    if connected {
        let path = std::ffi::CString::new(ps(&c.destination)?)?;
        if unsafe { libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) } == 0 {
            let stat = unsafe { stat.assume_init() };
            total = stat.f_blocks * stat.f_frsize;
            available = stat.f_bavail * stat.f_frsize;
            used = total - stat.f_bfree * stat.f_frsize;
        }
    }
    let configs = c
        .sources
        .iter()
        .filter_map(|s| s.snapper_config.as_ref().map(|n| (s, n)))
        .map(|(s, n)| {
            let text = fs::read_to_string(format!("/etc/snapper/configs/{n}")).ok();
            SnapperConfig {
                name: n.clone(),
                subvolume: s.mount.display().to_string(),
                timeline_create: text.as_ref().map(|s| s.contains("TIMELINE_CREATE=\"yes\"")),
                timeline_cleanup: text
                    .as_ref()
                    .map(|s| s.contains("TIMELINE_CLEANUP=\"yes\"")),
            }
        })
        .collect();
    Ok(Status {
        configured: true,
        drive: Drive {
            label: c
                .targets
                .iter()
                .find(|t| t.uuid == c.backup_uuid)
                .map(|t| t.label.clone())
                .unwrap_or_else(|| "Snap Back".into()),
            mount: c.destination.clone(),
            total_bytes: total,
            used_bytes: used,
            available_bytes: available,
            connected,
        },
        points: if connected { points(&c)? } else { vec![] },
        busy: busy(),
        progress: fs::read_to_string(format!("{RUNTIME}/progress")).ok(),
        last_error: fs::read_to_string(format!("{STATE}/last-error")).ok(),
        schedule_enabled: command("systemctl", &["is-enabled", "snap-back.timer"])
            .map(|s| s == "enabled")
            .unwrap_or(false),
        configs,
        recovery_pending: recovery::pending()
            .or_else(|| fs::read_to_string(format!("{STATE}/recovery-point")).ok()),
        backup_targets,
        backup_drives,
        message: None,
    })
}
fn send_receive(source: &Path, parent: Option<&Path>, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    let mut send = Command::new("btrfs");
    send.arg("send").arg("--proto").arg("2");
    if let Some(parent) = parent {
        send.arg("-p").arg(parent);
    }
    let mut child = send
        .arg(source)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let receive = Command::new("btrfs")
        .arg("receive")
        .arg(destination)
        .stdin(child.stdout.take().context("No send stream")?)
        .output()?;
    let sent = child.wait_with_output()?;
    ensure!(
        sent.status.success() && receive.status.success(),
        "Btrfs transfer failed: {} {}",
        String::from_utf8_lossy(&sent.stderr),
        String::from_utf8_lossy(&receive.stderr)
    );
    let received = destination.join(source.file_name().context("No snapshot name")?);
    ensure!(
        command("btrfs", &["property", "get", "-ts", ps(&received)?, "ro"])? == "ro=true",
        "Received snapshot is incomplete"
    );
    let source_uuid = command("btrfs", &["subvolume", "show", ps(source)?])?;
    let target_uuid = command("btrfs", &["subvolume", "show", ps(&received)?])?;
    let uuid = source_uuid
        .lines()
        .find_map(|l| l.trim().strip_prefix("UUID:").map(str::trim))
        .context("Missing source UUID")?;
    let received_uuid = target_uuid
        .lines()
        .find_map(|l| l.trim().strip_prefix("Received UUID:").map(str::trim))
        .context("Missing received UUID")?;
    ensure!(
        uuid == received_uuid,
        "Received snapshot UUID does not match source"
    );
    Ok(())
}
fn delete_subvol(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    ensure!(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "Refusing snapshot symlink {}",
        path.display()
    );
    if is_subvolume(path) {
        command("btrfs", &["subvolume", "delete", ps(path)?])?;
    } else {
        // A received parent contains empty directory placeholders for its nested
        // subvolumes. Recovery can leave one where an anchor used to be.
        fs::remove_dir(path).with_context(|| {
            format!(
                "{} is neither a Btrfs subvolume nor an empty placeholder",
                path.display()
            )
        })?;
    }
    Ok(())
}
fn is_subvolume(path: &Path) -> bool {
    path.is_dir() && command("btrfs", &["subvolume", "show", &path.to_string_lossy()]).is_ok()
}

/// A failed transfer must not keep filling either filesystem with hidden snapshots.
struct BackupWork {
    staging: PathBuf,
    incoming: PathBuf,
    sources: Vec<String>,
    keep_sources: bool,
}
fn cleanup_work_path(path: &Path, sources: &[String]) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    for source in sources {
        delete_subvol(&path.join(source).join("snapshot"))?;
    }
    fs::remove_dir_all(path)?;
    Ok(())
}
impl Drop for BackupWork {
    fn drop(&mut self) {
        if !self.keep_sources {
            let _ = cleanup_work_path(&self.staging, &self.sources);
        }
        let _ = cleanup_work_path(&self.incoming, &self.sources);
    }
}
fn legacy_anchor_generation() -> Option<String> {
    let id = fs::read_to_string(PathBuf::from(STATE).join("anchors/latest")).ok()?;
    valid_id(&id).ok()?;
    Some(id)
}
fn anchor_directory(c: &Config) -> PathBuf {
    PathBuf::from(STATE)
        .join("anchors/by-drive")
        .join(&c.backup_uuid)
}
fn anchor_generation(c: &Config) -> Option<String> {
    if let Ok(id) = fs::read_to_string(anchor_directory(c).join("latest")) {
        valid_id(&id).ok()?;
        Some(id)
    } else {
        legacy_anchor_generation()
    }
}
fn anchor_snapshot(c: &Config, id: &str, source: &str) -> PathBuf {
    let directory = anchor_directory(c).join(id);
    if directory.exists() {
        directory.join(source).join("snapshot")
    } else {
        // Read the old immutable generation in place during migration. Moving
        // a read-only Btrfs subvolume can fail on supported kernels.
        PathBuf::from(STATE)
            .join("anchors")
            .join(id)
            .join(source)
            .join("snapshot")
    }
}
fn cleanup_anchor_generations(c: &Config, keep: Option<&str>) -> Result<()> {
    let anchors = anchor_directory(c);
    if !anchors.exists() {
        return Ok(());
    }
    let sources = c.sources.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    for entry in fs::read_dir(&anchors)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !entry.file_type()?.is_dir() || valid_id(&name).is_err() || keep == Some(name.as_ref()) {
            continue;
        }
        // The pre-generation prototype used anchors/<source>/snapshot. Its old
        // anchors have no point ID and are also safe to discard after migration.
        if c.sources.iter().any(|s| s.name.as_str() == name.as_ref()) {
            delete_subvol(&entry.path().join("snapshot"))?;
            fs::remove_dir_all(entry.path())?;
        } else {
            cleanup_work_path(&entry.path(), &sources)?;
        }
    }
    Ok(())
}
fn cleanup_interrupted(c: &Config) -> Result<()> {
    let sources = c.sources.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    for (dir, prefix) in [
        (PathBuf::from(STATE).join("staging"), ""),
        (c.destination.join("points"), ".incomplete-"),
    ] {
        if !dir.exists() {
            continue;
        }
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Some(id) = name.strip_prefix(prefix) else {
                continue;
            };
            if valid_id(id).is_ok() {
                cleanup_work_path(&entry.path(), &sources)
                    .context("Unable to clean an interrupted backup; retained for inspection")?;
            }
        }
    }
    cleanup_anchor_generations(c, anchor_generation(c).as_deref())?;
    Ok(())
}
fn commit_anchor_generation(c: &Config, id: &str) -> Result<()> {
    let anchors = anchor_directory(c);
    let mut file = File::create(anchors.join(".latest.tmp"))?;
    use std::io::Write;
    file.write_all(id.as_bytes())?;
    file.sync_all()?;
    fs::rename(anchors.join(".latest.tmp"), anchors.join("latest"))?;
    File::open(anchors)?.sync_all()?;
    Ok(())
}
fn cleanup_legacy_anchors(c: &Config, previous: Option<&Point>) -> Result<()> {
    let Some(legacy) = legacy_anchor_generation() else {
        return Ok(());
    };
    if previous.is_none_or(|point| point.id != legacy) {
        return Ok(());
    }
    let directory = PathBuf::from(STATE).join("anchors");
    let sources = c
        .sources
        .iter()
        .map(|source| source.name.clone())
        .collect::<Vec<_>>();
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "by-drive" || !entry.file_type()?.is_dir() || valid_id(&name).is_err() {
            continue;
        }
        if c.sources.iter().any(|source| source.name == name) {
            delete_subvol(&entry.path().join("snapshot"))?;
            fs::remove_dir_all(entry.path())?;
        } else {
            cleanup_work_path(&entry.path(), &sources)?;
        }
    }
    fs::remove_file(directory.join("latest"))?;
    Ok(())
}

fn validate_sources(c: &Config) -> Result<()> {
    // A Btrfs snapshot does not recurse into child subvolumes. Refuse a backup
    // with silently omitted data if the machine layout has changed since setup.
    let output = command("btrfs", &["subvolume", "list", "-a", "/"])?;
    let configured: BTreeSet<_> = c
        .sources
        .iter()
        .map(|s| s.subvol.trim_start_matches('/'))
        .collect();
    let mounts: serde_json::Value = serde_json::from_str(&command(
        "findmnt",
        &["--json", "-t", "btrfs", "-o", "TARGET,FSROOT,UUID"],
    )?)?;
    fn check_mounts(
        entries: &serde_json::Value,
        c: &Config,
        configured: &BTreeSet<&str>,
    ) -> Result<()> {
        if let Some(entries) = entries.as_array() {
            for entry in entries {
                if entry.get("uuid").and_then(|v| v.as_str()) == Some(c.source_uuid.as_str()) {
                    let target = entry.get("target").and_then(|v| v.as_str()).unwrap_or("");
                    let fsroot = entry
                        .get("fsroot")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim_start_matches('/');
                    ensure!(
                        target.starts_with("/run/snap-back") || configured.contains(fsroot),
                        "Mounted Btrfs subvolume {target} ({fsroot}) is missing from the backup configuration"
                    );
                }
                check_mounts(&entry["children"], c, configured)?;
            }
        }
        Ok(())
    }
    check_mounts(&mounts["filesystems"], c, &configured)?;
    for line in output.lines() {
        let Some((_, path)) = line.split_once(" path ") else {
            continue;
        };
        let path = path.strip_prefix("<FS_TREE>/").unwrap_or(path);
        if path.split('/').any(|p| p == ".snapshots")
            || path.contains("/var/lib/snap-back/")
            || path.starts_with(".snap-back-")
        {
            continue;
        }
        let under_source = configured
            .iter()
            .any(|source| path == *source || path.starts_with(&format!("{source}/")));
        ensure!(
            !under_source || configured.contains(path),
            "Unconfigured nested Btrfs subvolume {path}; add it to /etc/snap-back/config.json before backing up"
        );
    }
    Ok(())
}

pub fn backup(
    label: Option<String>,
    scheduled: bool,
    requested: &[String],
    save_targets: bool,
) -> Result<Status> {
    let _op = Operation::start()?;
    let mut c = load_config()?;
    ensure!(
        !save_targets || !requested.is_empty(),
        "Choose backup drives before saving the selection"
    );
    let targets = c.selected_targets((!requested.is_empty()).then_some(requested))?;
    validate_sources(&c)?;
    ensure!(
        recovery::pending().is_none(),
        "Cancel or complete the pending recovery before backing up"
    );
    fs::create_dir_all(STATE)?;
    if save_targets {
        c.backup_targets = Some(targets.iter().map(|target| target.uuid.clone()).collect());
        atomic_json(Path::new(CONFIG), &c)?;
    }
    let mut completed = Vec::new();
    let mut failed = Vec::new();
    for target in &targets {
        write_progress(&format!("Preparing {}", target.label));
        let mut destination = c.clone();
        destination.destination = target.mount.clone();
        destination.backup_uuid = target.uuid.clone();
        let result = (|| -> Result<()> {
            drives::ensure_target_mounted(&destination, target)?;
            validate_drive(&destination)?;
            backup_inner(&destination, label.clone(), scheduled)
        })();
        match result {
            Ok(()) => completed.push(target.label.clone()),
            Err(error) => failed.push(format!("{}: {error:#}", target.label)),
        }
    }
    if !failed.is_empty() {
        let summary = if completed.is_empty() {
            format!("No drives were backed up. {}", failed.join("; "))
        } else {
            format!(
                "Backed up to {}. Could not back up to {}",
                completed.join(", "),
                failed.join("; ")
            )
        };
        fs::write(format!("{STATE}/last-error"), &summary)?;
        anyhow::bail!(summary);
    }
    let _ = fs::remove_file(format!("{STATE}/last-error"));
    drop(_op);
    let mut status = status()?;
    status.message = Some(if completed.len() == 1 {
        format!("Backed up to {}.", completed[0])
    } else {
        format!(
            "Backed up to {} drives: {}.",
            completed.len(),
            completed.join(", ")
        )
    });
    Ok(status)
}
fn backup_inner(c: &Config, label: Option<String>, scheduled: bool) -> Result<()> {
    cleanup_interrupted(c)?;
    let now = Utc::now();
    let id = now.format("%Y%m%dT%H%M%S%fZ").to_string();
    // Read-only subvolumes stay at the exact location where they were created.
    // Some Btrfs kernels reject renaming them, even within one filesystem.
    let staging = anchor_directory(c).join(&id);
    let incoming = c
        .destination
        .join("points")
        .join(format!(".incomplete-{id}"));
    fs::create_dir_all(&staging)?;
    fs::create_dir_all(&incoming)?;
    let mut cleanup = BackupWork {
        staging: staging.clone(),
        incoming: incoming.clone(),
        sources: c.sources.iter().map(|s| s.name.clone()).collect(),
        keep_sources: false,
    };
    let previous = points(c)?.into_iter().next();
    let mut snapshots = BTreeMap::new();
    let mut local_snapshots = Vec::new();
    for source in &c.sources {
        ensure!(
            command("findmnt", &["-n", "-T", ps(&source.mount)?, "-o", "UUID"])? == c.source_uuid,
            "Source {} is not on the configured filesystem",
            source.mount.display()
        );
        write_progress(&format!("Snapshotting {}", source.name));
        let local = staging.join(&source.name);
        fs::create_dir_all(&local)?;
        let snapshot = local.join("snapshot");
        if let Some(config) = &source.snapper_config {
            let number = command(
                "snapper",
                &[
                    "-c",
                    config,
                    "create",
                    "--type",
                    "single",
                    "--print-number",
                    "--description",
                    &format!("Snap Back {id}"),
                    "--userdata",
                    "snap-back=yes",
                ],
            )?;
            ensure!(
                number.chars().all(|c| c.is_ascii_digit()) && !number.is_empty(),
                "Invalid Snapper snapshot number"
            );
            let snapper_path = source
                .mount
                .join(".snapshots")
                .join(&number)
                .join("snapshot");
            command(
                "btrfs",
                &[
                    "subvolume",
                    "snapshot",
                    "-r",
                    ps(&snapper_path)?,
                    ps(&snapshot)?,
                ],
            )?;
            // Our retained immutable anchor carries incremental history; Snapper may clean its own snapshot.
            command(
                "snapper",
                &[
                    "-c",
                    config,
                    "modify",
                    "--cleanup-algorithm",
                    "number",
                    &number,
                ],
            )?;
            snapshots.insert(source.name.clone(), format!("{config}:{number}"));
        } else {
            command(
                "btrfs",
                &[
                    "subvolume",
                    "snapshot",
                    "-r",
                    ps(&source.mount)?,
                    ps(&snapshot)?,
                ],
            )?;
        }
        local_snapshots.push((source.name.clone(), snapshot));
    }
    // Every subvolume is now frozen before transfer; applications should quiesce databases for application consistency.
    let mut all_incremental = previous.is_some();
    for (name, snapshot) in &local_snapshots {
        let label = c
            .registered_targets()?
            .into_iter()
            .find(|target| target.uuid == c.backup_uuid)
            .map(|target| target.label)
            .unwrap_or_else(|| "backup drive".into());
        write_progress(&format!("Backing up {name} to {label}"));
        let anchor_id = anchor_generation(c);
        let anchor = anchor_snapshot(c, anchor_id.as_deref().unwrap_or("missing"), name);
        let parent = previous
            .as_ref()
            .filter(|p| {
                anchor_id.as_deref() == Some(p.id.as_str())
                    && is_subvolume(&anchor)
                    && is_subvolume(
                        &c.destination
                            .join("points")
                            .join(&p.id)
                            .join(name)
                            .join("snapshot"),
                    )
            })
            .map(|_| anchor.as_path());
        if parent.is_none() {
            all_incremental = false;
        }
        send_receive(snapshot, parent, &incoming.join(name))?;
    }
    write_progress("Saving boot files and completing the restore point");
    fs::create_dir_all(incoming.join("boot"))?;
    command(
        "rsync",
        &[
            "-a",
            "--exclude=/snap-back/",
            "/boot/",
            ps(&incoming.join("boot"))?,
        ],
    )?;
    let bytes = command("du", &["-sb", ps(&incoming)?])?
        .split_whitespace()
        .next()
        .unwrap_or("0")
        .parse()
        .unwrap_or(0);
    let point = Point {
        id: id.clone(),
        created_at: now.to_rfc3339(),
        label: label.unwrap_or_else(|| {
            if scheduled {
                "Hourly backup".into()
            } else {
                "Manual backup".into()
            }
        }),
        kind: if all_incremental {
            "incremental".into()
        } else {
            "full".into()
        },
        bytes,
        subvolumes: c.sources.iter().map(|s| s.name.clone()).collect(),
        snapshots,
        boot: true,
    };
    atomic_json(&incoming.join("manifest.json"), &point)?;
    command("sync", &["-f", ps(&incoming)?])?;
    fs::rename(&incoming, c.destination.join("points").join(&id))?;
    File::open(c.destination.join("points"))?.sync_all()?;
    // The published restore point now owns this immutable source generation.
    // Disarm cleanup before committing the pointer: an interrupted pointer write
    // may force the next backup to be full, but cannot delete a published backup.
    cleanup.keep_sources = true;
    commit_anchor_generation(c, &id)?;
    cleanup_anchor_generations(c, Some(&id))?;
    cleanup_legacy_anchors(c, previous.as_ref())?;
    write_progress("Applying 24 hourly, 7 daily, 4 weekly retention");
    prune(c)?;
    Ok(())
}

/// Keep the newest point in each occupied hourly/day/week bucket, with independent tier limits.
pub fn retention_keep(points: &[Point]) -> BTreeSet<String> {
    let mut sorted = points.to_vec();
    sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let mut keep = BTreeSet::new();
    let mut hours = BTreeSet::new();
    let mut days = BTreeSet::new();
    let mut weeks = BTreeSet::new();
    for p in sorted {
        let Ok(dt) = DateTime::parse_from_rfc3339(&p.created_at) else {
            keep.insert(p.id);
            continue;
        };
        let hour = (dt.year(), dt.ordinal(), dt.hour());
        let day = (dt.year(), dt.ordinal());
        let week = (dt.iso_week().year(), dt.iso_week().week());
        if hours.len() < 24 && hours.insert(hour) {
            keep.insert(p.id.clone());
        }
        if days.len() < 7 && days.insert(day) {
            keep.insert(p.id.clone());
        }
        if weeks.len() < 4 && weeks.insert(week) {
            keep.insert(p.id);
        }
    }
    keep
}
fn prune(c: &Config) -> Result<()> {
    let all = points(c)?;
    let keep = retention_keep(&all);
    for point in all {
        if !keep.contains(&point.id) {
            let dir = c.destination.join("points").join(&point.id);
            for name in &point.subvolumes {
                valid_id(name)?;
                delete_subvol(&dir.join(name).join("snapshot"))?;
            }
            fs::remove_dir_all(dir)?;
        }
    }
    Ok(())
}
fn point_path(c: &Config, id: &str, source: &str) -> Result<PathBuf> {
    validate_drive(c)?;
    valid_id(id)?;
    valid_id(source)?;
    ensure!(
        c.sources.iter().any(|s| s.name == source),
        "Unknown subvolume"
    );
    let point = points(c)?
        .into_iter()
        .find(|p| p.id == id)
        .context("Restore point not found")?;
    ensure!(
        point.subvolumes.iter().any(|s| s == source),
        "This restore point does not contain that subvolume"
    );
    Ok(c.destination
        .join("points")
        .join(id)
        .join(source)
        .join("snapshot"))
}
fn safe_relative(base: &Path, relative: &str) -> Result<PathBuf> {
    ensure!(
        !Path::new(relative).is_absolute(),
        "Expected a relative path inside the restore point"
    );
    ensure!(
        Path::new(relative)
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "Parent paths are not allowed"
    );
    let candidate = base.join(relative);
    // Never follow snapshot symlinks into the current live system, even when listing.
    let mut walk = base.to_owned();
    for c in Path::new(relative).components() {
        if let Component::Normal(c) = c {
            walk.push(c);
            ensure!(
                !fs::symlink_metadata(&walk)?.file_type().is_symlink(),
                "Symlinks cannot be browsed or restored directly"
            );
        }
    }
    ensure!(
        candidate.canonicalize()?.starts_with(base.canonicalize()?),
        "Path escapes the restore point"
    );
    Ok(candidate)
}
pub fn browse(id: &str, source: &str, relative: &str) -> Result<Vec<Entry>> {
    root()?;
    let c = load_config()?;
    let base = point_path(&c, id, source)?;
    let path = safe_relative(&base, relative)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        let path = entry.path();
        entries.push(Entry {
            name: entry.file_name().to_string_lossy().into_owned(),
            path: path.strip_prefix(&base)?.to_string_lossy().into_owned(),
            is_dir: meta.is_dir(),
            bytes: meta.len(),
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}
pub fn restore_file(id: &str, source: &str, relative: &str, to: &Path) -> Result<String> {
    let _op = Operation::start()?;
    let c = load_config()?;
    let base = point_path(&c, id, source)?;
    let from = safe_relative(&base, relative)?;
    let metadata = fs::symlink_metadata(&from)?;
    ensure!(
        metadata.is_file() || metadata.is_dir(),
        "Select a regular file or directory to recover"
    );
    ensure!(
        !relative.is_empty() && relative != ".",
        "Select a file or folder to recover"
    );
    ensure!(
        to.is_absolute(),
        "Recovery destination must be an absolute path"
    );
    ensure!(
        !to.exists(),
        "Destination already exists; choose a new name to avoid overwriting files"
    );
    let parent = to.parent().context("Invalid destination")?.canonicalize()?;
    ensure!(
        !parent.starts_with(&c.destination)
            && !parent.starts_with("/proc")
            && !parent.starts_with("/sys")
            && !parent.starts_with("/dev"),
        "Invalid recovery destination"
    );
    let uid = std::env::var("PKEXEC_UID")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    if uid != 0 {
        let user_home = command("getent", &["passwd", &uid.to_string()])?
            .split(':')
            .nth(5)
            .context("No home for caller")?
            .to_owned();
        ensure!(
            parent.starts_with(Path::new(&user_home).canonicalize()?),
            "Choose a destination inside your home folder"
        );
    }
    let filename = to.file_name().context("Invalid destination")?;
    let final_path = parent.join(filename);
    // Pin both directory descriptors. A user may rename entries in their home
    // while the privileged helper runs; child tools must never resolve a mutable
    // target pathname again after it has been checked.
    let parent_fd = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(&parent)?;
    let name = std::ffi::CString::new(filename.as_encoded_bytes())?;
    ensure!(
        unsafe { libc::mkdirat(parent_fd.as_raw_fd(), name.as_ptr(), 0o700) } == 0,
        "Destination already exists or is not writable: {}",
        std::io::Error::last_os_error()
    );
    let fd = unsafe {
        libc::openat(
            parent_fd.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    ensure!(fd >= 0, "Unable to open recovery destination");
    let target = unsafe { File::from_raw_fd(fd) };
    let target_meta = target.metadata()?;
    ensure!(
        target_meta.uid() == 0 && target_meta.permissions().mode() & 0o077 == 0,
        "Recovery destination changed before it could be secured"
    );
    let pinned = PathBuf::from(format!(
        "/proc/{}/fd/{}",
        std::process::id(),
        target.as_raw_fd()
    ));
    write_progress("Recovering selected files");
    // rsync preserves links and normal metadata but never recreates device nodes
    // or sockets from a system snapshot in a user's home directory.
    command(
        "rsync",
        &[
            "-a",
            "--no-devices",
            "--no-specials",
            "--",
            ps(&from)?,
            ps(&pinned)?,
        ],
    )?;
    sanitize_recovered(&pinned, uid)?;
    ensure!(
        unsafe { libc::fchown(target.as_raw_fd(), uid, !0) } == 0,
        "Unable to set recovered folder owner"
    );
    Ok(format!("Recovered to {}", final_path.display()))
}

fn sanitize_recovered(root: &Path, uid: u32) -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let meta = fs::symlink_metadata(&path)?;
        if meta.is_dir() {
            sanitize_recovered(&path, uid)?;
        }
        let raw = std::ffi::CString::new(path.as_os_str().as_bytes())?;
        ensure!(
            unsafe { libc::lchown(raw.as_ptr(), uid, !0) } == 0,
            "Unable to set recovered file owner"
        );
        if !meta.file_type().is_symlink() {
            fs::set_permissions(
                &path,
                fs::Permissions::from_mode(meta.permissions().mode() & 0o1777),
            )?;
        }
    }
    Ok(())
}
use std::os::unix::fs::OpenOptionsExt;

pub fn schedule(on: bool) -> Result<String> {
    let _op = Operation::start()?;
    command(
        "systemctl",
        &[
            if on { "enable" } else { "disable" },
            "--now",
            "snap-back.timer",
        ],
    )?;
    Ok(if on {
        "Hourly backups enabled"
    } else {
        "Automatic backups paused"
    }
    .into())
}
pub fn prepare_recovery(id: &str) -> Result<String> {
    let _op = Operation::start()?;
    let c = load_config()?;
    validate_drive(&c)?;
    valid_id(id)?;
    let point = points(&c)?
        .into_iter()
        .find(|p| p.id == id)
        .context("Restore point not found")?;
    ensure!(
        c.sources.iter().all(|s| point.subvolumes.contains(&s.name)),
        "Restore point is incomplete"
    );
    write_progress("Preparing offline system recovery; this can take several minutes");
    let sources = c
        .sources
        .iter()
        .map(|s| recovery::RecoverySource {
            name: s.name.clone(),
            subvol: s.subvol.clone(),
        })
        .collect::<Vec<_>>();
    let message = recovery::prepare(&c.destination, id, &c.source_uuid, &sources)?;
    fs::create_dir_all(STATE)?;
    fs::write(format!("{STATE}/recovery-point"), id)?;
    Ok(message)
}

/// Clear the public UI marker after a successfully cancelled recovery.
pub fn cancel_recovery() -> Result<String> {
    let _lock = Operation::start()?;
    let message = recovery::cancel()?;
    let _ = fs::remove_file(format!("{STATE}/recovery-point"));
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ConfigFixture(PathBuf);

    impl ConfigFixture {
        fn new() -> Self {
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "snap-back-config-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn config_path(&self) -> PathBuf {
            self.0.join("config.json")
        }
    }

    impl Drop for ConfigFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn missing_config_is_a_successful_first_launch_status() {
        let fixture = ConfigFixture::new();
        let status = status_at(&fixture.config_path()).unwrap();
        assert!(!status.configured);
        assert!(!status.drive.connected);
        assert!(status.drive.mount.as_os_str().is_empty());
        assert!(status.points.is_empty());
        assert!(status.backup_drives.is_empty());
        assert!(status.backup_targets.is_empty());
        assert!(status.configs.is_empty());
        assert!(status.last_error.is_none());
        assert!(!status.schedule_enabled);
        assert!(status.recovery_pending.is_none());
        let json = serde_json::to_value(&status).unwrap();
        assert_eq!(json["configured"], false);
        assert!(json["last_error"].is_null());
    }

    #[test]
    fn malformed_config_is_not_treated_as_first_launch() {
        let fixture = ConfigFixture::new();
        fs::write(fixture.config_path(), b"{broken configuration").unwrap();
        let error = status_at(&fixture.config_path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Invalid Snap Back configuration")
        );
        assert!(error.downcast_ref::<serde_json::Error>().is_some());
    }

    #[test]
    fn config_read_errors_are_not_treated_as_first_launch() {
        let fixture = ConfigFixture::new();
        fs::create_dir(fixture.config_path()).unwrap();
        let error = status_at(&fixture.config_path()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Could not read Snap Back configuration")
        );
        assert_ne!(
            error.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::NotFound
        );
    }

    fn point(id: usize, date: String) -> Point {
        Point {
            id: id.to_string(),
            created_at: date,
            label: String::new(),
            kind: "full".into(),
            bytes: 0,
            subvolumes: vec![],
            snapshots: BTreeMap::new(),
            boot: false,
        }
    }
    #[test]
    fn retention_preserves_tiers_and_latest() {
        let now = Utc::now();
        let ps = (0..24 * 50)
            .map(|i| point(i, (now - chrono::Duration::hours(i as i64)).to_rfc3339()))
            .collect::<Vec<_>>();
        let keep = retention_keep(&ps);
        assert!(keep.contains("0"));
        assert!(keep.len() >= 24);
        assert!(keep.len() <= 35);
        assert!(keep.iter().any(|id| id.parse::<usize>().unwrap() > 24 * 7));
    }
    #[test]
    fn retention_coalesces_points_in_the_same_hour() {
        let points = (0..60)
            .map(|i| point(i, format!("2026-11-14T12:00:{i:02}Z")))
            .collect::<Vec<_>>();
        let keep = retention_keep(&points);
        assert_eq!(keep.len(), 1);
        let newest = points
            .iter()
            .max_by(|a, b| a.created_at.cmp(&b.created_at))
            .unwrap();
        assert!(keep.contains(&newest.id));
    }
    #[test]
    fn traversal_is_rejected() {
        assert!(safe_relative(Path::new("/"), "../etc").is_err());
        assert!(safe_relative(Path::new("/"), "/etc").is_err());
        assert!(valid_id("../../x").is_err());
        assert!(valid_id("20260101T010000Z").is_ok());
    }
}
