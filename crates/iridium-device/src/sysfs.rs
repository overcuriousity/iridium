// sysfs.rs — walk /sys/block/ to enumerate block devices and their partitions.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{DeviceError, Disk, ioctl};

const SYSFS_BLOCK: &str = "/sys/block";

pub(crate) fn enumerate() -> Result<Vec<Disk>, DeviceError> {
    let mut disks = Vec::new();

    let rd = fs::read_dir(SYSFS_BLOCK).map_err(|e| DeviceError::Sysfs {
        path: PathBuf::from(SYSFS_BLOCK),
        source: e,
    })?;

    // Enumeration is best-effort: a device that disappears during the walk
    // (hot-unplug, transient ENOENT) is skipped rather than aborting the list.
    for entry in rd {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("iridium-device: skipping /sys/block entry: {e}");
                continue;
            }
        };

        let dev_name = entry.file_name();
        let dev_name = dev_name.to_string_lossy();

        let sysfs_dev = entry.path(); // e.g. /sys/block/sda
        let dev_path = PathBuf::from(format!("/dev/{dev_name}")); // e.g. /dev/sda

        let disk = match read_disk(&sysfs_dev, &dev_path, None) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("iridium-device: skipping {dev_path:?}: {e}");
                continue;
            }
        };
        let parent_path = disk.path.clone();
        disks.push(disk);

        let parts = match partitions_of(&sysfs_dev) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("iridium-device: skipping partitions of {dev_path:?}: {e}");
                vec![]
            }
        };
        for part in parts {
            let part_name = part
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let part_dev = PathBuf::from(format!("/dev/{part_name}"));
            match read_partition(&part, &part_dev, &sysfs_dev, &parent_path) {
                Ok(d) => disks.push(d),
                Err(e) => eprintln!("iridium-device: skipping partition {part_dev:?}: {e}"),
            }
        }
    }

    Ok(disks)
}

// ── Whole-disk ───────────────────────────────────────────────────────────────

fn read_disk(
    sysfs: &Path,
    dev_path: &Path,
    partition_of: Option<PathBuf>,
) -> Result<Disk, DeviceError> {
    let model = read_attr_optional(sysfs, "device/model");
    let serial = read_attr_optional(sysfs, "device/serial");
    let size_sectors = read_attr_u64(sysfs, "size")?;
    // logical_block_size drives LBA counts and O_DIRECT alignment requirements.
    // A value of 0 is invalid; clamp to the universal minimum of 512.
    let logical_sector_size = read_attr_u32(sysfs, "queue/logical_block_size")
        .unwrap_or(0)
        .max(512);
    let sector_size = read_attr_u32(sysfs, "queue/hw_sector_size")
        .unwrap_or(0)
        .max(512);
    let removable = read_attr_bool(sysfs, "removable");
    let rotational = read_attr_bool(sysfs, "queue/rotational");
    let read_only = read_attr_bool(sysfs, "ro");

    let size_bytes = size_sectors
        .checked_mul(512)
        .ok_or_else(|| DeviceError::Sysfs {
            path: sysfs.join("size"),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, "size overflow"),
        })?;

    let (hpa_size_bytes, dco_restricted) = ioctl::hpa_dco(dev_path, logical_sector_size);

    Ok(Disk {
        path: dev_path.to_path_buf(),
        model,
        serial,
        size_bytes,
        logical_sector_size,
        sector_size,
        hpa_size_bytes,
        dco_restricted,
        removable,
        rotational,
        read_only,
        partition_of,
    })
}

// ── Partition ────────────────────────────────────────────────────────────────

fn read_partition(
    sysfs_part: &Path,
    dev_path: &Path,
    sysfs_parent: &Path,
    parent_dev: &Path,
) -> Result<Disk, DeviceError> {
    // Partitions have their own `size` but no `device/model` or `device/serial`.
    let model = read_attr_optional(sysfs_parent, "device/model");
    let serial = read_attr_optional(sysfs_parent, "device/serial");
    let size_sectors = read_attr_u64(sysfs_part, "size")?;
    let logical_sector_size = read_attr_u32(sysfs_parent, "queue/logical_block_size")
        .unwrap_or(0)
        .max(512);
    let sector_size = read_attr_u32(sysfs_parent, "queue/hw_sector_size")
        .unwrap_or(0)
        .max(512);
    let rotational = read_attr_bool(sysfs_parent, "queue/rotational");
    let removable = read_attr_bool(sysfs_parent, "removable");
    let read_only = read_attr_bool(sysfs_part, "ro");

    let size_bytes = size_sectors
        .checked_mul(512)
        .ok_or_else(|| DeviceError::Sysfs {
            path: sysfs_part.join("size"),
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, "size overflow"),
        })?;

    Ok(Disk {
        path: dev_path.to_path_buf(),
        model,
        serial,
        size_bytes,
        logical_sector_size,
        sector_size,
        hpa_size_bytes: None, // HPA is a whole-disk concept
        dco_restricted: false,
        removable,
        rotational,
        read_only,
        partition_of: Some(parent_dev.to_path_buf()),
    })
}

// ── Partition discovery ───────────────────────────────────────────────────────

/// Return sysfs paths for all partition subdirs of a device directory.
///
/// Detection uses the `partition` sysfs attribute: the kernel exports
/// `{sysfs_dev}/{name}/partition` for every partition, regardless of device
/// naming convention. This is more reliable than name-based heuristics.
fn partitions_of(sysfs_dev: &Path) -> Result<Vec<PathBuf>, DeviceError> {
    let mut parts = Vec::new();

    let rd = fs::read_dir(sysfs_dev).map_err(|e| DeviceError::Sysfs {
        path: sysfs_dev.to_path_buf(),
        source: e,
    })?;

    for entry in rd {
        let entry = entry.map_err(|e| DeviceError::Sysfs {
            path: sysfs_dev.to_path_buf(),
            source: e,
        })?;
        let path = entry.path();
        if path.is_dir() && path.join("partition").exists() {
            parts.push(path);
        }
    }

    Ok(parts)
}

// ── sysfs attribute helpers ───────────────────────────────────────────────────

fn read_attr_raw(sysfs: &Path, attr: &str) -> Result<String, DeviceError> {
    let p = sysfs.join(attr);
    fs::read_to_string(&p)
        .map(|s| s.trim().to_owned())
        .map_err(|e| DeviceError::Sysfs { path: p, source: e })
}

fn read_attr_optional(sysfs: &Path, attr: &str) -> String {
    read_attr_raw(sysfs, attr).unwrap_or_default()
}

fn read_attr_u64(sysfs: &Path, attr: &str) -> Result<u64, DeviceError> {
    let s = read_attr_raw(sysfs, attr)?;
    s.parse::<u64>().map_err(|_| DeviceError::Sysfs {
        path: sysfs.join(attr),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("expected u64, got {:?}", s),
        ),
    })
}

fn read_attr_u32(sysfs: &Path, attr: &str) -> Result<u32, DeviceError> {
    let s = read_attr_raw(sysfs, attr)?;
    s.parse::<u32>().map_err(|_| DeviceError::Sysfs {
        path: sysfs.join(attr),
        source: std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("expected u32, got {:?}", s),
        ),
    })
}

fn read_attr_bool(sysfs: &Path, attr: &str) -> bool {
    read_attr_raw(sysfs, attr)
        .map(|s| s == "1")
        .unwrap_or(false)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn read_attr_optional_missing_returns_empty() {
        let p = Path::new("/nonexistent/sysfs/path");
        assert_eq!(read_attr_optional(p, "model"), "");
    }
}
