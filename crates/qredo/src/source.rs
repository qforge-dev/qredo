//! Captured source identity for lint pipelines.
//!
//! Follows `INPUTS.md`: immutable bytes plus filename context. The content
//! digest is SHA-256 over the exact bytes (uppercase hex, matching Credo's
//! `Base.encode16`), independent of checkout root. Syntax validation is not
//! performed here; see `pipeline` for the explicit pending boundary.

use sha2::{Digest, Sha256};
use std::fmt::Write as _;

/// Immutable captured source plus filename context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSnapshot {
    filename: String,
    content: String,
    hash: String,
}

impl SourceSnapshot {
    /// Capture source text with filename context.
    #[must_use]
    pub fn parse(content: &str, filename: &str) -> Self {
        let digest = Sha256::digest(content.as_bytes());
        let mut hash = String::with_capacity(64);
        for byte in digest {
            let _ = write!(hash, "{byte:02X}");
        }
        Self {
            filename: filename.to_owned(),
            content: content.to_owned(),
            hash,
        }
    }

    #[must_use]
    pub fn filename(&self) -> &str {
        &self.filename
    }

    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }

    #[must_use]
    pub fn lines(&self) -> Vec<&str> {
        self.content.split('\n').collect()
    }

    #[must_use]
    pub fn line_count(&self) -> usize {
        self.content.split('\n').count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_filename_and_hash() {
        let first = SourceSnapshot::parse("x = 1\n", "lib/a.ex");
        let second = SourceSnapshot::parse("x = 1\n", "lib/b.ex");
        assert_eq!(first.content(), "x = 1\n");
        assert_eq!(first.filename(), "lib/a.ex");
        // Same bytes share identity across filenames; filename stays contextual.
        assert_eq!(first.hash(), second.hash());
        assert_ne!(first.filename(), second.filename());
    }

    #[test]
    fn different_bytes_have_different_hashes() {
        let a = SourceSnapshot::parse("x = 1\n", "a.ex");
        let b = SourceSnapshot::parse("x = 2\n", "a.ex");
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn line_split_matches_credo_semantics() {
        let snapshot = SourceSnapshot::parse("a\r\nb\n", "a.ex");
        assert_eq!(snapshot.lines(), vec!["a\r", "b", ""]);
    }
}
