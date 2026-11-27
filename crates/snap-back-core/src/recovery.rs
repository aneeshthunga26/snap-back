//! Staged, offline recovery for CachyOS with Limine and mkinitcpio.
//!
//! Cross-device receives run before reboot. The initramfs then switches complete
//! subvolume sets before sysroot is mounted. Old volumes remain as rollback data.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Write,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::Duration,
};

const BOOT: &str = "/boot";
const CONTROL: &str = "/boot/snap-back";
const ONLINE_TOP: &str = "/run/snap-back-stage";
const OFFLINE_TOP: &str = "/run/snap-back-system";
const OFFLINE_BOOT: &str = "/run/snap-back-efi";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecoverySource {
    pub name: String,
    /// Filesystem-root-relative Btrfs subvolume path, e.g. `@home`.
    pub subvol: String,
}

pub(crate) struct Replacement {
    pub fstab: String,
    pub limine: String,
    pub old_root_uuid: String,
    pub config: crate::Config,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RecoveryPlan {
    version: u32,
    point_id: String,
    source_uuid: String,
    boot_uuid: String,
    sources: Vec<RecoverySource>,
    staging: String,
    phase: String,
    #[serde(default)]
    error: Option<String>,
}

fn root() -> Result<()> {
    ensure!(
        unsafe { libc::geteuid() } == 0,
        "System recovery requires root privileges"
    );
    Ok(())
}

fn component(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() < 150
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b)),
        "Invalid recovery identifier"
    );
    Ok(())
}

fn subvol_path(value: &str) -> Result<PathBuf> {
    let path = Path::new(value.trim_start_matches('/'));
    ensure!(
        !path.as_os_str().is_empty(),
        "Refusing filesystem-root recovery target"
    );
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_))),
        "Unsafe subvolume path: {value}"
    );
    Ok(path.to_owned())
}

fn run(command: &mut Command) -> Result<String> {
    let display = format!("{command:?}");
    let output = command
        .output()
        .with_context(|| format!("Unable to run {display}"))?;
    ensure!(
        output.status.success(),
        "{display} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn sync() -> Result<()> {
    ensure!(
        unsafe {
            libc::syncfs(
                File::open(CONTROL)
                    .or_else(|_| File::open(OFFLINE_BOOT))?
                    .as_raw_fd(),
            )
        } == 0,
        "Unable to sync recovery metadata"
    );
    Ok(())
}
use std::os::fd::AsRawFd;

fn write_json(path: &Path, plan: &RecoveryPlan) -> Result<()> {
    let temp = path.with_extension("json.tmp");
    let mut file = File::create(&temp)?;
    file.write_all(&serde_json::to_vec_pretty(plan)?)?;
    file.sync_all()?;
    fs::rename(temp, path)?;
    File::open(path.parent().context("No plan parent")?)?.sync_all()?;
    Ok(())
}

fn mount(uuid: &str, dest: &Path, btrfs: bool) -> Result<()> {
    fs::create_dir_all(dest)?;
    let mut cmd = Command::new("mount");
    if btrfs {
        cmd.args(["-t", "btrfs", "-o", "subvolid=5"]);
    }
    cmd.arg(format!("UUID={uuid}")).arg(dest);
    run(&mut cmd)?;
    Ok(())
}

fn unmount(dest: &Path) {
    let _ = Command::new("umount").arg(dest).status();
}

fn receive(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    let mut sender = Command::new("btrfs")
        .arg("send")
        .arg(source)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let receive = Command::new("btrfs")
        .arg("receive")
        .arg(target)
        .stdin(sender.stdout.take().context("Missing send stream")?)
        .output();
    let sent = sender.wait_with_output()?;
    let received = receive?;
    ensure!(
        sent.status.success() && received.status.success(),
        "Recovery staging failed: send: {}; receive: {}",
        String::from_utf8_lossy(&sent.stderr),
        String::from_utf8_lossy(&received.stderr)
    );
    Ok(())
}

fn delete_subvolume(path: &Path) -> Result<()> {
    if path.exists() {
        run(Command::new("btrfs")
            .args(["subvolume", "delete"])
            .arg(path))?;
    }
    Ok(())
}

fn copy_boot(from: &Path, to: &Path, delete: bool) -> Result<()> {
    fs::create_dir_all(to)?;
    let mut cmd = Command::new("rsync");
    cmd.args([
        "-rlt",
        "--exclude=/snap-back/",
        "--exclude=/limine.conf.old",
    ]);
    if delete {
        cmd.arg("--delete");
    }
    cmd.arg(format!("{}/", from.display()))
        .arg(format!("{}/", to.display()));
    run(&mut cmd)?;
    Ok(())
}

fn kernel() -> Result<String> {
    let current = run(Command::new("uname").arg("-r"))?;
    ensure!(
        Path::new("/usr/lib/modules")
            .join(&current)
            .join("vmlinuz")
            .is_file(),
        "Current kernel image unavailable; reboot into an installed kernel first"
    );
    Ok(current)
}

/// Return the pending restore point, if the EFI system partition has a plan.
pub fn pending() -> Option<String> {
    let plan: RecoveryPlan =
        serde_json::from_slice(&fs::read(Path::new(CONTROL).join("pending.json")).ok()?).ok()?;
    if matches!(
        plan.phase.as_str(),
        "prepared" | "retiring" | "installing" | "boot"
    ) {
        Some(plan.point_id)
    } else {
        None
    }
}

/// Stage copies on the system filesystem and make recovery the next Limine boot.
/// This does not reboot. The caller must hold the application's mutation lock.
pub(crate) fn prepare(
    destination: &Path,
    point_id: &str,
    source_uuid: &str,
    sources: &[RecoverySource],
    replacement: Option<Replacement>,
) -> Result<String> {
    root()?;
    component(point_id)?;
    component(source_uuid)?;
    ensure!(!sources.is_empty(), "No subvolumes configured for recovery");
    let existing = Path::new(CONTROL).join("pending.json");
    if existing.exists() {
        let prior: RecoveryPlan = serde_json::from_slice(&fs::read(&existing)?)?;
        if prior.phase == "failed" {
            cancel()?;
        } else {
            bail!("A recovery plan already exists; cancel it before preparing another");
        }
    }
    let mut names = std::collections::HashSet::new();
    let mut targets = std::collections::HashSet::new();
    for source in sources {
        component(&source.name)?;
        let target = subvol_path(&source.subvol)?;
        ensure!(
            names.insert(&source.name) && targets.insert(target),
            "Duplicate recovery source"
        );
        let snapshot = destination
            .join("points")
            .join(point_id)
            .join(&source.name)
            .join("snapshot");
        if replacement.is_some() {
            ensure!(
                fs::canonicalize(&snapshot)? == snapshot,
                "Unexpected path to backup subvolume {}",
                source.name
            );
        }
        ensure!(
            snapshot.is_dir(),
            "Missing backed up subvolume {}",
            source.name
        );
        let flags = run(Command::new("btrfs")
            .args(["property", "get", "-ts"])
            .arg(&snapshot)
            .arg("ro"))?;
        ensure!(
            flags == "ro=true",
            "Backup subvolume {} is not read-only",
            source.name
        );
    }
    ensure!(
        sources
            .iter()
            .any(|s| s.subvol.trim_start_matches('/') == "@"),
        "CachyOS root subvolume @ must be included"
    );
    let boot_source = destination.join("points").join(point_id).join("boot");
    ensure!(
        boot_source.join("limine.conf").is_file(),
        "Restore point has no boot loader backup"
    );
    let boot_uuid = run(Command::new("findmnt").args(["-no", "UUID", "--target", BOOT]))?;
    component(&boot_uuid)?;
    let version = kernel()?;
    fs::create_dir_all(CONTROL)?;
    let mut plan = RecoveryPlan {
        version: 1,
        point_id: point_id.into(),
        source_uuid: source_uuid.into(),
        boot_uuid,
        sources: sources.to_vec(),
        staging: format!(".snap-back-restore-{point_id}"),
        phase: "preparing".into(),
        error: None,
    };
    fs::copy(
        "/boot/limine.conf",
        Path::new(CONTROL).join("limine-before.conf"),
    )?;
    mount(source_uuid, Path::new(ONLINE_TOP), true)?;
    let result = (|| -> Result<()> {
        let staging = Path::new(ONLINE_TOP).join(&plan.staging);
        ensure!(
            !staging.exists(),
            "Recovery staging directory already exists: {}",
            staging.display()
        );
        fs::create_dir_all(staging.join("next"))?;
        fs::create_dir_all(staging.join("before"))?;
        // Check the entire destination layout before arming the boot entry.
        // The offline switch expects an existing volume at every target path.
        if replacement.is_some() {
            for source in &plan.sources {
                let target = Path::new(ONLINE_TOP).join(subvol_path(&source.subvol)?);
                ensure!(
                    fs::canonicalize(&target)? == target && crate::is_subvolume(&target),
                    "Expected destination subvolume {}",
                    source.subvol
                );
            }
        }
        for source in &plan.sources {
            let incoming = staging.join("received").join(&source.name);
            receive(
                &destination
                    .join("points")
                    .join(point_id)
                    .join(&source.name)
                    .join("snapshot"),
                &incoming,
            )?;
            run(Command::new("btrfs")
                .args(["subvolume", "snapshot"])
                .arg(incoming.join("snapshot"))
                .arg(staging.join("next").join(&source.name)))?;
        }
        copy_boot(
            Path::new(BOOT),
            &Path::new(CONTROL).join("boot-before"),
            true,
        )?;
        copy_boot(&boot_source, &Path::new(CONTROL).join("boot-payload"), true)?;
        if let Some(replacement) = replacement {
            let restored_root = staging.join("next/root");
            for relative in ["etc", "etc/snap-back"] {
                let dir = restored_root.join(relative);
                ensure!(
                    fs::canonicalize(&dir)? == dir && dir.is_dir(),
                    "Unexpected directory in restored system: {relative}"
                );
            }
            let fstab = restored_root.join("etc/fstab");
            ensure!(
                fs::symlink_metadata(&fstab)?.is_file(),
                "Backup fstab is not a regular file"
            );
            fs::write(fstab, replacement.fstab)?;
            let config = restored_root.join("etc/snap-back/config.json");
            ensure!(
                fs::symlink_metadata(&config)?.is_file(),
                "Backup configuration is not a regular file"
            );
            fs::write(config, serde_json::to_vec_pretty(&replacement.config)?)?;
            // Keep future Limine entries pointed at the replacement disk too.
            for relative in ["etc/default/limine", "etc/kernel/cmdline"] {
                let path = restored_root.join(relative);
                if path.exists() {
                    ensure!(
                        fs::canonicalize(&path)? == path && path.is_file(),
                        "Unexpected kernel settings path: {relative}"
                    );
                    let old = format!("UUID={}", replacement.old_root_uuid);
                    let new = format!("UUID={}", replacement.config.source_uuid);
                    let settings = fs::read_to_string(&path)?.replace(&old, &new);
                    fs::write(path, settings)?;
                }
            }
            // Preserve the bootloader installed on the destination ESP and use
            // the backed-up kernel paths with an updated root command line.
            let payload = Path::new(CONTROL).join("boot-payload");
            let efi = payload.join("EFI");
            ensure!(
                !efi.exists() || fs::symlink_metadata(&efi)?.is_dir(),
                "Unexpected backup EFI directory"
            );
            copy_boot(&Path::new(BOOT).join("EFI"), &efi, true)?;
            let limine = payload.join("limine.conf");
            ensure!(
                fs::symlink_metadata(&limine)?.is_file(),
                "Backup Limine configuration is not a regular file"
            );
            fs::write(limine, replacement.limine)?;
        }
        fs::copy(
            Path::new("/usr/lib/modules").join(&version).join("vmlinuz"),
            Path::new(CONTROL).join("vmlinuz"),
        )?;
        plan.phase = "prepared".into();
        write_json(&Path::new(CONTROL).join("pending.json"), &plan)?;
        install_initcpio()?;
        fs::write(
            "/run/snap-back-mkinitcpio.conf",
            "MODULES=(btrfs vfat virtio_blk virtio_pci)\nBINARIES=()\nFILES=()\nHOOKS=(base systemd modconf block filesystems snap-back)\nCOMPRESSION=zstd\n",
        )?;
        run(Command::new("mkinitcpio").args([
            "--nopost",
            "-k",
            &version,
            "-c",
            "/run/snap-back-mkinitcpio.conf",
            "-g",
            "/boot/snap-back/recovery.img",
        ]))?;
        let normal = fs::read_to_string("/boot/limine.conf")?;
        let first = normal
            .lines()
            .position(|l| l.starts_with('/'))
            .context("Unsupported Limine configuration: no boot entries")?;
        let lines: Vec<_> = normal.lines().collect();
        let globals = lines[..first]
            .iter()
            .filter(|l| {
                !l.trim_start().starts_with("default_entry:")
                    && !l.trim_start().starts_with("remember_last_entry:")
                    && !l.trim_start().starts_with("timeout:")
            })
            .copied()
            .collect::<Vec<_>>()
            .join("\n");
        let new_config = format!(
            "{globals}\ntimeout: 5\ndefault_entry: 1\nremember_last_entry: no\n\n/Snap Back Recovery\ncomment: Restore root and home from {point_id}; previous volumes are retained\nprotocol: linux\npath: boot():/snap-back/vmlinuz\nmodule_path: boot():/snap-back/recovery.img\ncmdline: rw root=UUID={source_uuid} rootflags=subvol=/@ snap_back.recovery=1 console=tty0 console=ttyS0,115200\n\n{}\n",
            lines[first..].join("\n")
        );
        fs::write("/boot/limine.conf", new_config)?;
        sync()?;
        Ok(())
    })();
    unmount(Path::new(ONLINE_TOP));
    if let Err(error) = result {
        // Leave staging available for diagnosis, but never leave an armed boot on failure.
        let _ = fs::copy(
            Path::new(CONTROL).join("limine-before.conf"),
            "/boot/limine.conf",
        );
        plan.phase = "failed".into();
        plan.error = Some(format!("{error:#}"));
        let _ = write_json(&Path::new(CONTROL).join("pending.json"), &plan);
        return Err(error);
    }
    Ok(format!(
        "Recovery for {point_id} is ready. Restart to restore all backed up subvolumes. Previous data will remain in rollback subvolumes."
    ))
}

fn install_initcpio() -> Result<()> {
    fs::create_dir_all("/usr/lib/initcpio/install")?;
    fs::create_dir_all("/usr/lib/snap-back/sysroot.mount.d")?;
    fs::write(
        "/usr/lib/initcpio/install/snap-back",
        include_str!("../../../packaging/initcpio/snap-back"),
    )?;
    fs::write(
        "/usr/lib/systemd/system/snap-back-recovery.service",
        include_str!("../../../packaging/initcpio/snap-back-recovery.service"),
    )?;
    fs::write(
        "/usr/lib/snap-back/sysroot.mount.d/recovery.conf",
        include_str!("../../../packaging/initcpio/recovery.conf"),
    )?;
    Ok(())
}

/// Disarm a recovery which has not begun; delete only its temporary subvolumes.
pub fn cancel() -> Result<String> {
    root()?;
    let pending = Path::new(CONTROL).join("pending.json");
    if !pending.exists() {
        return Ok("No recovery is pending.".into());
    }
    let plan: RecoveryPlan = serde_json::from_slice(&fs::read(&pending)?)?;
    validate_plan(&plan)?;
    ensure!(
        matches!(plan.phase.as_str(), "prepared" | "preparing" | "failed"),
        "Recovery has already begun; use the recovery boot to finish it"
    );
    mount(&plan.source_uuid, Path::new(ONLINE_TOP), true)?;
    let result = (|| -> Result<()> {
        let staging = Path::new(ONLINE_TOP).join(&plan.staging);
        ensure!(
            !staging
                .join("before")
                .read_dir()
                .map(|mut d| d.next().is_some())
                .unwrap_or(false),
            "Rollback subvolumes exist; refusing to remove them"
        );
        for source in &plan.sources {
            delete_subvolume(&staging.join("next").join(&source.name))?;
            delete_subvolume(&staging.join("received").join(&source.name).join("snapshot"))?;
        }
        if staging.exists() {
            fs::remove_dir_all(staging)?;
        }
        fs::copy(
            Path::new(CONTROL).join("limine-before.conf"),
            "/boot/limine.conf",
        )?;
        fs::remove_file(pending)?;
        let payload = Path::new(CONTROL).join("boot-payload");
        if payload.exists() {
            fs::remove_dir_all(payload)?;
        }
        sync()?;
        Ok(())
    })();
    unmount(Path::new(ONLINE_TOP));
    result?;
    Ok("Pending recovery cancelled. Your current system is unchanged.".into())
}

fn validate_plan(plan: &RecoveryPlan) -> Result<()> {
    ensure!(plan.version == 1, "Unsupported recovery format");
    component(&plan.point_id)?;
    component(&plan.source_uuid)?;
    component(&plan.boot_uuid)?;
    ensure!(
        plan.staging == format!(".snap-back-restore-{}", plan.point_id),
        "Unsafe recovery staging path"
    );
    ensure!(!plan.sources.is_empty(), "Recovery source set is empty");
    let mut names = std::collections::HashSet::new();
    let mut paths = std::collections::HashSet::new();
    for source in &plan.sources {
        component(&source.name)?;
        let path = subvol_path(&source.subvol)?;
        ensure!(
            names.insert(&source.name) && paths.insert(path),
            "Duplicate recovery subvolume"
        );
    }
    Ok(())
}

fn await_device(uuid: &str) -> Result<()> {
    let path = Path::new("/dev/disk/by-uuid").join(uuid);
    for _ in 0..120 {
        if path.exists() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(250));
    }
    bail!("Recovery device UUID={uuid} did not appear");
}

fn depth(source: &RecoverySource) -> usize {
    source.subvol.trim_start_matches('/').split('/').count()
}

fn preserve_snapshots(
    top: &Path,
    staging: &Path,
    source: &RecoverySource,
    reverse: bool,
) -> Result<()> {
    let current = top.join(subvol_path(&source.subvol)?).join(".snapshots");
    let old = staging.join("before").join(&source.name).join(".snapshots");
    let (from, to) = if reverse {
        (&current, &old)
    } else {
        (&old, &current)
    };
    // A Btrfs snapshot contains an empty placeholder for a nested subvolume.
    // Never move ordinary backed-up .snapshots content or overwrite a non-empty tree.
    if from.exists() && run(Command::new("btrfs").args(["subvolume", "show"]).arg(from)).is_ok() {
        if to.exists() {
            fs::remove_dir(to).with_context(|| {
                format!("Snapshot mount placeholder {} is not empty", to.display())
            })?;
        }
        fs::rename(from, to)?;
    }
    Ok(())
}

fn rollback(top: &Path, plan: &RecoveryPlan) -> Result<()> {
    let staging = top.join(&plan.staging);
    let mut sources = plan.sources.clone();
    sources.sort_by_key(|s| std::cmp::Reverse(depth(s)));
    for source in &sources {
        let before = staging.join("before").join(&source.name);
        let current = top.join(subvol_path(&source.subvol)?);
        let next = staging.join("next").join(&source.name);
        if before.exists() && current.exists() && !next.exists() {
            preserve_snapshots(top, &staging, source, true)?;
            fs::rename(&current, &next)?;
        }
    }
    sources.sort_by_key(depth);
    for source in &sources {
        let before = staging.join("before").join(&source.name);
        if before.exists() {
            let current = top.join(subvol_path(&source.subvol)?);
            if current.exists() {
                fs::remove_dir(&current)?;
            }
            if let Some(parent) = current.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(before, current)?;
        }
    }
    Ok(())
}

/// Called only by the dedicated recovery initramfs before sysroot.mount.
/// Rejects invocation on a running system. Its durable journal supports retries.
pub fn recover_offline(plan_path: &Path) -> Result<()> {
    root()?;
    ensure!(
        Path::new("/etc/initrd-release").exists(),
        "Offline recovery must run from the Snap Back recovery initramfs"
    );
    ensure!(
        fs::read_to_string("/proc/cmdline")?
            .split_whitespace()
            .any(|s| s == "snap_back.recovery=1"),
        "Recovery kernel flag missing"
    );
    ensure!(
        !fs::read_to_string("/proc/self/mountinfo")?
            .lines()
            .any(|line| line.split_whitespace().nth(4) == Some("/sysroot")),
        "Refusing recovery because /sysroot is already mounted"
    );
    let initial: RecoveryPlan = serde_json::from_slice(&fs::read(plan_path)?)?;
    validate_plan(&initial)?;
    await_device(&initial.boot_uuid)?;
    await_device(&initial.source_uuid)?;
    mount(&initial.boot_uuid, Path::new(OFFLINE_BOOT), false)?;
    mount(&initial.source_uuid, Path::new(OFFLINE_TOP), true)?;
    let control = Path::new(OFFLINE_BOOT).join("snap-back");
    let journal = control.join("pending.json");
    let mut plan: RecoveryPlan = serde_json::from_slice(&fs::read(&journal)?)?;
    validate_plan(&plan)?;
    ensure!(
        plan.point_id == initial.point_id
            && plan.source_uuid == initial.source_uuid
            && plan.boot_uuid == initial.boot_uuid,
        "Recovery journal does not match initramfs"
    );
    eprintln!(
        "Snap Back: recovering {} (phase {})",
        plan.point_id, plan.phase
    );
    let top = Path::new(OFFLINE_TOP);
    let staging = top.join(&plan.staging);
    let result = (|| -> Result<()> {
        ensure!(
            matches!(
                plan.phase.as_str(),
                "prepared" | "retiring" | "installing" | "boot" | "completed"
            ),
            "Recovery plan is not armed: {}",
            plan.phase
        );
        let mut sources = plan.sources.clone();
        if matches!(plan.phase.as_str(), "prepared" | "retiring") {
            plan.phase = "retiring".into();
            write_json(&journal, &plan)?;
            sources.sort_by_key(|s| std::cmp::Reverse(depth(s)));
            for source in &sources {
                let before = staging.join("before").join(&source.name);
                if !before.exists() {
                    eprintln!("Snap Back: retaining previous {}", source.subvol);
                    fs::rename(top.join(subvol_path(&source.subvol)?), &before)
                        .with_context(|| format!("Retiring {}", source.subvol))?;
                }
            }
            plan.phase = "installing".into();
            write_json(&journal, &plan)?;
        }
        if plan.phase == "installing" {
            sources.sort_by_key(depth);
            for source in &sources {
                let next = staging.join("next").join(&source.name);
                let target = top.join(subvol_path(&source.subvol)?);
                if next.exists() {
                    if target.exists() {
                        fs::remove_dir(&target).with_context(|| {
                            format!("Non-empty subvolume placeholder {}", target.display())
                        })?;
                    }
                    if let Some(parent) = target.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    eprintln!("Snap Back: installing restored {}", source.subvol);
                    fs::rename(next, &target)?;
                }
                ensure!(
                    target.exists(),
                    "Staged subvolume {} disappeared",
                    source.name
                );
            }
            for source in &sources {
                preserve_snapshots(top, &staging, source, false)?;
            }
            plan.phase = "boot".into();
            write_json(&journal, &plan)?;
        }
        if plan.phase == "boot" {
            eprintln!("Snap Back: restoring matching boot files");
            copy_boot(&control.join("boot-payload"), Path::new(OFFLINE_BOOT), true)?;
            plan.phase = "completed".into();
            write_json(&journal, &plan)?;
        }
        fs::rename(&journal, control.join("last-recovery.json"))?;
        run(Command::new("btrfs").args(["filesystem", "sync"]).arg(top))?;
        File::open(OFFLINE_BOOT)?.sync_all()?;
        Ok(())
    })();
    if let Err(error) = result {
        let rollback_result = rollback(top, &plan);
        plan.phase = "failed".into();
        plan.error = Some(format!(
            "Recovery failed: {error:#}; rollback: {rollback_result:?}"
        ));
        let _ = write_json(&journal, &plan);
        let boot_rollback_result = if rollback_result.is_ok() {
            (|| -> Result<()> {
                // A successful subvolume rollback is only safe to boot after
                // restoring its matching kernel/initramfs and normal boot menu.
                copy_boot(&control.join("boot-before"), Path::new(OFFLINE_BOOT), true)
                    .context("Restoring the original boot files")?;
                fs::copy(
                    control.join("limine-before.conf"),
                    Path::new(OFFLINE_BOOT).join("limine.conf"),
                )
                .context("Restoring the normal boot selection")?;
                let original_root = plan
                    .sources
                    .iter()
                    .find(|source| source.subvol.trim_start_matches('/') == "@")
                    .context("Original root is not in the recovery plan")?;
                let state = top
                    .join(subvol_path(&original_root.subvol)?)
                    .join("var/lib/snap-back");
                fs::create_dir_all(&state)?;
                fs::write(
                    state.join("last-error"),
                    format!(
                        "System recovery failed and was rolled back safely. Your previous system was restored. Details: {error:#}"
                    ),
                )?;
                let marker = state.join("recovery-point");
                match fs::remove_file(&marker) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error).context("Clearing the recovery marker"),
                }
                write_json(&journal, &plan)?;
                run(Command::new("btrfs").args(["filesystem", "sync"]).arg(top))?;
                File::open(OFFLINE_BOOT)?.sync_all()?;
                run(Command::new("umount").arg(top))?;
                run(Command::new("umount").arg(OFFLINE_BOOT))?;
                eprintln!(
                    "Snap Back: recovery failed; previous system and boot files restored safely, rebooting normally"
                );
                reboot_normally()
            })()
        } else {
            Err(anyhow::anyhow!(
                "Original subvolumes could not all be restored"
            ))
        };
        if let Err(boot_error) = boot_rollback_result {
            plan.error = Some(format!(
                "Recovery failed: {error:#}; rollback: {rollback_result:?}; boot rollback: {boot_error:#}"
            ));
            let _ = write_json(&journal, &plan);
            unmount(top);
            unmount(Path::new(OFFLINE_BOOT));
            return Err(error.context(format!(
                "Recovery could not safely reboot: {boot_error:#}. See EFI snap-back/pending.json"
            )));
        }
        return Ok(());
    }
    unmount(top);
    unmount(Path::new(OFFLINE_BOOT));
    // Boot normally with the restored kernel/initramfs, never continue with a
    // recovery kernel whose modules may differ from the selected restore point.
    eprintln!("Snap Back: recovery complete, rebooting normally");
    reboot_normally()
}

fn reboot_normally() -> Result<()> {
    unsafe {
        libc::sync();
    }
    let result = unsafe { libc::reboot(libc::RB_AUTOBOOT) };
    ensure!(
        result == 0,
        "System reboot failed: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}
