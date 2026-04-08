// Integration test: write a 1 MiB EWF image then read it back and verify.
//
// This test requires libewf to be installed / linked.  It is not gated by a
// feature flag — it will simply fail at compile time / link time if libewf is
// unavailable, which is the intended signal in CI.

use iridium_ewf::EwfHandle;
use iridium_ewf_sys::{LIBEWF_FORMAT_ENCASE6, LIBEWF_MEDIA_FLAG_PHYSICAL, LIBEWF_MEDIA_TYPE_FIXED};
use tempfile::TempDir;

const MIB: usize = 1024 * 1024;

fn make_test_data(size: usize) -> Vec<u8> {
    // Repeating pattern: easy to verify without storing a reference copy.
    (0..size).map(|i| (i % 251) as u8).collect()
}

#[test]
fn ewf_write_read_roundtrip() {
    let dir = TempDir::new().expect("tempdir");
    let base = dir.path().join("test_image");

    let data = make_test_data(MIB);

    // ── Write ──────────────────────────────────────────────────────────────
    {
        let mut h = EwfHandle::new().expect("EwfHandle::new");

        // All metadata setters must be called after open_write on the
        // system-installed libewf; media_size must also come after.
        h.open_write(&base).expect("open_write");
        h.set_media_type(LIBEWF_MEDIA_TYPE_FIXED)
            .expect("set_media_type");
        h.set_media_flags(LIBEWF_MEDIA_FLAG_PHYSICAL)
            .expect("set_media_flags");
        h.set_format(LIBEWF_FORMAT_ENCASE6).expect("set_format");
        h.set_bytes_per_sector(512).expect("set_bytes_per_sector");
        h.set_media_size(MIB as u64).expect("set_media_size");

        // Chain-of-custody metadata
        h.set_header_value(b"case_number", b"IRIDIUM-TEST-001")
            .expect("set case_number");
        h.set_header_value(b"examiner_name", b"iridium integration test")
            .expect("set examiner_name");
        h.set_header_value(b"description", b"1 MiB round-trip test image")
            .expect("set description");

        // Write in 64 KiB chunks
        let chunk = 64 * 1024;
        for offset in (0..MIB).step_by(chunk) {
            let end = (offset + chunk).min(MIB);
            let written = h.write_buffer(&data[offset..end]).expect("write_buffer");
            assert_eq!(written, end - offset, "short write at offset {offset}");
        }

        h.write_finalize().expect("write_finalize");
        h.close().expect("close");
    }

    // Verify the segment file was created.
    let segment = base.with_extension("e01");
    assert!(
        segment.exists(),
        "segment file {segment:?} not found after write"
    );

    // ── Read back ─────────────────────────────────────────────────────────
    {
        let mut h = EwfHandle::new().expect("EwfHandle::new (read)");
        h.open_read(&[segment.as_path()]).expect("open_read");

        let size = h.media_size().expect("media_size");
        assert_eq!(size, MIB as u64, "media_size mismatch");

        let mut readback = vec![0u8; MIB];
        let mut offset = 0usize;
        while offset < MIB {
            let n = h.read_buffer(&mut readback[offset..]).expect("read_buffer");
            assert_ne!(n, 0, "unexpected EOF at offset {offset}");
            offset += n;
        }

        assert_eq!(readback, data, "read-back data does not match written data");

        h.close().expect("close (read)");
    }
}

#[test]
fn ewf_write_with_md5_hash() {
    use md5::{Digest as _, Md5};

    let dir = TempDir::new().expect("tempdir");
    let base = dir.path().join("hashed_image");
    let data = make_test_data(MIB);

    // Compute MD5 of the test data.
    let mut hasher = Md5::new();
    hasher.update(&data);
    let digest: [u8; 16] = hasher.finalize().into();

    {
        let mut h = EwfHandle::new().expect("EwfHandle::new");
        h.open_write(&base).expect("open_write");
        h.set_format(LIBEWF_FORMAT_ENCASE6).expect("set_format");
        h.set_media_size(MIB as u64).expect("set_media_size");

        for chunk in data.chunks(64 * 1024) {
            h.write_buffer(chunk).expect("write_buffer");
        }
        h.set_md5_hash(&digest).expect("set_md5_hash");
        h.write_finalize().expect("write_finalize");
        h.close().expect("close");
    }

    // Read back and verify the stored hash matches.
    {
        let mut h = EwfHandle::new().expect("EwfHandle::new (read)");
        let seg = base.with_extension("e01");
        h.open_read(&[seg.as_path()]).expect("open_read");

        let stored = h
            .md5_hash()
            .expect("md5_hash getter")
            .expect("hash not set");
        assert_eq!(stored, digest, "stored MD5 does not match computed MD5");
        h.close().expect("close");
    }
}
