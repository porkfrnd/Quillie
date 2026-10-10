//! Source file fingerprints (§7.1).
//!
//! # Why this is not a whole-file hash
//!
//! The point of a fingerprint is to notice that a file *changed*, so a
//! cached proxy can be invalidated and a relink can be suggested. It is not
//! to prove the contents forever.
//!
//! Hashing a 40 GB source on import would turn every import into a multi-minute
//! stall on a 5400 RPM disk -- the exact machine class the floor is defined
//! by. Size, mtime, and a hash of the first and last MiB catch every realistic
//! case (a re-export changes both size and content; a rename changes mtime)
//! at a cost that is two 1 MiB reads.

use crate::project::Fingerprint;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::UNIX_EPOCH;

pub const WINDOW: usize = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum FingerprintError {
    #[error("cannot read {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
}

/// A 128-bit window hash.
///
/// A custom mixing function rather than a general-purpose hash crate: this runs
/// once per import over 2 MiB, and adding a dependency for it is not worth the
/// licence review. The distribution matters, not cryptographic strength --
/// this identifies a file, it does not authenticate one.
fn window_hash(bytes: &[u8], seed: u64) -> [u8; 16] {
    // FNV-1a, 64-bit, run twice with different seeds to fill 16 bytes.
    fn fnv1a(bytes: &[u8], seed: u64) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64 ^ seed;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        // Final avalanche so the low bits are not correlated with length.
        h ^= h >> 33;
        h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
        h ^ (h >> 33)
    }
    let a = fnv1a(bytes, seed);
    let b = fnv1a(bytes, seed ^ 0x9e37_79b9_7f4a_7c15);
    let mut out = [0u8; 16];
    out[..8].copy_from_slice(&a.to_le_bytes());
    out[8..].copy_from_slice(&b.to_le_bytes());
    out
}

/// Fingerprint a file from its first and last MiB.
///
/// Symlinks are followed. A file smaller than 2 MiB is hashed whole (both
/// windows overlap, which is harmless).
pub fn fingerprint_file(path: &Path) -> Result<Fingerprint, FingerprintError> {
    let err = |e: std::io::Error| FingerprintError::Io { path: path.display().to_string(), source: e };

    let meta = std::fs::metadata(path).map_err(err)?;
    let size = meta.len();
    let mtime_unix = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    let mut f = File::open(path).map_err(err)?;

    let mut head = vec![0u8; WINDOW.min(size as usize)];
    f.read_exact(&mut head).map_err(err)?;

    let tail = if size > WINDOW as u64 {
        let tail_len = WINDOW.min((size as usize).saturating_sub(WINDOW));
        f.seek(SeekFrom::Start(size - tail_len as u64)).map_err(err)?;
        let mut buf = vec![0u8; tail_len];
        f.read_exact(&mut buf).map_err(err)?;
        buf
    } else {
        head.clone()
    };

    Ok(Fingerprint {
        size_bytes: size,
        mtime_unix,
        head_hash: window_hash(&head, 0),
        tail_hash: window_hash(&tail, 1),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("quillie-fp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(name);
        let mut f = File::create(&p).unwrap();
        f.write_all(bytes).unwrap();
        p
    }

    #[test]
    fn identical_content_gives_identical_fingerprints() {
        let a = tmp("a.bin", &(0..4096u32).map(|i| i as u8).collect::<Vec<u8>>());
        let b = tmp("b.bin", &(0..4096u32).map(|i| i as u8).collect::<Vec<u8>>());
        let fa = fingerprint_file(&a).unwrap();
        let fb = fingerprint_file(&b).unwrap();
        assert_eq!(fa.head_hash, fb.head_hash);
        assert_eq!(fa.tail_hash, fb.tail_hash);
        assert_eq!(fa.size_bytes, fb.size_bytes);
    }

    #[test]
    fn a_changed_head_changes_the_fingerprint() {
        let a = tmp("c1.bin", &vec![1u8; 8192]);
        let b = tmp("c2.bin", &vec![1u8; 4096]);
        let fa = fingerprint_file(&a).unwrap();
        let fb = fingerprint_file(&b).unwrap();
        assert_ne!(fa.head_hash, fb.head_hash);
    }

    #[test]
    fn a_changed_tail_is_detected_even_when_the_head_matches() {
        // Two 3 MiB files with identical first MiB but different last MiB.
        // This is the case a head-only hash would miss entirely.
        let mut a = vec![7u8; 3 * 1024 * 1024];
        let mut b = a.clone();
        let last = a.len() - 1;
        a[last] = 0xAA;
        b[last] = 0xBB;
        let pa = tmp("t1.bin", &a);
        let pb = tmp("t2.bin", &b);
        let fa = fingerprint_file(&pa).unwrap();
        let fb = fingerprint_file(&pb).unwrap();
        assert_eq!(fa.head_hash, fb.head_hash, "heads should match");
        assert_ne!(fa.tail_hash, fb.tail_hash, "tails must differ");
    }

    #[test]
    fn a_missing_file_is_an_error_not_a_panic() {
        let r = fingerprint_file(Path::new("/definitely/not/here.mp4"));
        assert!(r.is_err());
    }

    #[test]
    fn an_empty_file_is_handled() {
        let p = tmp("empty.bin", b"");
        let f = fingerprint_file(&p).unwrap();
        assert_eq!(f.size_bytes, 0);
    }
}