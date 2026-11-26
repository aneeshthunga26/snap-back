//! Identify dedicated Btrfs swap subvolumes without excluding ordinary data.
use crate::{Source, command, ps};
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

pub(crate) fn subvolumes(list: &str) -> impl Iterator<Item = (u64, &str)> {
    list.lines().filter_map(|line| {
        let (fields, path) = line.split_once(" path ")?;
        let id = fields
            .strip_prefix("ID ")?
            .split_whitespace()
            .next()?
            .parse()
            .ok()?;
        Some((id, path.strip_prefix("<FS_TREE>/").unwrap_or(path)))
    })
}

pub(crate) fn root_id(path: &Path) -> Result<u64> {
    Ok(command("btrfs", &["inspect-internal", "rootid", ps(path)?])?.parse()?)
}

// Both Linux and fstab represent spaces and backslashes with octal escapes.
fn unescape(field: &str) -> PathBuf {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};
    let mut decoded = Vec::new();
    let mut rest = field.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        if byte == b'\\'
            && let [
                a @ b'0'..=b'3',
                b @ b'0'..=b'7',
                c @ b'0'..=b'7',
                remaining @ ..,
            ] = tail
        {
            decoded.push((a - b'0') * 64 + (b - b'0') * 8 + (c - b'0'));
            rest = remaining;
        } else {
            decoded.push(byte);
            rest = tail;
        }
    }
    PathBuf::from(OsString::from_vec(decoded))
}

pub(crate) fn dedicated(
    source_uuid: &str,
    list: &str,
    sources: &[Source],
) -> Result<BTreeSet<String>> {
    let listed: BTreeMap<_, _> = subvolumes(list).collect();
    let active = fs::read_to_string("/proc/swaps").context("Could not check active swap")?;
    let fstab = fs::read_to_string("/etc/fstab").context("Could not check configured swap")?;
    let candidates = active
        .lines()
        .skip(1)
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let path = fields.next()?;
            (fields.next()? == "file").then_some((path, true))
        })
        .chain(fstab.lines().filter_map(|line| {
            let mut fields = line.split_whitespace();
            let path = fields.next()?;
            fields.next()?;
            (fields.next()? == "swap" && !path.starts_with('#')).then_some((path, false))
        }));

    let mut volumes = BTreeMap::<u64, BTreeSet<PathBuf>>::new();
    for (path, active) in candidates {
        let path = unescape(path);
        if !path.is_absolute() {
            continue;
        }
        let Ok(path) = path.canonicalize() else {
            continue;
        };
        if !path.is_file()
            || command("findmnt", &["-n", "-T", ps(&path)?, "-o", "UUID"])? != source_uuid
        {
            continue;
        }
        if !active
            && !command("blkid", &["-p", "-s", "TYPE", "-o", "value", ps(&path)?])
                .is_ok_and(|kind| kind == "swap")
        {
            continue;
        }
        volumes.entry(root_id(&path)?).or_default().insert(path);
    }

    let mut dedicated = BTreeSet::new();
    for (id, files) in volumes {
        // A bind mount of a file or a directory inside the volume cannot prove
        // what else is stored there. Only inspect the actual subvolume root.
        let folder = files.first().and_then(|path| path.parent()).unwrap();
        let is_root = crate::is_subvolume(folder) && root_id(folder)? == id;
        if is_root {
            let entries = fs::read_dir(folder)?
                .map(|entry| Ok(entry?.path()))
                .collect::<std::io::Result<BTreeSet<_>>>()?;
            if entries == files
                && let Some(&subvol) = listed.get(&id)
            {
                dedicated.insert(subvol.to_owned());
                continue;
            }
        }
        let path = listed.get(&id).copied().unwrap_or("");
        ensure!(
            !sources
                .iter()
                .any(|source| source.subvol.trim_start_matches('/') == path
                    || (path.is_empty() && source.mount == Path::new("/"))),
            "Swap file {} shares a backup source with other files. Move swap into its own Btrfs subvolume containing only swap files.",
            files.first().unwrap().display()
        );
    }
    Ok(dedicated)
}
