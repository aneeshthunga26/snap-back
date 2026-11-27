//! Replace a compatible new installation from a backup on a separate drive.
use crate::{
    BackupTarget, Config, Operation, Point, Source, command, points_on, ps, recovery, valid_id,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
};

fn mount_value(path: &Path, column: &str) -> Result<String> {
    command("findmnt", &["-n", "-M", ps(path)?, "-o", column])
}

fn backup_drive(path: &Path) -> Result<(PathBuf, String)> {
    let path = path
        .canonicalize()
        .context("Open the mounted backup drive")?;
    ensure!(
        mount_value(&path, "FSTYPE")? == "btrfs" && mount_value(&path, "FSROOT")? == "/",
        "Choose the mount path of the backup drive's top-level Btrfs filesystem"
    );
    let uuid = mount_value(&path, "UUID")?;
    valid_id(&uuid)?;
    ensure!(
        uuid != mount_value(Path::new("/"), "UUID")?,
        "Choose a separate backup filesystem"
    );
    let points = path.join("points");
    ensure!(
        fs::symlink_metadata(&points).is_ok_and(|m| m.is_dir()),
        "No Snap Back points directory on this drive"
    );
    Ok((path, uuid))
}

pub fn points(backup_mount: &Path) -> Result<Vec<Point>> {
    let (destination, _) = backup_drive(backup_mount)?;
    points_on(&destination)
}

fn uuid_source(source: &str, uuid: &str) -> bool {
    source
        .strip_prefix("UUID=")
        .is_some_and(|value| value.eq_ignore_ascii_case(uuid))
}

/// Keep the destination's storage configuration. Mounts for the restored
/// system must already match the backed-up subvolume layout.
fn replacement_fstab(
    current: &str,
    system_uuid: &str,
    boot_uuid: &str,
    backup: &BackupTarget,
    sources: &[Source],
) -> Result<String> {
    let mut system_mounts = BTreeSet::new();
    let mut boot_found = false;
    let mut lines = Vec::new();
    for line in current.lines() {
        let fields = line
            .split_whitespace()
            .take_while(|field| !field.starts_with('#'))
            .collect::<Vec<_>>();
        if fields.is_empty() {
            lines.push(line);
            continue;
        }
        let [device, mount, filesystem, options, ..] = fields.as_slice() else {
            anyhow::bail!("Incomplete destination fstab entry: {line}");
        };
        if uuid_source(device, &backup.uuid) {
            continue;
        }
        if *mount == "/boot" && *filesystem == "vfat" && uuid_source(device, boot_uuid) {
            ensure!(!boot_found, "Duplicate /boot in the destination fstab");
            boot_found = true;
        } else {
            let source = sources
                .iter()
                .find(|source| source.mount == Path::new(mount));
            ensure!(
                *filesystem == "btrfs"
                    && uuid_source(device, system_uuid)
                    && source.is_some_and(|source| options.split(',').any(|option| option
                        .strip_prefix("subvol=")
                        .is_some_and(|name| name.trim_start_matches('/')
                            == source.subvol.trim_start_matches('/')))),
                "Unsupported destination fstab entry at {mount}. Use a matching plain Btrfs CachyOS layout with /boot on its EFI partition."
            );
            ensure!(
                system_mounts.insert(*mount),
                "Duplicate destination mount {mount}"
            );
        }
        lines.push(line);
    }
    ensure!(
        boot_found && system_mounts.contains("/") && system_mounts.contains("/home"),
        "The destination fstab must mount root, home, and its EFI partition"
    );
    let mut fstab = lines.join("\n");
    for source in sources {
        if system_mounts.contains(source.mount.to_str().context("Invalid source mount")?) {
            continue;
        }
        let parent = sources
            .iter()
            .filter(|parent| {
                parent.mount != source.mount && source.mount.starts_with(&parent.mount)
            })
            .max_by_key(|parent| parent.mount.components().count())
            .context("Missing parent mount for a backed-up subvolume")?;
        let expected = Path::new(parent.subvol.trim_start_matches('/'))
            .join(source.mount.strip_prefix(&parent.mount)?);
        ensure!(
            expected == Path::new(source.subvol.trim_start_matches('/')),
            "The destination fstab does not mount backed-up subvolume {} at {}",
            source.subvol,
            source.mount.display()
        );
    }
    fstab.push_str(&format!(
        "\n\n# Snap Back backup drive\nUUID={} {} btrfs nofail,noatime,compress=zstd:3,x-systemd.device-timeout=5s 0 0\n",
        backup.uuid, backup.mount.display()
    ));
    Ok(fstab)
}

fn local_boot_file(boot: &Path, value: &str) -> Result<()> {
    let relative = value
        .strip_prefix("boot():/")
        .context("Expected a boot-relative Limine image path")?;
    let path = Path::new(relative.split('#').next().unwrap_or_default());
    ensure!(
        !path.as_os_str().is_empty()
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "Unsafe Limine image path"
    );
    let file = boot.join(path);
    ensure!(
        file.canonicalize().is_ok_and(|canonical| canonical == file) && file.is_file(),
        "Missing backed-up boot image {value}"
    );
    Ok(())
}

/// Create a small default boot menu from the normal backed-up Linux entries.
/// Snapshot and secondary OS entries may refer to data absent on the new disk.
fn replacement_limine(boot: &Path, config: &str, old_uuid: &str, new_uuid: &str) -> Result<String> {
    let mut lines: Vec<Vec<&str>> = Vec::new();
    for line in config.lines().map(str::trim) {
        if line.starts_with('/') {
            lines.push(Vec::new());
        } else if let Some(entry) = lines.last_mut() {
            entry.push(line);
        }
    }
    let mut menu = String::from("timeout: 5\ndefault_entry: 1\nremember_last_entry: no\n");
    let mut count = 0;
    for entry in lines {
        let fields: Vec<_> = entry
            .iter()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.trim(), value.trim()))
            .collect();
        if !fields.contains(&("protocol", "linux")) {
            continue;
        }
        let Some((_, cmdline)) = fields.iter().find(|(key, _)| *key == "cmdline") else {
            continue;
        };
        let args: Vec<_> = cmdline.split_whitespace().collect();
        let roots: Vec<_> = args
            .iter()
            .filter_map(|arg| arg.strip_prefix("root="))
            .collect();
        if roots.len() != 1
            || !uuid_source(roots[0], old_uuid)
            || !args.iter().any(|arg| {
                arg.strip_prefix("rootflags=").is_some_and(|options| {
                    options
                        .split(',')
                        .any(|opt| matches!(opt, "subvol=/@" | "subvol=@"))
                })
            })
        {
            continue;
        }
        ensure!(
            !args.iter().any(|arg| arg.starts_with("rd.luks")
                || arg.starts_with("cryptdevice=")
                || arg.starts_with("rd.lvm")),
            "Encrypted and LVM boot configurations are unsupported for replacement"
        );
        let kernel = fields
            .iter()
            .find(|(key, _)| *key == "path")
            .context("Missing backed-up kernel path")?
            .1;
        let modules: Vec<_> = fields
            .iter()
            .filter_map(|(key, value)| (*key == "module_path").then_some(*value))
            .collect();
        ensure!(!modules.is_empty(), "Missing backed-up initramfs");
        local_boot_file(boot, kernel)?;
        for module in &modules {
            local_boot_file(boot, module)?;
        }
        count += 1;
        menu.push_str(&format!(
            "\n/Restored CachyOS ({count})\nprotocol: linux\npath: {kernel}\n"
        ));
        for module in modules {
            menu.push_str(&format!("module_path: {module}\n"));
        }
        let args = args
            .into_iter()
            .filter(|arg| {
                !arg.starts_with("root=")
                    && !arg.starts_with("resume=")
                    && !arg.starts_with("resume_offset=")
            })
            .collect::<Vec<_>>()
            .join(" ");
        menu.push_str(&format!("cmdline: {args} root=UUID={new_uuid}\n"));
    }
    ensure!(
        count > 0,
        "No normal CachyOS Limine boot entry for the backed-up root subvolume @"
    );
    Ok(menu)
}

pub fn prepare(backup_mount: &Path, point_id: &str, confirm: &str) -> Result<String> {
    ensure!(
        confirm == "REPLACE",
        "Type REPLACE to prepare replacement of this installation"
    );
    let _lock = Operation::start()?;
    valid_id(point_id)?;
    let (destination, backup_uuid) = backup_drive(backup_mount)?;
    let point = points_on(&destination)?
        .into_iter()
        .find(|point| point.id == point_id)
        .context("Restore point not found on the backup drive")?;
    ensure!(point.boot, "The restore point has no boot files");
    let point_dir = destination.join("points").join(point_id);
    ensure!(
        point_dir.canonicalize()? == point_dir,
        "Unexpected path to the restore point"
    );
    let config_path = point_dir.join("root/snapshot/etc/snap-back/config.json");
    ensure!(
        config_path.canonicalize()? == config_path && config_path.is_file(),
        "The restore point has no usable Snap Back configuration"
    );
    let mut restored: Config = serde_json::from_slice(&fs::read(config_path)?)?;
    let old_uuid = restored.source_uuid.clone();
    valid_id(&old_uuid)?;
    restored
        .sources
        .retain(|source| point.subvolumes.contains(&source.name));
    for (name, mount, subvol) in [("root", "/", "@"), ("home", "/home", "@home")] {
        ensure!(
            restored.sources.iter().any(|source| source.name == name
                && source.mount == Path::new(mount)
                && source.subvol.trim_start_matches('/') == subvol),
            "The restore point must contain {mount} as {subvol}"
        );
    }
    ensure!(
        restored.sources.len() == point.subvolumes.len(),
        "The restore point does not describe every backed-up subvolume"
    );
    let root = Path::new("/");
    let boot = Path::new("/boot");
    let system_uuid = mount_value(root, "UUID")?;
    let boot_uuid = mount_value(boot, "UUID")?;
    valid_id(&system_uuid)?;
    valid_id(&boot_uuid)?;
    ensure!(
        mount_value(root, "FSTYPE")? == "btrfs"
            && mount_value(root, "FSROOT")? == "/@"
            && mount_value(Path::new("/home"), "UUID")? == system_uuid
            && mount_value(Path::new("/home"), "FSROOT")? == "/@home"
            && mount_value(boot, "FSTYPE")? == "vfat"
            && boot.join("limine.conf").is_file()
            && boot.join("EFI").is_dir(),
        "Use a destination with Limine, a separate EFI partition at /boot, and Btrfs @ and @home"
    );
    ensure!(
        system_uuid != old_uuid,
        "Use ordinary system recovery for the original installation"
    );
    let device = mount_value(root, "SOURCE")?;
    let device = device
        .split('[')
        .next()
        .context("Missing destination system device")?;
    ensure!(
        matches!(
            command("lsblk", &["-ndo", "TYPE", device])?.as_str(),
            "disk" | "part"
        ),
        "Replacement requires plain Btrfs on the destination system disk"
    );
    let listing = command("btrfs", &["subvolume", "list", "-a", "/"])?;
    let subvolumes: std::collections::BTreeMap<_, _> = crate::swap::subvolumes(&listing)
        .map(|(id, path)| (path, id))
        .collect();
    for source in &restored.sources {
        let expected = subvolumes
            .get(source.subvol.trim_start_matches('/'))
            .with_context(|| format!("Missing destination subvolume {}", source.subvol))?;
        ensure!(
            command("findmnt", &["-n", "-T", ps(&source.mount)?, "-o", "UUID"])? == system_uuid
                && crate::swap::root_id(&source.mount)? == *expected,
            "Destination path {} does not open subvolume {}",
            source.mount.display(),
            source.subvol
        );
    }

    let backup = BackupTarget {
        mount: Path::new("/backup/snap-back-drives").join(&backup_uuid),
        uuid: backup_uuid,
        label: "Snap Back".into(),
    };
    let fstab = replacement_fstab(
        &fs::read_to_string("/etc/fstab")?,
        &system_uuid,
        &boot_uuid,
        &backup,
        &restored.sources,
    )?;
    let boot_source = point_dir.join("boot");
    let limine_file = boot_source.join("limine.conf");
    ensure!(
        limine_file.canonicalize()? == limine_file,
        "Unexpected Limine configuration path"
    );
    let limine = replacement_limine(
        &boot_source,
        &fs::read_to_string(&limine_file)?,
        &old_uuid,
        &system_uuid,
    )?;

    restored.source_uuid = system_uuid.clone();
    restored.backup_uuid = backup.uuid.clone();
    restored.destination = backup.mount.clone();
    restored.targets = vec![backup];
    restored.backup_targets = None;
    let sources: Vec<_> = restored
        .sources
        .iter()
        .map(|source| recovery::RecoverySource {
            name: source.name.clone(),
            subvol: source.subvol.clone(),
        })
        .collect();
    crate::write_progress("Staging replacement for this CachyOS installation");
    recovery::prepare(
        &destination,
        point_id,
        &system_uuid,
        &sources,
        Some(recovery::Replacement {
            fstab,
            limine,
            old_root_uuid: old_uuid,
            config: restored,
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_menu_uses_new_root_and_omits_snapshot_and_resume_entries() {
        let dir =
            std::env::temp_dir().join(format!("snap-back-limine-test-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let result = (|| -> Result<String> {
            let dir = dir.canonicalize()?;
            fs::write(dir.join("vmlinuz"), [])?;
            fs::write(dir.join("initramfs"), [])?;
            let config = "/CachyOS\nprotocol: linux\npath: boot():/vmlinuz\nmodule_path: boot():/initramfs\ncmdline: rw root=UUID=old rootflags=subvol=/@ resume=UUID=old-swap resume_offset=12\n\n/Snapshot\nprotocol: linux\npath: boot():/not-present\ncmdline: root=UUID=old rootflags=subvol=/@/.snapshots/105/snapshot\n";
            replacement_limine(&dir, config, "old", "new")
        })();
        fs::remove_dir_all(dir).unwrap();
        let menu = result.unwrap();
        assert!(menu.contains("root=UUID=new"));
        assert!(menu.contains("module_path: boot():/initramfs"));
        assert!(!menu.contains("root=UUID=old"));
        assert!(!menu.contains("resume"));
        assert!(!menu.contains("not-present"));
    }

    #[test]
    fn replacement_requires_the_backed_up_mount_layout() {
        let sources =
            [("root", "/", "@"), ("home", "/home", "@home")].map(|(name, mount, subvol)| Source {
                name: name.into(),
                mount: mount.into(),
                subvol: subvol.into(),
                snapper_config: None,
            });
        let backup = BackupTarget {
            uuid: "backup-disk".into(),
            label: "Snap Back".into(),
            mount: "/backup/snap-back-drives/backup-disk".into(),
        };
        let fstab = "UUID=new / btrfs subvol=@ 0 0\nUUID=new /home btrfs subvol=/@home 0 0\nUUID=boot /boot vfat defaults 0 2\n";
        let result = replacement_fstab(fstab, "new", "boot", &backup, &sources).unwrap();
        assert!(result.starts_with(fstab));
        assert!(result.contains("UUID=backup-disk /backup/snap-back-drives/backup-disk"));
        assert!(
            replacement_fstab(
                &fstab.replace("subvol=/@home", "subvol=/@data"),
                "new",
                "boot",
                &backup,
                &sources
            )
            .is_err()
        );
    }
}
