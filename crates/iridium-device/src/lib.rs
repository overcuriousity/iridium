// iridium-device: block device enumeration, HPA/DCO detection, read-only access.

mod ioctl;
mod reader;
mod sysfs;

pub use reader::DeviceReader;

use std::{io, path::PathBuf};
use thiserror::Error;

// ── Error type ────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum DeviceError {
    #[error("sysfs read error for {path}: {source}")]
    Sysfs { path: PathBuf, source: io::Error },

    #[error("open failed on {path}: {source}")]
    Open { path: PathBuf, source: nix::Error },

    #[error("read failed at offset {offset}: {source}")]
    Read { offset: u64, source: nix::Error },
}

// ── Disk ──────────────────────────────────────────────────────────────────────

/// A block device (whole disk or partition) available for imaging.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Disk {
    /// Path to the device node, e.g. `/dev/sda` or `/dev/nvme0n1`.
    pub path: PathBuf,
    /// Human-readable model string from the firmware, trimmed of whitespace.
    pub model: String,
    /// Serial number string from the firmware, trimmed of whitespace.
    pub serial: String,
    /// Total visible size in bytes; computed as `size_sectors × 512` because
    /// `/sys/block/*/size` always reports 512-byte units regardless of physical sector size.
    pub size_bytes: u64,
    /// Logical (software-visible) sector size in bytes, from `queue/logical_block_size`.
    /// Used for O_DIRECT alignment and LBA-count multiplications.
    pub logical_sector_size: u32,
    /// Physical (hardware) sector size in bytes, from `queue/hw_sector_size`.
    pub sector_size: u32,
    /// Visible (possibly HPA-restricted) size in bytes when the SET MAX feature set
    /// is enabled, `None` for NVMe/loop devices or when the ioctl is unavailable.
    ///
    /// **Note:** this is the *current restricted* capacity reported by IDENTIFY, NOT
    /// the native maximum. Retrieving the native max (via READ NATIVE MAX ADDRESS EXT)
    /// is deferred to Phase 8.
    pub hpa_size_bytes: Option<u64>,
    /// `true` if the DCO (Device Configuration Overlay) feature set is active,
    /// meaning the firmware has restricted the reported capacity or features.
    pub dco_restricted: bool,
    /// `true` if the device is flagged removable by the kernel.
    pub removable: bool,
    /// `true` for spinning-platter drives; `false` for SSDs and NVMe.
    pub rotational: bool,
    /// `true` if the kernel has opened the device read-only (e.g. write-blocker).
    pub read_only: bool,
    /// For partitions: the path of the parent whole-disk device.
    /// `None` for whole-disk devices.
    pub partition_of: Option<PathBuf>,
}

impl Disk {
    /// Enumerate **all** block devices visible to the kernel via `/sys/block/`,
    /// including physical disks, NVMe, optical drives, SD/eMMC, loop devices,
    /// software RAID arrays, device-mapper volumes, and RAM-backed devices.
    ///
    /// No device class is excluded at this layer — callers are responsible for
    /// any display-time filtering.
    ///
    /// HPA and DCO detection require `CAP_SYS_RAWIO` (typically: root). If the
    /// process lacks the capability those fields are set to `None`/`false` so
    /// unprivileged callers can still enumerate devices for display purposes.
    pub fn enumerate() -> Result<Vec<Disk>, DeviceError> {
        sysfs::enumerate()
    }

    /// Open the device for forensic read-only access.
    ///
    /// Uses `O_RDONLY | O_DIRECT | O_NOATIME`. If the device does not support
    /// `O_DIRECT` the flag is dropped and a warning is printed; all other open
    /// failures are returned as errors.
    pub fn open_read_only(&self) -> Result<DeviceReader, DeviceError> {
        reader::open_read_only(self)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_does_not_panic() {
        // Must not panic regardless of privilege level.
        let _ = Disk::enumerate();
    }

    #[test]
    #[ignore = "requires /dev nodes present in the test environment"]
    fn enumerate_paths_exist() {
        let disks = match Disk::enumerate() {
            Ok(d) => d,
            Err(_) => return,
        };
        for disk in &disks {
            assert!(
                disk.path.exists(),
                "device path {:?} from enumerate() does not exist in /dev",
                disk.path
            );
        }
    }
}
