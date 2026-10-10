//! Custom URI scheme for frames, thumbnails and peaks (§5 rule 6).
//!
//! Binary media never crosses the IPC boundary as JSON. Decoded RGBA through
//! `invoke` would mean base64, a 33% size increase, and a JSON parse on the
//! hot path of every preview frame.
//!
//! Instead the host registers a `quillie://` scheme and the UI requests media
//! by URL. The webview's own image and media decoders handle the payload, so
//! the bytes go straight from Rust to the compositor with no JS involvement.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The scheme the host registers.
pub const SCHEME: &str = "quillie";

/// One cached binary artefact, addressed by an opaque id.
#[derive(Clone)]
pub struct Resource {
    pub bytes: Arc<Vec<u8>>,
    pub mime: &'static str,
}

/// In-memory resources keyed by id.
///
/// This is the *reference* implementation and a correctness harness. The real
/// one reads from the cache directory (§14.2) and registers each entry with the
/// budget manager so it can be evicted. Keeping the interface narrow now means
/// swapping the backing store later touches this file alone.
#[derive(Default)]
pub struct ResourceStore {
    entries: Mutex<HashMap<String, Resource>>,
}

impl ResourceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&self, id: impl Into<String>, bytes: Vec<u8>, mime: &'static str) {
        self.entries
            .lock()
            .expect("resource store mutex poisoned")
            .insert(id.into(), Resource { bytes: Arc::new(bytes), mime });
    }

    pub fn get(&self, id: &str) -> Option<Resource> {
        self.entries
            .lock()
            .expect("resource store mutex poisoned")
            .get(id)
            .cloned()
    }

    /// Drop a resource and report how many bytes were released.
    pub fn evict(&self, id: &str) -> usize {
        self.entries
            .lock()
            .expect("resource store mutex poisoned")
            .remove(id)
            .map(|r| r.bytes.len())
            .unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.entries.lock().expect("resource store mutex poisoned").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn bytes_used(&self) -> usize {
        self.entries
            .lock()
            .expect("resource store mutex poisoned")
            .values()
            .map(|r| r.bytes.len())
            .sum()
    }
}

/// Build the URL the UI requests a resource by.
///
/// Ids are opaque strings. They are validated here rather than trusted,
/// because a `..` in an id would let a request escape the resource namespace.
pub fn resource_url(id: &str) -> Option<String> {
    if id.is_empty() || id.len() > 128 {
        return None;
    }
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    Some(format!("{SCHEME}://resource/{id}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resources_round_trip() {
        let s = ResourceStore::new();
        s.put("frame-1", vec![1, 2, 3], "image/webp");
        let r = s.get("frame-1").unwrap();
        assert_eq!(*r.bytes, vec![1, 2, 3]);
        assert_eq!(r.mime, "image/webp");
        assert_eq!(s.len(), 1);
        assert_eq!(s.bytes_used(), 3);
    }

    #[test]
    fn eviction_reports_the_bytes_it_freed() {
        let s = ResourceStore::new();
        s.put("a", vec![0u8; 100], "image/webp");
        assert_eq!(s.evict("a"), 100);
        assert_eq!(s.evict("a"), 0);
        assert!(s.is_empty());
    }

    #[test]
    fn urls_are_built_only_for_safe_ids() {
        assert!(resource_url("frame-12_ab").is_some());
        // Path traversal and separators are rejected outright.
        assert_eq!(resource_url("../etc/passwd"), None);
        assert_eq!(resource_url("a/b"), None);
        assert_eq!(resource_url(""), None);
        assert_eq!(resource_url(&"x".repeat(129)), None);
    }
}