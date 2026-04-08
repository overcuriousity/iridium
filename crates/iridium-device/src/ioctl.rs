// ioctl.rs — ATA IDENTIFY DEVICE ioctl for HPA and DCO detection.
//
// Only applicable to SATA/PATA/SAS drives. NVMe devices do not support
// HDIO_DRIVE_CMD; we detect this by path name and skip gracefully.
//
// Reference: ATA/ATAPI Command Set (ACS-3), clause 7.12 (IDENTIFY DEVICE).

use std::path::Path;

use nix::errno::Errno;

// HDIO_DRIVE_CMD ioctl number (Linux asm-generic/hdregs.h)
const HDIO_DRIVE_CMD: u64 = 0x031f;

// ATA command: IDENTIFY DEVICE
const WIN_IDENTIFY: u8 = 0xEC;

// ATA word offsets (each word = 2 bytes, little-endian)
// Words 60-61: 28-bit LBA addressable sector count (user-visible; may be HPA-limited)
const WORD_LBA28_LO: usize = 60;
// Words 100-103: 48-bit user-addressable sector count (user-visible; may be HPA-limited)
const WORD_LBA48_0: usize = 100;
const WORD_LBA48_1: usize = 101;
const WORD_LBA48_2: usize = 102;
const WORD_LBA48_3: usize = 103;
// Word 82 bit 8: SET MAX (HPA) feature set supported
const WORD_CMD_SET_SUPPORTED: usize = 82;
// Word 85 bit 8: SET MAX (HPA) feature set enabled in current command set
const WORD_CMD_SET_ENABLED: usize = 85;
const HPA_BIT: u16 = 1 << 8;
// Word 86 bit 11: SET MAX EXT / DCO active in command set active
const WORD_CMD_SET_ACTIVE: usize = 86;
const DCO_BIT: u16 = 1 << 11;

nix::ioctl_read_bad!(hdio_drive_cmd, HDIO_DRIVE_CMD, [u8; 4 + 512]);

/// Issue ATA IDENTIFY DEVICE and return the 512-byte IDENTIFY data.
///
/// Returns `Ok(None)` for NVMe and loop devices (HDIO_DRIVE_CMD not applicable).
/// Returns `Err(EPERM)` / `Err(EACCES)` when the process lacks privilege —
/// the caller (`hpa_dco`) matches on those and logs a warning.
/// Returns `Err(_)` for any other ioctl failure (ENOTTY, EINVAL, etc.).
fn ata_identify(dev_path: &Path) -> Result<Option<[u16; 256]>, nix::Error> {
    // NVMe devices do not support HDIO_DRIVE_CMD.
    let name = dev_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name.starts_with("nvme") || name.starts_with("loop") {
        return Ok(None);
    }

    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(dev_path)
        .map_err(|e| {
            e.raw_os_error()
                .map(|errno| nix::Error::from(Errno::from_raw(errno)))
                .unwrap_or(nix::Error::ENODEV)
        })?;

    use std::os::unix::io::AsRawFd;
    let fd = file.as_raw_fd();

    // HDIO_DRIVE_CMD buffer layout:
    //   byte 0: command (WIN_IDENTIFY = 0xEC)
    //   byte 1: sector count (1 for IDENTIFY)
    //   byte 2: feature register (0 for IDENTIFY)
    //   byte 3: sector count (unused)
    //   bytes 4..516: 512-byte response buffer
    let mut buf = [0u8; 4 + 512];
    buf[0] = WIN_IDENTIFY;
    buf[1] = 1; // 1 sector

    // SAFETY: buf is correctly sized for the ioctl, fd is valid.
    unsafe { hdio_drive_cmd(fd, &mut buf) }?;

    // Convert bytes 4..516 to little-endian u16 words.
    let mut words = [0u16; 256];
    for (i, w) in words.iter_mut().enumerate() {
        let base = 4 + i * 2;
        *w = u16::from_le_bytes([buf[base], buf[base + 1]]);
    }

    Ok(Some(words))
}

/// Return `(hpa_size_bytes, dco_restricted)` for a device.
///
/// For NVMe / loop devices or on permission errors both values are
/// `(None, false)` — the caller is not expected to surface this as an error
/// during enumeration; it is logged implicitly by returning defaults.
pub(crate) fn hpa_dco(dev_path: &Path, logical_sector_size: u32) -> (Option<u64>, bool) {
    match ata_identify(dev_path) {
        Ok(None) => (None, false), // NVMe / loop — not applicable
        Err(nix::Error::EPERM) | Err(nix::Error::EACCES) => {
            eprintln!(
                "iridium-device: HPA/DCO detection skipped for {:?} \
                 (insufficient privileges — run as root for full detection)",
                dev_path
            );
            (None, false)
        }
        Err(_) => (None, false), // ENOTTY, EINVAL, etc. — not an ATA device
        Ok(Some(words)) => parse_hpa_dco(&words, logical_sector_size),
    }
}

fn parse_hpa_dco(words: &[u16; 256], logical_sector_size: u32) -> (Option<u64>, bool) {
    // HPA detection via SET MAX feature status bits.
    //
    // Both the LBA28 (words 60-61) and LBA48 (words 100-103) counts in IDENTIFY
    // can themselves be restricted by an active HPA — they cannot be compared
    // against each other to detect HPA. The authoritative indicator is the SET MAX
    // feature status in the command-set words:
    //   word 82 bit 8 = SET MAX supported
    //   word 85 bit 8 = SET MAX enabled (HPA is restricting capacity)
    //
    // When HPA is active we report the current (already-restricted) LBA48 visible
    // size. The actual native capacity requires a separate READ NATIVE MAX ADDRESS
    // EXT ioctl (HDIO_DRIVE_TASKFILE), deferred to Phase 8.
    let hpa_active = (words[WORD_CMD_SET_SUPPORTED] & HPA_BIT) != 0
        && (words[WORD_CMD_SET_ENABLED] & HPA_BIT) != 0;

    let lba48 = (words[WORD_LBA48_0] as u64)
        | ((words[WORD_LBA48_1] as u64) << 16)
        | ((words[WORD_LBA48_2] as u64) << 32)
        | ((words[WORD_LBA48_3] as u64) << 48);

    let lba28 = (words[WORD_LBA28_LO] as u32) | ((words[WORD_LBA28_LO + 1] as u32) << 16);

    let visible_sectors = if lba48 > 0 { lba48 } else { lba28 as u64 };

    // Some(visible_bytes) when HPA is active — note: this is the restricted visible
    // size, not the native capacity. The native max requires Phase 8 implementation.
    // Multiply by logical sector size: IDENTIFY sector counts are in logical sectors.
    // Use checked_mul: malformed IDENTIFY data could otherwise overflow u64.
    let hpa_size_bytes = if hpa_active && visible_sectors > 0 {
        visible_sectors.checked_mul(logical_sector_size as u64)
    } else {
        None
    };

    // DCO: command set active word 86, bit 11.
    let dco_restricted = (words[WORD_CMD_SET_ACTIVE] & DCO_BIT) != 0;

    (hpa_size_bytes, dco_restricted)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal IDENTIFY buffer with the given LBA48 sector count,
    /// SET MAX feature bits, and optional DCO bit.
    fn make_identify(lba48_sectors: u64, hpa_active: bool, dco_bit: bool) -> [u16; 256] {
        let mut w = [0u16; 256];
        w[WORD_LBA48_0] = (lba48_sectors & 0xFFFF) as u16;
        w[WORD_LBA48_1] = ((lba48_sectors >> 16) & 0xFFFF) as u16;
        w[WORD_LBA48_2] = ((lba48_sectors >> 32) & 0xFFFF) as u16;
        w[WORD_LBA48_3] = ((lba48_sectors >> 48) & 0xFFFF) as u16;
        if hpa_active {
            // Both supported and enabled bits must be set.
            w[WORD_CMD_SET_SUPPORTED] |= HPA_BIT;
            w[WORD_CMD_SET_ENABLED] |= HPA_BIT;
        }
        if dco_bit {
            w[WORD_CMD_SET_ACTIVE] |= DCO_BIT;
        }
        w
    }

    #[test]
    fn no_hpa_when_set_max_not_enabled() {
        // SET MAX bits not set → HPA not active, even with a non-zero LBA count.
        let w = make_identify(1_000_000, false, false);
        let (hpa, dco) = parse_hpa_dco(&w, 512);
        assert_eq!(hpa, None);
        assert!(!dco);
    }

    #[test]
    fn hpa_detected_via_set_max_bits() {
        // SET MAX supported + enabled → HPA is active; visible size is reported.
        let visible_sectors: u64 = 900_000;
        let w = make_identify(visible_sectors, true, false);
        let (hpa, dco) = parse_hpa_dco(&w, 512);
        assert_eq!(hpa, Some(visible_sectors * 512));
        assert!(!dco);
    }

    #[test]
    fn dco_flag_detected() {
        let w = make_identify(1_000_000, false, true);
        let (_, dco) = parse_hpa_dco(&w, 512);
        assert!(dco);
    }

    #[test]
    fn hpa_and_dco_both_active() {
        let sectors: u64 = 500_000;
        let w = make_identify(sectors, true, true);
        let (hpa, dco) = parse_hpa_dco(&w, 4096);
        assert_eq!(hpa, Some(sectors * 4096));
        assert!(dco);
    }
}
