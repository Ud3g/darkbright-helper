//! Rolling file sink for diagnostic logging.
//!
//! Release builds hide the console, so log output is lost unless captured in
//! a file. This module provides a size-capped two-file rotation: the active
//! file plus one `.old` predecessor, bounding worst-case disk use while
//! always retaining a window of recent history.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::PathBuf;

/// File name of the active log file (lives in the config directory).
pub const LOG_FILE_NAME: &str = "darkbright.log";

/// File name of the rotated predecessor.
pub const LOG_FILE_OLD_NAME: &str = "darkbright.log.old";

/// Rotation threshold for the active file (worst-case disk use is ~2×).
pub const LOG_MAX_BYTES: u64 = 1024 * 1024;

/// Size-capped log file writer with two-file rotation.
///
/// When a write would push the active file past `max_bytes`, the active file
/// is renamed over the `.old` sibling (replacing it) and a fresh active file
/// starts, so at least one full window of recent history always survives.
#[derive(Debug)]
pub struct RotatingFileWriter {
    file: File,
    path: PathBuf,
    old_path: PathBuf,
    current_len: u64,
    max_bytes: u64,
}

impl RotatingFileWriter {
    /// Opens the log file at `path` for appending, creating it if missing.
    /// The rotated sibling gets the same name with `.old` appended.
    ///
    /// # Errors
    ///
    /// Returns any I/O error from opening or inspecting the file.
    pub fn open(path: PathBuf, max_bytes: u64) -> io::Result<Self> {
        let old_path = {
            let mut os = path.clone().into_os_string();
            os.push(".old");
            PathBuf::from(os)
        };
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let current_len = file.metadata()?.len();
        Ok(Self {
            file,
            path,
            old_path,
            current_len,
            max_bytes,
        })
    }

    /// Renames the active file over the `.old` sibling and starts fresh.
    ///
    /// On failure (e.g. the file is transiently locked by a scanner) the
    /// current file stays in use and the next over-cap write retries, so a
    /// failed rotation degrades to a temporarily oversized file, never to
    /// lost logging.
    fn rotate(&mut self) -> io::Result<()> {
        self.file.flush()?;
        // Windows: std opens files with FILE_SHARE_DELETE, so renaming while
        // our handle is open is fine; the handle then points at the .old file
        // and is replaced below.
        std::fs::rename(&self.path, &self.old_path)?;
        self.file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        self.current_len = 0;
        Ok(())
    }
}

impl Write for RotatingFileWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let incoming = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        // Never rotate an empty file: a single over-cap record is written
        // as-is instead of rotating forever without progress.
        if self.current_len > 0 && self.current_len.saturating_add(incoming) > self.max_bytes {
            // A failed rotation must not cost the record: keep appending to
            // the oversized file — the next over-cap write retries. env_logger
            // discards writer errors, so propagating here would silently drop
            // every record for as long as the rotation keeps failing.
            let _ = self.rotate();
        }
        let written = self.file.write(buf)?;
        self.current_len = self
            .current_len
            .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn appends_across_reopen() {
        let dir = test_dir("darkbright_test_logfile_append");
        let path = dir.join(LOG_FILE_NAME);
        {
            let mut w = RotatingFileWriter::open(path.clone(), 1024).unwrap();
            w.write_all(b"first\n").unwrap();
        }
        let mut w = RotatingFileWriter::open(path.clone(), 1024).unwrap();
        w.write_all(b"second\n").unwrap();
        w.flush().unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, "first\nsecond\n");
    }

    #[test]
    fn rotates_when_the_size_cap_would_be_exceeded() {
        let dir = test_dir("darkbright_test_logfile_rotate");
        let path = dir.join(LOG_FILE_NAME);
        let mut w = RotatingFileWriter::open(path.clone(), 10).unwrap();
        w.write_all(b"0123456789").unwrap(); // exactly at the cap: no rotation
        w.write_all(b"next\n").unwrap(); // would exceed: rotates first
        w.flush().unwrap();

        let old = std::fs::read_to_string(dir.join(LOG_FILE_OLD_NAME)).unwrap();
        assert_eq!(old, "0123456789");
        let active = std::fs::read_to_string(&path).unwrap();
        assert_eq!(active, "next\n");
    }

    #[test]
    fn failed_rotation_appends_to_oversized_file_instead_of_dropping_records() {
        let dir = test_dir("darkbright_test_logfile_rotate_blocked");
        let path = dir.join(LOG_FILE_NAME);
        // Block rotation: a directory squatting on the .old path makes the
        // rename fail, standing in for a transient lock by a scanner.
        std::fs::create_dir(dir.join(LOG_FILE_OLD_NAME)).unwrap();

        let mut w = RotatingFileWriter::open(path.clone(), 4).unwrap();
        w.write_all(b"aaaa").unwrap();
        w.write_all(b"bbbb").unwrap(); // over cap, rotation fails: must still be written
        w.write_all(b"cccc").unwrap(); // and logging must keep going
        w.flush().unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "aaaabbbbcccc");
    }

    #[test]
    fn rotation_recovers_once_the_blocker_clears() {
        let dir = test_dir("darkbright_test_logfile_rotate_recover");
        let path = dir.join(LOG_FILE_NAME);
        let blocker = dir.join(LOG_FILE_OLD_NAME);
        std::fs::create_dir(&blocker).unwrap();

        let mut w = RotatingFileWriter::open(path.clone(), 4).unwrap();
        w.write_all(b"aaaa").unwrap();
        w.write_all(b"bbbb").unwrap(); // rotation blocked, file oversized
        std::fs::remove_dir(&blocker).unwrap();
        w.write_all(b"cccc").unwrap(); // retry succeeds: oversized file rotates out
        w.flush().unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join(LOG_FILE_OLD_NAME)).unwrap(),
            "aaaabbbb"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "cccc");
    }

    #[test]
    fn second_rotation_replaces_the_old_file() {
        let dir = test_dir("darkbright_test_logfile_rotate2");
        let path = dir.join(LOG_FILE_NAME);
        let mut w = RotatingFileWriter::open(path.clone(), 4).unwrap();
        w.write_all(b"aaaa").unwrap();
        w.write_all(b"bbbb").unwrap(); // rotation 1: .old = aaaa
        w.write_all(b"cccc").unwrap(); // rotation 2: .old replaced with bbbb
        w.flush().unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.join(LOG_FILE_OLD_NAME)).unwrap(),
            "bbbb"
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "cccc");
    }
}
