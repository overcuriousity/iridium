# ADR 0004 — Device Enumeration: sysfs-only, no udev

**Status:** Accepted

## Context

iridium must enumerate block devices available for imaging: physical disks (SATA,
NVMe, SAS), loop devices, and their partitions. Options considered:

- **udev / libgudev** — the approach used by Guymager; requires a running udev
  daemon and adds a C library dependency.
- **sysfs-only** — walk `/sys/block/` and read kernel-exported attributes directly.

## Decision

Enumerate devices via `/sys/block/` only. No udev, no libgudev.

- Top-level entries in `/sys/block/` represent whole-disk devices.
- Partition subdirectories of `/sys/block/{dev}/` represent partitions; they are
  identified by the presence of a `partition` sysfs attribute, not by name
  pattern (which varies: `sda1`, `nvme0n1p1`, `mmcblk0p1`, etc.).
- Attributes (model, serial, size, sector size, rotational, removable, ro) are
  read from the sysfs attribute files exposed by the kernel.

All entries under `/sys/block/` are surfaced without filtering — physical disks,
NVMe, optical drives, SD/eMMC, loop devices, software RAID arrays (`md*`),
device-mapper volumes (`dm-*`), and RAM-backed devices (`ram*`, `zram*`) are all
valid imaging targets depending on the investigation. Device-class filtering is
a display/UI concern and must not be baked into the enumeration layer.

## Consequences

- No runtime dependency on udev or D-Bus; works inside containers, chroots,
  and minimal airgap systems (consistent with the static musl binary goal).
- sysfs is a stable kernel ABI; attribute paths used here have been stable
  since Linux 2.6.
- Attributes are read at enumeration time (snapshot), not live. A second call
  to `Disk::enumerate()` is needed to detect hotplug events.
- udev symlinks (e.g. `/dev/disk/by-id/`) are not used. Callers see canonical
  `/dev/{name}` paths only.

## HPA / DCO detection note

HPA is detected via ATA IDENTIFY DEVICE SET MAX feature bits (words 82/85).
The current `hpa_size_bytes` field reports the already-restricted visible size,
not the true native capacity. Retrieving the native max requires a separate
`READ NATIVE MAX ADDRESS EXT` ioctl (`HDIO_DRIVE_TASKFILE`), deferred to Phase 8.
