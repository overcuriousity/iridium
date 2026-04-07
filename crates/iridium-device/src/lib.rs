// iridium-device: disk enumeration, HPA/DCO detection, read-only access.
// Phase 2 will flesh out Disk::enumerate() and ioctl calls.

/// Opaque handle representing a block device available for imaging.
#[derive(Debug)]
pub struct Disk {
    pub path: std::path::PathBuf,
    pub model: String,
    pub serial: String,
    pub size_bytes: u64,
    pub sector_size: u32,
}

#[cfg(test)]
mod tests {
    #[test]
    fn smoke() {
        // placeholder — real tests added in Phase 2
        assert!(true);
    }
}
