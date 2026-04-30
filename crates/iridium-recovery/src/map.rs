// map.rs — GNU ddrescue-compatible mapfile state and atomic serialiser.

use std::{
    fmt, fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use time::OffsetDateTime;

// ── Status ────────────────────────────────────────────────────────────────────

/// Status of a region in the recovery map, using GNU ddrescue conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// `?` — not yet attempted.
    NonTried,
    /// `*` — attempted at chunk size; needs sector-level trimming.
    NonTrimmed,
    /// `/` — trimmed but not individually scraped.
    NonScraped,
    /// `-` — all retry attempts failed; zero-filled in the output image.
    BadSector,
    /// `+` — successfully read and written.
    Finished,
}

impl Status {
    pub fn as_char(self) -> char {
        match self {
            Status::NonTried => '?',
            Status::NonTrimmed => '*',
            Status::NonScraped => '/',
            Status::BadSector => '-',
            Status::Finished => '+',
        }
    }

    /// Human-readable name used in audit log `map_status` fields.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::NonTried => "?",
            Status::NonTrimmed => "*",
            Status::NonScraped => "/",
            Status::BadSector => "-",
            Status::Finished => "+",
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_char())
    }
}

// ── Region ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    pub pos: u64,
    pub size: u64,
    pub status: Status,
}

impl Region {
    #[inline]
    pub fn end(&self) -> u64 {
        self.pos + self.size
    }
}

// ── MapState ──────────────────────────────────────────────────────────────────

/// In-memory mapfile state.  All mutations go through [`mark`](MapState::mark)
/// which keeps the region list normalised (no overlaps, adjacent same-status
/// regions merged).
pub struct MapState {
    pub regions: Vec<Region>,
    pub total_bytes: u64,
    /// Position of the last attempted read; written to the mapfile header.
    pub current_pos: u64,
    pub current_status: Status,
    /// 1-based pass counter for the mapfile header.
    pub current_pass: u8,
    pub mapfile_path: PathBuf,
    start_time: OffsetDateTime,
    iridium_version: String,
    argv: Vec<String>,
    /// Cached totals updated by every `mark()` call so callers are O(1).
    finished_bytes_cache: u64,
    bad_bytes_cache: u64,
}

impl MapState {
    pub fn new(
        total_bytes: u64,
        mapfile_path: PathBuf,
        iridium_version: String,
        argv: Vec<String>,
    ) -> Self {
        Self {
            regions: vec![Region {
                pos: 0,
                size: total_bytes,
                status: Status::NonTried,
            }],
            total_bytes,
            current_pos: 0,
            current_status: Status::NonTried,
            current_pass: 1,
            mapfile_path,
            start_time: OffsetDateTime::now_utc(),
            iridium_version,
            argv,
            finished_bytes_cache: 0,
            bad_bytes_cache: 0,
        }
    }

    // ── Query helpers ─────────────────────────────────────────────────────────

    pub fn finished_bytes(&self) -> u64 {
        self.finished_bytes_cache
    }

    pub fn bad_bytes(&self) -> u64 {
        self.bad_bytes_cache
    }

    /// Returns `true` if any region has the given `status`.
    pub fn has_status(&self, s: Status) -> bool {
        self.regions.iter().any(|r| r.status == s)
    }

    /// Iterate regions that have the given `status`, in ascending offset order.
    pub fn regions_with_status(&self, s: Status) -> impl Iterator<Item = &Region> {
        self.regions.iter().filter(move |r| r.status == s)
    }

    // ── Mutation ──────────────────────────────────────────────────────────────

    /// Mark `[pos, pos+size)` with `status`, splitting and merging regions as
    /// needed.  `size == 0` is a no-op.
    pub fn mark(&mut self, pos: u64, size: u64, status: Status) {
        if size == 0 {
            return;
        }
        let end = pos + size;
        self.split_at(pos);
        self.split_at(end);
        // Regions are kept sorted by pos; binary-search to avoid an O(n) scan
        // on every mark call (which would be O(n²) over a full recovery run).
        let start_idx = self
            .regions
            .binary_search_by(|r| r.pos.cmp(&pos))
            .unwrap_or_else(|i| i);
        let end_idx = self
            .regions
            .binary_search_by(|r| r.pos.cmp(&end))
            .unwrap_or_else(|i| i);
        for r in &mut self.regions[start_idx..end_idx] {
            r.status = status;
        }
        self.merge_adjacent();
        self.refresh_counters();
    }

    // ── Mapfile I/O ───────────────────────────────────────────────────────────

    /// Write the mapfile atomically: serialise to `<path>.tmp`, fsync, rename,
    /// then fsync the parent directory to make the rename durable across a
    /// power loss.
    pub fn flush(&self) -> io::Result<()> {
        let tmp = tmp_path(&self.mapfile_path);
        {
            let mut f = fs::File::create(&tmp)?;
            write_mapfile(&mut f, self)?;
            f.flush()?;
            f.sync_data()?;
        }
        fs::rename(&tmp, &self.mapfile_path)?;
        // Syncing the directory entry makes the rename visible after a crash.
        if let Some(parent) = self.mapfile_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::File::open(parent)?.sync_all()?;
        }
        Ok(())
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    fn refresh_counters(&mut self) {
        self.finished_bytes_cache = self
            .regions
            .iter()
            .filter(|r| r.status == Status::Finished)
            .map(|r| r.size)
            .sum();
        self.bad_bytes_cache = self
            .regions
            .iter()
            .filter(|r| r.status == Status::BadSector)
            .map(|r| r.size)
            .sum();
    }

    fn split_at(&mut self, at: u64) {
        for i in 0..self.regions.len() {
            let r = &self.regions[i];
            if r.pos < at && r.end() > at {
                let status = r.status;
                let left = Region {
                    pos: r.pos,
                    size: at - r.pos,
                    status,
                };
                let right = Region {
                    pos: at,
                    size: r.end() - at,
                    status,
                };
                self.regions.splice(i..=i, [left, right]);
                return;
            }
        }
    }

    fn merge_adjacent(&mut self) {
        let mut i = 0;
        while i + 1 < self.regions.len() {
            if self.regions[i].status == self.regions[i + 1].status {
                self.regions[i].size += self.regions[i + 1].size;
                self.regions.remove(i + 1);
            } else {
                i += 1;
            }
        }
    }
}

// ── Serialiser ────────────────────────────────────────────────────────────────

fn tmp_path(mapfile: &Path) -> PathBuf {
    let mut s = mapfile.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

fn write_mapfile(w: &mut impl io::Write, m: &MapState) -> io::Result<()> {
    use time::format_description::well_known::Rfc3339;

    let format_ts = |ts: OffsetDateTime| -> io::Result<String> {
        ts.format(&Rfc3339)
            .map_err(|e| io::Error::other(e.to_string()))
    };

    writeln!(
        w,
        "# Mapfile. Created by iridium-recovery v{}",
        m.iridium_version
    )?;
    writeln!(w, "# Command line: {}", m.argv.join(" "))?;
    writeln!(w, "# Start time:   {}", format_ts(m.start_time)?)?;
    writeln!(
        w,
        "# Current time: {}",
        format_ts(OffsetDateTime::now_utc())?
    )?;
    writeln!(w, "# current_pos  current_status  current_pass")?;
    writeln!(
        w,
        "0x{:012x}  {}  {}",
        m.current_pos, m.current_status, m.current_pass
    )?;
    writeln!(w, "#      pos        size  status")?;
    for r in &m.regions {
        writeln!(w, "0x{:012x}  0x{:012x}  {}", r.pos, r.size, r.status)?;
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_map(total: u64) -> MapState {
        MapState::new(
            total,
            PathBuf::from("/tmp/test.map"),
            "0.1.0".into(),
            vec![],
        )
    }

    // ── mark / split / merge ─────────────────────────────────────────────────

    #[test]
    fn initial_state_is_single_non_tried_region() {
        let m = make_map(1024);
        assert_eq!(m.regions.len(), 1);
        assert_eq!(
            m.regions[0],
            Region {
                pos: 0,
                size: 1024,
                status: Status::NonTried
            }
        );
    }

    #[test]
    fn mark_full_range_replaces_single_region() {
        let mut m = make_map(512);
        m.mark(0, 512, Status::Finished);
        assert_eq!(
            m.regions,
            vec![Region {
                pos: 0,
                size: 512,
                status: Status::Finished
            }]
        );
    }

    #[test]
    fn mark_prefix_splits_region() {
        let mut m = make_map(1024);
        m.mark(0, 512, Status::Finished);
        assert_eq!(m.regions.len(), 2);
        assert_eq!(
            m.regions[0],
            Region {
                pos: 0,
                size: 512,
                status: Status::Finished
            }
        );
        assert_eq!(
            m.regions[1],
            Region {
                pos: 512,
                size: 512,
                status: Status::NonTried
            }
        );
    }

    #[test]
    fn mark_middle_creates_three_regions() {
        let mut m = make_map(1024);
        m.mark(256, 512, Status::NonTrimmed);
        assert_eq!(m.regions.len(), 3);
        assert_eq!(m.regions[0].status, Status::NonTried);
        assert_eq!(m.regions[1].status, Status::NonTrimmed);
        assert_eq!(m.regions[2].status, Status::NonTried);
    }

    #[test]
    fn mark_adjacent_same_status_merges() {
        let mut m = make_map(1024);
        m.mark(0, 512, Status::Finished);
        m.mark(512, 512, Status::Finished);
        assert_eq!(
            m.regions,
            vec![Region {
                pos: 0,
                size: 1024,
                status: Status::Finished
            }]
        );
    }

    #[test]
    fn mark_zero_size_is_noop() {
        let mut m = make_map(512);
        m.mark(0, 0, Status::Finished);
        assert_eq!(m.regions.len(), 1);
        assert_eq!(m.regions[0].status, Status::NonTried);
    }

    #[test]
    fn finished_bytes_and_bad_bytes_are_accurate() {
        let mut m = make_map(1024);
        m.mark(0, 256, Status::Finished);
        m.mark(256, 256, Status::BadSector);
        // remaining 512 bytes are NonTried
        assert_eq!(m.finished_bytes(), 256);
        assert_eq!(m.bad_bytes(), 256);
    }

    #[test]
    fn has_status_reflects_presence() {
        let m = make_map(512);
        assert!(m.has_status(Status::NonTried));
        assert!(!m.has_status(Status::Finished));
    }

    // ── Mapfile serialisation ────────────────────────────────────────────────

    fn parse_data_lines(content: &str) -> Vec<(u64, u64, char)> {
        content
            .lines()
            .filter(|l| !l.starts_with('#') && !l.is_empty())
            .skip(1) // skip current_pos line
            .map(|l| {
                let parts: Vec<&str> = l.split_whitespace().collect();
                let pos = u64::from_str_radix(parts[0].trim_start_matches("0x"), 16).unwrap();
                let size = u64::from_str_radix(parts[1].trim_start_matches("0x"), 16).unwrap();
                let status = parts[2].chars().next().unwrap();
                (pos, size, status)
            })
            .collect()
    }

    #[test]
    fn mapfile_round_trip_single_region() {
        let m = make_map(0x200);
        let mut buf = Vec::new();
        write_mapfile(&mut buf, &m).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let rows = parse_data_lines(&s);
        assert_eq!(rows, vec![(0, 0x200, '?')]);
    }

    #[test]
    fn mapfile_round_trip_multi_region() {
        let mut m = make_map(0x400);
        m.mark(0x000, 0x100, Status::Finished);
        m.mark(0x100, 0x100, Status::BadSector);
        // 0x200..0x400 stays NonTried
        let mut buf = Vec::new();
        write_mapfile(&mut buf, &m).unwrap();
        let s = String::from_utf8(buf).unwrap();
        let rows = parse_data_lines(&s);
        assert_eq!(
            rows,
            vec![
                (0x000, 0x100, '+'),
                (0x100, 0x100, '-'),
                (0x200, 0x200, '?')
            ]
        );
    }

    #[test]
    fn mapfile_flush_is_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.map");
        let mut m = make_map(512);
        m.mapfile_path = path.clone();
        m.flush().unwrap();
        assert!(path.exists());
        // tmp file must not remain after rename
        assert!(!PathBuf::from(format!("{}.tmp", path.display())).exists());
    }
}
