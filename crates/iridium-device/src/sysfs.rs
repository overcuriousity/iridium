// sysfs.rs — walk /sys/block/ to enumerate block devices and their partitions.

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{DeviceError, Disk, ioctl};

const SYSFS_BLOCK: &str = "/sys/block";

/// Prefixes that identify virtual / non-physical devices to skip.
const SKIP_PREFIXES: &[&str] = &["dm-", "md", "zram", "ram"];

pub(crate) fn enumerate() -> Result<Vec<Disk>, DeviceError> {
    let mut disks = Vec::new();

    let rd = fs::read_dir(SYSFS_BLOCK).map_err(|e| DeviceError::Sysfs {
        path: PathBuf::from(SYSFS_BLOCK),
        source: e,
    })?;

    for entry in rd {
        let entry = entry.map_err(|e| DeviceError::Sysfs {
            path: PathBuf::from(SYSFS_BLOCK),
            source: e,
        })?;

        let dev_name = entry.file_name();
        let dev_name = dev_name.to_string_lossy();

        if SKIP_PREFIXES.iter().any(|p| dev_name.starts_with(p)) {
            continue;
        }

        let sysfs_dev = entry.path(); // e.g. /sys/block/sda
        let dev_path = PathBuf::from(format!("/dev/{dev_name}")); // e.g. /dev/sda

        let disk = read_disk(&sysfs_dev, &dev_path, None)?;
        let parent_path = disk.path.clone();
        disks.push(disk);

        // Partitions live as subdirs of the device dir named {dev}[0-9]* or {dev}p[0-9]*.
        for part in partitions_of(&sysfs_dev, &dev_name)? {
            let raw_name = part.file_name();
            let part_name = raw_name
                .as_deref()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let part_dev = PathBuf::from(format!("/dev/{part_name}"));
            disks.push(read_partition(&part, &part_dev, &sysfs_dev, &parent_path)?);
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
    let sector_size = read_attr_u32(sysfs, "queue/hw_sector_size").unwrap_or(512);
    let removable = read_attr_bool(sysfs, "removable");
    let rotational = read_attr_bool(sysfs, "queue/rotational");
    let read_only = read_attr_bool(sysfs, "ro");

    let size_bytes = size_sectors * 512; // sysfs size is always in 512-byte units

    let (hpa_size_bytes, dco_restricted) = ioctl::hpa_dco(dev_path, sector_size);

    Ok(Disk {
        path: dev_path.to_path_buf(),
        model,
        serial,
        size_bytes,
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
    let sector_size = read_attr_u32(sysfs_parent, "queue/hw_sector_size").unwrap_or(512);
    let rotational = read_attr_bool(sysfs_parent, "queue/rotational");
    let removable = read_attr_bool(sysfs_parent, "removable");
    let read_only = read_attr_bool(sysfs_part, "ro");

    Ok(Disk {
        path: dev_path.to_path_buf(),
        model,
        serial,
        size_bytes: size_sectors * 512,
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
/// Partition dirs are named `{dev_name}[0-9]+` (SCSI/loop) or `{dev_name}p[0-9]+` (NVMe).
fn partitions_of(sysfs_dev: &Path, dev_name: &str) -> Result<Vec<PathBuf>, DeviceError> {
    let mut parts = Vec::new();

    let rd = match fs::read_dir(sysfs_dev) {
        Ok(r) => r,
        Err(e) => {
            return Err(DeviceError::Sysfs {
                path: sysfs_dev.to_path_buf(),
                source: e,
            });
        }
    };

    for entry in rd {
        let entry = entry.map_err(|e| DeviceError::Sysfs {
            path: sysfs_dev.to_path_buf(),
            source: e,
        })?;

        let name = entry.file_name();
        let name = name.to_string_lossy();

        if !name.starts_with(dev_name) {
            continue;
        }

        // The suffix after dev_name must be purely digits, or 'p' followed by digits.
        let suffix = &name[dev_name.len()..];
        let is_partition = suffix.chars().all(|c| c.is_ascii_digit())
            || (suffix.starts_with('p') && suffix[1..].chars().all(|c| c.is_ascii_digit()));

        if is_partition && !suffix.is_empty() && entry.path().is_dir() {
            parts.push(entry.path());
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
    fn skip_prefixes_filter_dm() {
        assert!(SKIP_PREFIXES.iter().any(|p| "dm-0".starts_with(p)));
        assert!(SKIP_PREFIXES.iter().any(|p| "md0".starts_with(p)));
        assert!(SKIP_PREFIXES.iter().any(|p| "zram0".starts_with(p)));
        assert!(!SKIP_PREFIXES.iter().any(|p| "sda".starts_with(p)));
        assert!(!SKIP_PREFIXES.iter().any(|p| "nvme0n1".starts_with(p)));
        assert!(!SKIP_PREFIXES.iter().any(|p| "loop0".starts_with(p)));
    }

    #[test]
    fn partition_suffix_detection() {
        // SCSI style: sda1, sda12
        let check = |dev: &str, name: &str| -> bool {
            if !name.starts_with(dev) {
                return false;
            }
            let suffix = &name[dev.len()..];
            !suffix.is_empty()
                && (suffix.chars().all(|c| c.is_ascii_digit())
                    || (suffix.starts_with('p') && suffix[1..].chars().all(|c| c.is_ascii_digit())))
        };
        assert!(check("sda", "sda1"));
        assert!(check("sda", "sda12"));
        assert!(!check("sda", "sda")); // whole disk
        assert!(check("nvme0n1", "nvme0n1p1"));
        assert!(check("nvme0n1", "nvme0n1p12"));
        assert!(!check("nvme0n1", "nvme0n1")); // whole disk
        assert!(!check("sda", "sdb1")); // different device
    }

    #[test]
    fn read_attr_optional_missing_returns_empty() {
        let p = Path::new("/nonexistent/sysfs/path");
        assert_eq!(read_attr_optional(p, "model"), "");
    }
}
