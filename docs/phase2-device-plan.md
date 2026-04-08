# Phase 2 — iridium-device implementation plan

## Scope

Implement `crates/iridium-device/`: block device enumeration, HPA/DCO detection,
read-only forensic access. Matches and extends Guymager's device layer.

## Design decisions

| Question | Decision |
|----------|----------|
| NVMe serial | sysfs-first; empty string if absent |
| Privileges | Root required — EPERM → `DeviceError::PermissionDenied` with hint |
| O_DIRECT alignment | Handled internally in `DeviceReader` (aligned buffer, copy out) |
| Loop devices | Included (`loop*`) |
| Partitions | Included (two-level sysfs walk) |

## Public API

```rust
pub struct Disk {
    pub path: PathBuf,
    pub model: String,
    pub serial: String,
    pub size_bytes: u64,
    pub sector_size: u32,
    pub hpa_size_bytes: Option<u64>,   // Some(native_bytes) if HPA present
    pub dco_restricted: bool,
    pub removable: bool,
    pub rotational: bool,              // false = SSD/NVMe
    pub read_only: bool,
    pub partition_of: Option<PathBuf>, // Some(parent) if this is a partition
}

#[derive(Debug, Error)]
pub enum DeviceError {
    Sysfs { path: PathBuf, source: io::Error },
    Ioctl { path: PathBuf, source: nix::Error },
    Open { path: PathBuf, source: nix::Error },
    Read { offset: u64, source: nix::Error },
    PermissionDenied,
}

impl Disk {
    pub fn enumerate() -> Result<Vec<Disk>, DeviceError>;
    pub fn open_read_only(&self) -> Result<DeviceReader, DeviceError>;
}

pub struct DeviceReader { /* fd, size_bytes, sector_size, aligned_buf */ }
impl DeviceReader {
    pub fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, DeviceError>;
    pub fn size_bytes(&self) -> u64;
    pub fn sector_size(&self) -> u32;
}
```

## Module layout

```
crates/iridium-device/src/
  lib.rs      — Disk, DeviceError, public re-exports
  sysfs.rs    — sysfs walk, attribute readers, partition enumeration
  ioctl.rs    — ATA IDENTIFY ioctl, HPA/DCO extraction
  reader.rs   — DeviceReader: O_DIRECT, aligned buffer, pread64
tests/
  enumerate.rs — integration: enumerate() smoke, loop device presence
```

## Step-by-step

### 1 — sysfs enumeration (`sysfs.rs`)

- Walk `/sys/block/`
- **Skip**: `dm-*` `md*` `zram*` `ram*`
- **Include**: `sd*` `nvme*` `hd*` `vd*` `loop*`
- Read per device: `device/model`, `device/serial`, `size` (×512), `queue/hw_sector_size`,
  `removable`, `queue/rotational`, `ro`
- Partitions: scan device dir for entries matching `{dev}[0-9]*` or `{dev}p[0-9]*`;
  inherit model/serial from parent, set `partition_of`

### 2 — HPA detection (`ioctl.rs`, SATA/SAS only)

- Open `O_RDONLY | O_NONBLOCK`
- `HDIO_DRIVE_CMD` ioctl → 512-byte ATA IDENTIFY data
- Compare words 60-61 (LBA28 visible) vs 100-103 (LBA48 native max)
- If native > visible → `hpa_size_bytes = Some(native * sector_size)`
- EPERM → `DeviceError::PermissionDenied`
- Any other failure (NVMe, not applicable) → `hpa_size_bytes = None`

### 3 — DCO detection (`ioctl.rs`)

- Same IDENTIFY data (reuse from HPA step)
- Word 86 bit 11 set → `dco_restricted = true`

### 4 — DeviceReader (`reader.rs`)

- Open: `O_RDONLY | O_DIRECT | O_NOATIME`; retry without `O_DIRECT` on `EINVAL`
- `read_at`: aligned buffer (sector-aligned, allocated at open), `pread64`, copy to caller
- Resize aligned buffer on demand if `buf.len()` exceeds current allocation

### 5 — Tests

- Unit: sysfs parse helpers with fixture strings (no real device needed)
- Unit: IDENTIFY byte-array parsing for HPA/DCO logic
- Integration `tests/enumerate.rs`: smoke (non-empty result, paths exist in /dev);
  loop device test (create via `losetup`, check presence, teardown)

### 6 — ADR

`docs/adr/0004-device-enumeration.md`: sysfs-only, no udev/libgudev dependency.
