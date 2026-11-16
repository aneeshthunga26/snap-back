//! Explicit whole-disk setup. Every destructive request is tied to a short-lived
//! device identity plan and revalidated under the same lock as backups/restores.
use crate::{BackupTarget, CONFIG, Config, Operation, command, load_config, recovery, root, setup};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Duration, Utc};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    os::{
        fd::AsRawFd,
        unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

const PLANS: &str = "/run/snap-back/drive-plans";
const MOUNTS: &str = "/backup/snap-back-drives";

#[derive(Clone, Debug, Deserialize)]
struct Block {
    name: String,
    path: String,
    #[serde(rename = "type")]
    kind: String,
    size: u64,
    #[serde(default)]
    fstype: Option<String>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    uuid: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    serial: Option<String>,
    #[serde(default)]
    ro: bool,
    #[serde(rename = "maj:min")]
    number: String,
    #[serde(default)]
    mountpoints: Vec<Option<String>>,
    #[serde(default)]
    children: Vec<Block>,
}
#[derive(Deserialize)]
struct Blocks {
    blockdevices: Vec<Block>,
}
#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
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
#[derive(Clone, Debug, Serialize)]
pub struct SavedDrive {
    pub uuid: String,
    pub mount: PathBuf,
    pub label: String,
    pub connected: bool,
    pub active: bool,
    pub mounted: bool,
    pub unmounted: bool,
    pub device: Option<String>,
    pub size_bytes: u64,
}
#[derive(Serialize)]
pub struct Drives {
    pub registered: Vec<SavedDrive>,
    pub candidates: Vec<Candidate>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Identity {
    path: String,
    number: String,
    size: u64,
    serial: String,
    model: String,
    diskseq: String,
    rdev: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrivePlan {
    pub token: String,
    pub device: String,
    pub name: String,
    pub model: String,
    pub serial: String,
    pub size_bytes: u64,
    pub confirmation: String,
    pub expires_at: String,
    identity: Identity,
}

fn config() -> Result<Option<Config>> {
    if Path::new(CONFIG).exists() {
        Ok(Some(load_config()?))
    } else {
        Ok(None)
    }
}
fn inventory() -> Result<Vec<Block>> {
    command("udevadm", &["settle", "--timeout=10"])?;
    let blocks: Blocks = serde_json::from_str(&command(
        "lsblk",
        &[
            "--json",
            "--bytes",
            "--tree",
            "--output",
            "NAME,PATH,TYPE,SIZE,FSTYPE,LABEL,UUID,MOUNTPOINTS,MODEL,SERIAL,RO,MAJ:MIN",
        ],
    )?)?;
    Ok(blocks.blockdevices)
}
fn all_nodes<'a>(nodes: &'a [Block], out: &mut Vec<&'a Block>) {
    for b in nodes {
        out.push(b);
        all_nodes(&b.children, out);
    }
}
fn saved(c: &Config) -> Vec<BackupTarget> {
    let mut targets = c.targets.clone();
    if !targets.iter().any(|t| t.uuid == c.backup_uuid) {
        targets.insert(
            0,
            BackupTarget {
                uuid: c.backup_uuid.clone(),
                mount: c.destination.clone(),
                label: "Snap Back".into(),
            },
        );
    }
    targets
}
fn protected(c: Option<&Config>) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    if let Some(c) = c {
        set.insert(c.source_uuid.clone());
        for target in saved(c) {
            set.insert(target.uuid);
        }
    }
    set
}
fn reason(b: &Block, c: Option<&Config>) -> Option<String> {
    let mut nodes = Vec::new();
    all_nodes(std::slice::from_ref(b), &mut nodes);
    if nodes.iter().any(|n| n.ro) {
        return Some("Read-only drive".into());
    }
    let source = c.map(|c| c.source_uuid.as_str());
    if nodes.iter().any(|n| {
        n.uuid.as_deref().is_some_and(|u| Some(u) == source)
            || n.mountpoints
                .iter()
                .flatten()
                .any(|p| matches!(p.as_str(), "/" | "/boot" | "/home" | "/usr" | "/var"))
    }) {
        return Some("System drive — cannot be erased".into());
    }
    let protected = protected(c);
    if nodes
        .iter()
        .any(|n| n.uuid.as_ref().is_some_and(|u| protected.contains(u)))
    {
        return Some("Saved backup drive — choose Use drive instead".into());
    }
    if nodes
        .iter()
        .any(|n| n.mountpoints.iter().flatten().any(|p| !p.is_empty()))
    {
        return Some("Drive is mounted or used for swap. Unmount it before setup.".into());
    }
    for n in &nodes {
        let holders = Path::new("/sys/class/block").join(&n.name).join("holders");
        match fs::read_dir(&holders) {
            Ok(mut entries) => {
                if entries.next().is_some() {
                    return Some("Drive is in use by another block device".into());
                }
            }
            Err(_) => return Some("Cannot verify whether this drive is in use".into()),
        }
    }
    if b.size < 1_073_741_824 {
        return Some("Drive must be at least 1 GiB".into());
    }
    None
}
fn real_disks(nodes: &[Block]) -> Vec<&Block> {
    nodes
        .iter()
        .filter(|b| {
            b.kind == "disk"
                && !b.name.starts_with("zram")
                && !b.name.starts_with("ram")
                && !b.name.starts_with("loop")
        })
        .collect()
}
fn identity(b: &Block) -> Result<Identity> {
    let path = Path::new(&b.path);
    ensure!(
        path.starts_with("/dev") && path.canonicalize()? == path,
        "Device must be its canonical block-device path"
    );
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.file_type().is_block_device(),
        "Selected path is not a block device"
    );
    let diskseq = fs::read_to_string(Path::new("/sys/class/block").join(&b.name).join("diskseq"))
        .context("Cannot identify this physical drive safely")?
        .trim()
        .to_owned();
    ensure!(!diskseq.is_empty(), "Missing drive identity");
    Ok(Identity {
        path: b.path.clone(),
        number: b.number.clone(),
        size: b.size,
        serial: b.serial.clone().unwrap_or_default(),
        model: b.model.clone().unwrap_or_default(),
        diskseq,
        rdev: meta.rdev(),
    })
}
fn candidate(b: &Block, c: Option<&Config>) -> Candidate {
    let reason = reason(b, c);
    Candidate {
        device: b.path.clone(),
        name: b.name.clone(),
        model: b.model.clone().unwrap_or_default(),
        serial: b.serial.clone().unwrap_or_default(),
        size_bytes: b.size,
        fstype: b.fstype.clone(),
        label: b.label.clone(),
        eligible: reason.is_none(),
        reason,
    }
}
pub fn drives() -> Result<Drives> {
    root()?;
    let config = config()?;
    let nodes = inventory()?;
    let mut all = Vec::new();
    all_nodes(&nodes, &mut all);
    let registered = if let Some(c) = &config {
        saved(c)
            .into_iter()
            .map(|t| {
                let device = all
                    .iter()
                    .find(|n| n.uuid.as_deref() == Some(t.uuid.as_str()));
                SavedDrive {
                    active: t.uuid == c.backup_uuid,
                    mounted: mounted(&t).unwrap_or(false),
                    unmounted: intentionally_unmounted(&t.uuid),
                    connected: device.is_some(),
                    uuid: t.uuid,
                    mount: t.mount,
                    label: t.label,
                    device: device.map(|n| n.path.clone()),
                    size_bytes: device.map(|n| n.size).unwrap_or(0),
                }
            })
            .collect()
    } else {
        vec![]
    };
    Ok(Drives {
        registered,
        candidates: real_disks(&nodes)
            .into_iter()
            .map(|b| candidate(b, config.as_ref()))
            .collect(),
    })
}
fn no_recovery() -> Result<()> {
    ensure!(
        recovery::pending().is_none() && !Path::new("/var/lib/snap-back/recovery-point").exists(),
        "Complete or cancel pending system recovery before changing backup drives"
    );
    Ok(())
}
fn token() -> Result<String> {
    let mut bytes = [0u8; 24];
    ensure!(
        unsafe { libc::getrandom(bytes.as_mut_ptr().cast(), bytes.len(), 0) }
            == bytes.len() as isize,
        "Could not create setup confirmation"
    );
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
pub fn plan_drive(device: &Path) -> Result<DrivePlan> {
    let _lock = Operation::start()?;
    no_recovery()?;
    let config = config()?;
    let nodes = inventory()?;
    let b = real_disks(&nodes)
        .into_iter()
        .find(|b| Path::new(&b.path) == device)
        .context("Select a whole physical drive from the drive list")?;
    ensure!(
        reason(b, config.as_ref()).is_none(),
        "{}",
        reason(b, config.as_ref()).unwrap_or_default()
    );
    let plan = DrivePlan {
        token: token()?,
        device: b.path.clone(),
        name: b.name.clone(),
        model: b.model.clone().unwrap_or_default(),
        serial: b.serial.clone().unwrap_or_default(),
        size_bytes: b.size,
        confirmation: format!("ERASE {}", b.path),
        expires_at: (Utc::now() + Duration::minutes(10)).to_rfc3339(),
        identity: identity(b)?,
    };
    fs::create_dir_all(PLANS)?;
    fs::set_permissions(PLANS, fs::Permissions::from_mode(0o700))?;
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(Path::new(PLANS).join(format!("{}.json", plan.token)))?;
    f.write_all(&serde_json::to_vec(&plan)?)?;
    f.sync_all()?;
    Ok(plan)
}
fn validate_plan(plan: &DrivePlan, config: Option<&Config>) -> Result<Block> {
    ensure!(
        DateTime::parse_from_rfc3339(&plan.expires_at)? > Utc::now(),
        "Drive confirmation expired. Select the drive again."
    );
    let nodes = inventory()?;
    let b = real_disks(&nodes)
        .into_iter()
        .find(|b| b.path == plan.device)
        .context("Selected drive was disconnected. Select it again.")?;
    ensure!(
        identity(b)? == plan.identity,
        "Selected drive changed. Select the drive again before erasing."
    );
    ensure!(
        reason(b, config).is_none(),
        "Drive cannot be erased: {}",
        reason(b, config).unwrap_or_default()
    );
    Ok(b.clone())
}
fn safe_parent(path: &Path) -> Result<()> {
    fs::create_dir_all(path)?;
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.is_dir() && !m.file_type().is_symlink() && m.uid() == 0 && m.mode() & 0o022 == 0,
        "Unsafe system setup directory {}",
        path.display()
    );
    Ok(())
}
fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let mut random = [0u8; 8];
    ensure!(
        unsafe { libc::getrandom(random.as_mut_ptr().cast(), random.len(), 0) } == 8,
        "No random source"
    );
    let tmp = path.with_extension(format!("snap-back-{:x}", u64::from_ne_bytes(random)));
    let mut f = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o644)
        .open(&tmp)?;
    let result = (|| -> Result<()> {
        f.write_all(content)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(path.parent().context("No parent")?)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
fn add_fstab(target: &BackupTarget) -> Result<()> {
    let old = fs::read_to_string("/etc/fstab")?;
    for line in old.lines().filter(|l| !l.trim_start().starts_with('#')) {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.first() == Some(&format!("UUID={}", target.uuid).as_str()) {
            ensure!(
                fields.get(1).copied() == target.mount.to_str() && fields.get(2) == Some(&"btrfs"),
                "This drive has a different mount configured in /etc/fstab. Correct that entry before using the drive."
            );
            return Ok(());
        }
    }
    ensure!(
        target
            .mount
            .to_string_lossy()
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/-_".contains(c)),
        "Invalid mount path"
    );
    let new = format!(
        "{}\n# Snap Back backup drive\nUUID={} {} btrfs nofail,noatime,compress=zstd:3,x-systemd.device-timeout=5s 0 0\n",
        old.trim_end(),
        target.uuid,
        target.mount.display()
    );
    write_atomic(Path::new("/etc/fstab"), new.as_bytes())
}
fn activate(config: &mut Config, target: BackupTarget, include_for_backup: bool) -> Result<()> {
    update_target(config, target, include_for_backup)?;
    write_atomic(Path::new(CONFIG), &serde_json::to_vec_pretty(config)?)?;
    Ok(())
}
fn update_target(
    config: &mut Config,
    target: BackupTarget,
    include_for_backup: bool,
) -> Result<()> {
    let mut selected = config
        .selected_targets(None)?
        .into_iter()
        .map(|target| target.uuid)
        .collect::<Vec<_>>();
    if include_for_backup && !selected.contains(&target.uuid) {
        selected.push(target.uuid.clone());
    }
    let mut targets = saved(config);
    if let Some(t) = targets.iter_mut().find(|t| t.uuid == target.uuid) {
        *t = target.clone()
    } else {
        targets.push(target.clone());
    }
    config.destination = target.mount.clone();
    config.backup_uuid = target.uuid.clone();
    config.targets = targets;
    config.backup_targets = Some(selected);
    config.registered_targets()?;
    Ok(())
}
fn mounted(target: &BackupTarget) -> Result<bool> {
    match command(
        "findmnt",
        &[
            "-n",
            "-M",
            target.mount.to_str().context("Invalid mount path")?,
            "-o",
            "UUID,FSTYPE",
        ],
    ) {
        Ok(text) => {
            let fields = text.split_whitespace().collect::<Vec<_>>();
            ensure!(
                fields.first() == Some(&target.uuid.as_str()) && fields.get(1) == Some(&"btrfs"),
                "Another filesystem is mounted at {}",
                target.mount.display()
            );
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}
fn mount_target(target: &BackupTarget) -> Result<bool> {
    if mounted(target)? {
        return Ok(false);
    }
    safe_parent(&target.mount)?;
    ensure!(
        fs::read_dir(&target.mount)?.next().is_none(),
        "Backup mount directory is not empty"
    );
    command(
        "mount",
        &[
            "-t",
            "btrfs",
            "-o",
            "noatime,compress=zstd:3",
            &format!("UUID={}", target.uuid),
            target.mount.to_str().context("Invalid mount path")?,
        ],
    )?;
    if let Err(error) = mounted(target).and_then(|yes| {
        ensure!(yes, "Backup drive did not mount");
        Ok(())
    }) {
        let _ = command(
            "umount",
            &[target.mount.to_str().context("Invalid mount path")?],
        );
        return Err(error);
    }
    Ok(true)
}

pub(crate) fn ensure_target_mounted(config: &Config, target: &BackupTarget) -> Result<()> {
    ensure!(
        target.uuid != config.source_uuid,
        "System filesystem cannot be a backup drive"
    );
    let nodes = inventory()?;
    let mut all = Vec::new();
    all_nodes(&nodes, &mut all);
    let matching = all
        .iter()
        .filter(|node| node.uuid.as_deref() == Some(&target.uuid))
        .collect::<Vec<_>>();
    ensure!(
        matching.len() == 1 && matching[0].fstype.as_deref() == Some("btrfs"),
        "Saved backup drive is disconnected, ambiguous, or no longer Btrfs"
    );
    ensure!(
        !intentionally_unmounted(&target.uuid),
        "Backup drive was unmounted. Choose Use drive to resume backups"
    );
    mount_target(target)?;
    Ok(())
}

pub fn set_backup_drives(uuids: &[String]) -> Result<String> {
    let _lock = Operation::start()?;
    no_recovery()?;
    let mut config = load_config()?;
    let selected = config.selected_targets(Some(uuids))?;
    config.backup_targets = Some(selected.iter().map(|target| target.uuid.clone()).collect());
    crate::atomic_json(Path::new(CONFIG), &config)?;
    Ok(format!(
        "Backups will use {}.",
        selected
            .iter()
            .map(|target| target.label.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

struct Originals {
    fstab: Vec<u8>,
    config: Option<Vec<u8>>,
}
impl Originals {
    fn read() -> Result<Self> {
        let fstab = fs::read("/etc/fstab")
            .context("Cannot read mount configuration; no drive data was changed")?;
        let config = if Path::new(CONFIG).exists() {
            Some(fs::read(CONFIG)?)
        } else {
            None
        };
        Ok(Self { fstab, config })
    }
    fn restore(&self) -> Result<()> {
        if fs::read("/etc/fstab").ok().as_ref() != Some(&self.fstab) {
            write_atomic(Path::new("/etc/fstab"), &self.fstab)?;
        }
        if let Some(bytes) = &self.config {
            if fs::read(CONFIG).ok().as_ref() != Some(bytes) {
                write_atomic(Path::new(CONFIG), bytes)?;
            }
        } else if Path::new(CONFIG).exists() {
            fs::remove_file(CONFIG)?;
        }
        Ok(())
    }
}

fn device_command(
    program: &str,
    args: &[&str],
    device: &File,
    expected: &Identity,
) -> Result<String> {
    // Resolve tools through the already-open block inode, never a /dev name
    // which could be reassigned by hotplug. The fd remains open until setup ends.
    let mut diskseq = 0u64;
    ensure!(
        unsafe {
            libc::ioctl(
                device.as_raw_fd(),
                0x80081280 as libc::c_ulong,
                &mut diskseq,
            )
        } == 0
            && diskseq.to_string() == expected.diskseq
            && device.metadata()?.rdev() == expected.rdev,
        "Physical drive disconnected or changed; setup was stopped"
    );
    let path = format!("/proc/{}/fd/{}", std::process::id(), device.as_raw_fd());
    let mut args = args.to_vec();
    args.push(&path);
    command(program, &args)
}

pub fn setup_drive(token: &str, confirmation: &str) -> Result<String> {
    let _lock = Operation::start()?;
    no_recovery()?;
    ensure!(
        token.len() == 48 && token.chars().all(|c| c.is_ascii_hexdigit()),
        "Invalid drive confirmation token"
    );
    let file = Path::new(PLANS).join(format!("{token}.json"));
    let plan: DrivePlan = serde_json::from_slice(
        &fs::read(&file).context("Drive confirmation no longer exists. Select the drive again.")?,
    )?;
    ensure!(
        plan.token == token
            && confirmation == plan.confirmation
            && confirmation == format!("ERASE {}", plan.device),
        "Type the exact erase confirmation for the selected drive"
    );
    let mut config = config()?;
    let b = validate_plan(&plan, config.as_ref())?;
    let originals = Originals::read()?;
    if config.is_none() {
        ensure!(
            command("findmnt", &["-n", "-M", "/", "-o", "FSTYPE"])? == "btrfs",
            "System root must use Btrfs; no drive data was changed"
        );
        let root_uuid = command("findmnt", &["-n", "-M", "/", "-o", "UUID"])?;
        let home_uuid = command("findmnt", &["-n", "-M", "/home", "-o", "UUID"])?;
        ensure!(
            home_uuid == root_uuid
                && command("findmnt", &["-n", "-M", "/home", "-o", "FSROOT"])?
                    != command("findmnt", &["-n", "-M", "/", "-o", "FSROOT"])?,
            "Separate root and home Btrfs subvolumes on the same filesystem are required; no drive data was changed"
        );
        command("snapper", &["--jsonout", "list-configs"])?;
    }
    for binary in [
        "/usr/bin/wipefs",
        "/usr/bin/mkfs.btrfs",
        "/usr/bin/mount",
        "/usr/bin/blkid",
        "/usr/bin/blockdev",
        "/usr/bin/udevadm",
    ] {
        ensure!(
            Path::new(binary).is_file(),
            "Required setup tool is missing: {binary}"
        );
    }
    safe_parent(Path::new("/backup"))?;
    safe_parent(Path::new(MOUNTS))?;
    safe_parent(Path::new("/etc/snap-back"))?;
    // Consume the one-shot plan before destruction. A failed attempt requires a
    // fresh device check and a fresh explicit confirmation.
    fs::remove_file(&file)?;
    let pinned = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&b.path)?;
    ensure!(
        pinned.metadata()?.rdev() == plan.identity.rdev,
        "Drive identity changed"
    );
    validate_plan(&plan, config.as_ref())?;
    crate::write_progress(&format!(
        "Erasing {} and creating the backup filesystem",
        plan.device
    ));
    pinned
        .try_lock_exclusive()
        .context("Another disk tool is using this drive")?;
    device_command("wipefs", &["--all", "--lock=no"], &pinned, &plan.identity)
        .context("Could not erase drive signatures; the previous backup drive remains selected")?;
    device_command("blockdev", &["--rereadpt"], &pinned, &plan.identity)?;
    // Let udev process partition removal before checking the post-wipe tree.
    // Holding the cooperative flock while waiting for udev would deadlock it.
    FileExt::unlock(&pinned)?;
    command("udevadm", &["settle", "--timeout=10"])?;
    // udev may have recreated the node after the old partition table was removed.
    // Its kernel disk sequence and physical identity must still match the plan.
    let fresh = inventory()?;
    let fresh = real_disks(&fresh)
        .into_iter()
        .find(|d| d.path == plan.device)
        .context("Drive disconnected during setup")?;
    ensure!(
        identity(fresh)? == plan.identity,
        "Drive changed during setup; formatting was stopped"
    );
    ensure!(
        reason(fresh, config.as_ref()).is_none(),
        "Drive became busy during setup"
    );
    pinned
        .try_lock_exclusive()
        .context("Another disk tool is using this drive")?;
    device_command("mkfs.btrfs", &["--force","--label","SNAPBACK"],&pinned,&plan.identity).context("Formatting failed; the selected drive may be partially erased, and the previous backup drive remains selected")?;
    let uuid = device_command(
        "blkid",
        &["-p", "-s", "UUID", "-o", "value"],
        &pinned,
        &plan.identity,
    )?;
    ensure!(
        uuid.chars().all(|c| c.is_ascii_hexdigit() || c == '-') && uuid.len() == 36,
        "Formatter returned an invalid filesystem UUID"
    );
    FileExt::unlock(&pinned)?;
    command("udevadm", &["settle", "--timeout=10"])?;
    let target = BackupTarget {
        mount: Path::new(MOUNTS).join(&uuid),
        uuid,
        label: format!("Snap Back ({})", plan.name),
    };
    crate::write_progress("Mounting and saving the new backup drive");
    let configured = (|| -> Result<()> {
        mount_target(&target)?;
        add_fstab(&target)?;
        if let Some(c) = config.as_mut() {
            activate(c, target.clone(), true)?;
        } else {
            setup::configure(&target.mount)?;
            let mut c = load_config()?;
            activate(&mut c, target.clone(), true)?;
        }
        Ok(())
    })();
    if let Err(e) = configured {
        let rollback = originals.restore();
        if mounted(&target).unwrap_or(false) {
            let _ = command("umount", &[target.mount.to_str().context("Invalid mount")?]);
        }
        rollback.context("Drive was erased and restoring the previous configuration failed; inspect /etc/snap-back/config.json and /etc/fstab")?;
        return Err(e.context("Drive was erased, but setup could not be completed. The previous backup configuration was restored."));
    }
    let _ = command("systemctl", &["daemon-reload"]);
    Ok(format!(
        "{} is ready and included in backups. Other backup drives and their backups were kept.",
        target.label
    ))
}
// Runtime-only suppression prevents the hourly service from remounting a drive
// the user has just unmounted. Explicit Use drive (or reboot) resumes it.
fn unmounted_marker(uuid: &str) -> Result<PathBuf> {
    crate::valid_id(uuid)?;
    Ok(Path::new("/run/snap-back/unmounted").join(uuid))
}
pub(crate) fn intentionally_unmounted(uuid: &str) -> bool {
    unmounted_marker(uuid).is_ok_and(|path| path.exists())
}
pub fn unmount_drive(uuid: &str) -> Result<String> {
    let _lock = Operation::start()?;
    no_recovery()?;
    let c = load_config()?;
    let target = c
        .registered_targets()?
        .into_iter()
        .find(|t| t.uuid == uuid)
        .context("Choose a saved backup drive")?;
    ensure!(
        uuid != c.source_uuid,
        "The system filesystem cannot be unmounted"
    );
    ensure!(mounted(&target)?, "This backup drive is already unmounted");
    let marker = unmounted_marker(uuid)?;
    fs::create_dir_all(marker.parent().unwrap())?;
    // Write the suppression before umount so a crash cannot re-enable backups.
    fs::write(&marker, b"Unmounted by user\n")?;
    if let Err(error) = command(
        "umount",
        &["--", target.mount.to_str().context("Invalid mount path")?],
    ) {
        let _ = fs::remove_file(&marker);
        return Err(error.context("Could not unmount the backup drive. Close files or applications using it and try again."));
    }
    Ok(format!(
        "{} is unmounted. Use drive resumes backups; restarting also resumes them. Other partitions on this disk may still be in use.",
        target.label
    ))
}

pub fn use_drive(uuid: &str) -> Result<String> {
    let _lock = Operation::start()?;
    no_recovery()?;
    let mut c = load_config()?;
    let target = saved(&c)
        .into_iter()
        .find(|t| t.uuid == uuid)
        .context("Choose a saved backup drive")?;
    let nodes = inventory()?;
    let mut all = Vec::new();
    all_nodes(&nodes, &mut all);
    let matching = all
        .iter()
        .filter(|n| n.uuid.as_deref() == Some(uuid))
        .collect::<Vec<_>>();
    ensure!(
        matching.len() == 1 && matching[0].fstype.as_deref() == Some("btrfs"),
        "Saved backup drive is disconnected, ambiguous, or no longer Btrfs"
    );
    ensure!(
        uuid != c.source_uuid,
        "System filesystem cannot be used as the backup drive"
    );
    let originals = Originals::read()?;
    let new_mount = mount_target(&target)?;
    let result = (|| -> Result<()> {
        add_fstab(&target)?;
        activate(&mut c, target.clone(), false)
    })();
    if let Err(error) = result {
        let rollback = originals.restore();
        if new_mount {
            let _ = command("umount", &[target.mount.to_str().context("Invalid mount")?]);
        }
        rollback.context("Could not restore the previous backup configuration")?;
        return Err(error);
    }
    let marker = unmounted_marker(uuid)?;
    if marker.exists() {
        fs::remove_file(marker)?;
    }
    let _ = command("systemctl", &["daemon-reload"]);
    Ok(format!(
        "Showing backups on {}. Backup destinations were kept. No data was erased.",
        target.label
    ))
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    fn config() -> Config {
        serde_json::from_value(serde_json::json!({
            "destination":"/backup/first", "backup_uuid":"first", "source_uuid":"system",
            "sources":[], "targets":[
                {"uuid":"first","mount":"/backup/first","label":"First"},
                {"uuid":"second","mount":"/backup/second","label":"Second"}
            ]
        }))
        .unwrap()
    }
    #[test]
    fn changing_browse_drive_materializes_and_keeps_legacy_backup_selection() {
        let mut c = config();
        assert!(c.backup_targets.is_none());
        let second = c.targets[1].clone();
        update_target(&mut c, second, false).unwrap();
        assert_eq!(c.backup_uuid, "second");
        assert_eq!(c.destination, Path::new("/backup/second"));
        assert_eq!(c.backup_targets, Some(vec!["first".into()]));
    }
    #[test]
    fn adding_a_drive_keeps_other_selections_and_does_not_duplicate_targets() {
        let mut c = config();
        let second = c.targets[1].clone();
        update_target(&mut c, second.clone(), true).unwrap();
        update_target(&mut c, second, true).unwrap();
        assert_eq!(
            c.backup_targets,
            Some(vec!["first".into(), "second".into()])
        );
        assert_eq!(c.targets.len(), 2);
    }
    #[test]
    fn target_selection_deduplicates_and_rejects_unknown_empty_and_source_drives() {
        let mut c = config();
        assert_eq!(c.selected_targets(None).unwrap()[0].uuid, "first");
        let requested = vec!["second".into(), "first".into(), "second".into()];
        assert_eq!(
            c.selected_targets(Some(&requested))
                .unwrap()
                .iter()
                .map(|t| t.uuid.as_str())
                .collect::<Vec<_>>(),
            ["second", "first"]
        );
        for requested in [vec![], vec!["unknown".into()], vec!["../first".into()]] {
            assert!(c.selected_targets(Some(&requested)).is_err());
        }
        c.targets[1].uuid = "system".into();
        assert!(c.selected_targets(Some(&["system".into()])).is_err());
    }
    #[test]
    fn target_registry_rejects_shared_mounts_and_relative_paths() {
        let mut c = config();
        c.targets[1].mount = c.targets[0].mount.clone();
        assert!(c.registered_targets().is_err());
        c.targets[1].mount = "relative".into();
        assert!(c.registered_targets().is_err());
    }
}
