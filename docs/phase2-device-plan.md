# Phase 2 — iridium-device implementation plan

## Scope

Implement `crates/iridium-device/`: block device enumeration, HPA/DCO detection,
read-only forensic access. Matches and extends Guymager's device layer.

## Design decisions

| Question | Decision |
|----------|----------|
| NVMe serial | sysfs-first; empty string if absent |
| Privileges | Root recommended; EPERM on HPA/DCO ioctl → log warning, return `(None, false)` (enumeration continues) |
| O_DIRECT alignment | Handled internally in `DeviceReader` (offset rounded down + aligned buffer + copy out) |
| Loop devices | Included (`loop*`) |
| Partitions | Included (two-level sysfs walk) |

## Public API

```rust
pub struct Disk {
    pub path: PathBuf,
    pub model: String,
    pub serial: String,
    pub size_bytes: u64,              // size_sectors × 512 (sysfs always 512-byte units)
    pub logical_sector_size: u32,     // queue/logical_block_size — for LBA math & O_DIRECT
    pub sector_size: u32,             // queue/hw_sector_size — physical sector size
    pub hpa_size_bytes: Option<u64>,  // Some(visible_bytes) if SET MAX active (NOT native max)
    pub dco_restricted: bool,
    pub removable: bool,
    pub rotational: bool,              // false = SSD/NVMe
    pub read_only: bool,
    pub partition_of: Option<PathBuf>, // Some(parent) if this is a partition
}

#[derive(Debug, Error)]
pub enum DeviceError {
    Sysfs { path: PathBuf, source: io::Error },
    Open { path: PathBuf, source: nix::Error },
    Read { offset: u64, source: nix::Error },
}

impl Disk {
    pub fn enumerate() -> Result<Vec<Disk>, DeviceError>;
    pub fn open_read_only(&self) -> Result<DeviceReader, DeviceError>;
}

pub struct DeviceReader { /* fd, size_bytes, logical_sector_size, aligned_buf */ }
impl DeviceReader {
    pub fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<usize, DeviceError>;
    pub fn size_bytes(&self) -> u64;
    pub fn logical_sector_size(&self) -> u32;
}
```

## Module layout

```
crates/iridium-device/src/
  lib.rs      — Disk, DeviceError, public re-exports; #[cfg(test)] enumerate smoke tests
  sysfs.rs    — sysfs walk, attribute readers, partition enumeration
  ioctl.rs    — ATA IDENTIFY ioctl, HPA/DCO extraction
  reader.rs   — DeviceReader: O_DIRECT, aligned buffer, pread64; #[cfg(test)] unit tests
```

## Step-by-step

### 1 — sysfs enumeration (`sysfs.rs`)

- Walk `/sys/block/` and surface **every** entry — no device class is excluded
  at the enumeration layer; callers filter for display purposes
- Read per device: `device/model`, `device/serial`, `size` (×512), `queue/hw_sector_size`,
  `removable`, `queue/rotational`, `ro`
- Partitions: a subdir of the device dir is a partition if it contains a `partition`
  sysfs attribute (kernel exports this for every partition, unconditionally);
  inherit model/serial from parent, set `partition_of`

### 2 — HPA detection (`ioctl.rs`, SATA/SAS only)

- Open `O_RDONLY | O_NONBLOCK`
- `HDIO_DRIVE_CMD` ioctl → 512-byte ATA IDENTIFY data
- HPA detection via SET MAX feature status bits (ACS-3 §7.12):
  - word 82 bit 8 = SET MAX supported
  - word 85 bit 8 = SET MAX enabled (HPA is restricting capacity)
  - Both bits must be set to flag HPA active
- When HPA is active: `hpa_size_bytes = Some(visible_sectors * logical_sector_size)`
  - `visible_sectors` is taken from words 100-103 (LBA48) or 60-61 (LBA28 fallback)
  - **Limitation**: this is the *restricted visible* size, not the native max.
    Retrieving native capacity (READ NATIVE MAX ADDRESS EXT) is deferred to Phase 8.
- EPERM/EACCES → log warning, return `(None, false)` (enumeration continues)
- Any other failure (NVMe, ENOTTY, EINVAL, not applicable) → `hpa_size_bytes = None`

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
- Unit: `DeviceReader` O_DIRECT alignment math (offset/prefix/aligned_len)
- `enumerate_does_not_panic` — smoke test, runs in CI (no /dev required)
- `enumerate_paths_exist` — marked `#[ignore]`; requires /dev nodes in the
  test environment; run explicitly with `cargo test -- --ignored`

### 6 — ADR

`docs/adr/0004-device-enumeration.md`: sysfs-only, no udev/libgudev dependency.
