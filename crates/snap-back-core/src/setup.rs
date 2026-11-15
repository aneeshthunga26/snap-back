//! Configure an already formatted, explicitly selected Btrfs backup volume.
use crate::{CONFIG, Config, Source, command, root, valid_id};
use anyhow::{Context, Result, ensure};
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path};

pub fn configure(destination: &Path) -> Result<String> {
    root()?;
    ensure!(
        destination.is_absolute(),
        "Choose an absolute backup mount path"
    );
    ensure!(
        !Path::new(CONFIG).exists(),
        "Configuration already exists at {CONFIG}; edit it explicitly to change disks"
    );
    let dest = destination.canonicalize()?;
    let mount = dest.to_str().context("Invalid mount path")?;
    let backup_uuid = command("findmnt", &["-n", "-M", mount, "-o", "UUID"])?;
    ensure!(
        command("findmnt", &["-n", "-M", mount, "-o", "FSTYPE"])? == "btrfs",
        "Backup mount must be Btrfs"
    );
    let source_uuid = command("findmnt", &["-n", "-M", "/", "-o", "UUID"])?;
    ensure!(
        !backup_uuid.is_empty() && source_uuid != backup_uuid,
        "Backup must be on a separate Btrfs filesystem"
    );
    let config_json: Value =
        serde_json::from_str(&command("snapper", &["--jsonout", "list-configs"])?)?;
    let mut snapper = BTreeMap::new();
    if let Some(configs) = config_json.get("configs").and_then(Value::as_array) {
        for config in configs {
            if let (Some(name), Some(subvolume)) = (
                config.get("config").and_then(Value::as_str),
                config.get("subvolume").and_then(Value::as_str),
            ) {
                snapper.insert(subvolume.to_owned(), name.to_owned());
            }
        }
    }
    for (name, mount) in [("root", "/"), ("home", "/home")] {
        if !snapper.contains_key(mount) {
            command("snapper", &["-c", name, "create-config", mount])?;
            snapper.insert(mount.into(), name.into());
        }
    }
    let mounts: Value = serde_json::from_str(&command(
        "findmnt",
        &["--json", "-t", "btrfs", "-o", "TARGET,FSROOT,UUID"],
    )?)?;
    let mut source_mounts = BTreeMap::new();
    fn walk(values: &[Value], uuid: &str, map: &mut BTreeMap<String, String>) {
        for v in values {
            if v.get("uuid").and_then(Value::as_str) == Some(uuid)
                && let (Some(root), Some(target)) = (
                    v.get("fsroot").and_then(Value::as_str),
                    v.get("target").and_then(Value::as_str),
                )
            {
                map.insert(root.trim_start_matches('/').into(), target.into());
            }
            if let Some(children) = v.get("children").and_then(Value::as_array) {
                walk(children, uuid, map);
            }
        }
    }
    walk(
        mounts
            .get("filesystems")
            .and_then(Value::as_array)
            .context("No Btrfs mounts")?,
        &source_uuid,
        &mut source_mounts,
    );
    let root_subvol = command("findmnt", &["-n", "-M", "/", "-o", "FSROOT"])?;
    let root_subvol = root_subvol.trim_start_matches('/');
    for line in command("btrfs", &["subvolume", "list", "-a", "/"])?.lines() {
        let Some((_, p)) = line.split_once(" path ") else {
            continue;
        };
        let p = p.strip_prefix("<FS_TREE>/").unwrap_or(p);
        if p.split('/').any(|c| c == ".snapshots")
            || p.contains("/var/lib/snap-back/")
            || p.starts_with(".snap-back-")
        {
            continue;
        }
        if let Some(relative) = p.strip_prefix(&format!("{root_subvol}/")) {
            source_mounts
                .entry(p.into())
                .or_insert_with(|| format!("/{relative}"));
        }
    }
    let mut sources = Vec::new();
    for (subvol, mount) in source_mounts {
        let name = match mount.as_str() {
            "/" => "root".into(),
            "/home" => "home".into(),
            "/root" => "root-user".into(),
            _ => mount.trim_matches('/').replace('/', "-"),
        };
        valid_id(&name)?;
        let config = snapper.get(&mount).cloned();
        if let Some(c) = &config {
            command(
                "snapper",
                &[
                    "-c",
                    c,
                    "set-config",
                    "TIMELINE_LIMIT_HOURLY=24",
                    "TIMELINE_LIMIT_DAILY=7",
                    "TIMELINE_LIMIT_WEEKLY=4",
                ],
            )?;
        }
        sources.push(Source {
            name,
            mount: mount.into(),
            subvol,
            snapper_config: config,
        });
    }
    ensure!(
        sources.iter().any(|s| s.name == "root") && sources.iter().any(|s| s.name == "home"),
        "Separate root and home subvolumes are required"
    );
    let config = Config {
        destination: dest,
        backup_uuid,
        source_uuid,
        sources,
        targets: Vec::new(),
        backup_targets: None,
    };
    fs::create_dir_all("/etc/snap-back")?;
    fs::write(CONFIG, serde_json::to_vec_pretty(&config)?)?;
    Ok(format!(
        "Configured {} subvolumes for backup to {}",
        config.sources.len(),
        config.destination.display()
    ))
}
