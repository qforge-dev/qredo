//! Persistent incremental cache for `--stale` runs.
//!
//! Layout: `$XDG_CACHE_HOME/qredo/<proj>/cache-v3.json`, falling back to
//! `~/.cache/qredo/<proj>/` when `XDG_CACHE_HOME` is unset. `<proj>` is the
//! first 16 hex chars of the SHA-256 over the canonical root path, so
//! distinct checkouts never share entries. A moved checkout simply starts
//! cold under a new directory; no garbage collection yet.
//!
//! Per file we store the content hash (uppercase hex SHA-256, matching
//! [`crate::SourceSnapshot`]), per-check project vote counts
//! (`check -> kind -> count`), syntax validity and comment-validation
//! outcome. Globally we store the sorted report, config fingerprint,
//! file order, pattern-validation result and per-check majority winners.
//! Any fingerprint mismatch invalidates the whole cache; a content hash
//! mismatch invalidates just that file. Corrupt or unreadable caches fail
//! open to a full run, never to silent divergence.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// On-disk schema version; bumps invalidate every existing cache.
pub const CACHE_VERSION: u32 = 3;

/// Cached comment-registration failure for one exact source hash.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachedCommentError {
    pub line_no: usize,
    pub message: String,
}

/// One cached file unit.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CachedFile {
    /// Uppercase hex SHA-256 over exact source bytes.
    pub hash: String,
    /// Project vote counts by check (`check -> kind -> count`).
    pub project_counts: BTreeMap<String, BTreeMap<String, usize>>,
    /// Syntax-gate outcome: invalid files stay skipped without re-parsing.
    pub skipped_invalid: bool,
    /// `defmodule` + `def`-family count for this valid file (zero when
    /// validation excludes it).
    pub mods_funs: usize,
    /// Comment-validation failure, reusable only with the matching hash.
    pub comment_error: Option<CachedCommentError>,
}

/// Whole-project cache payload.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct DiskCache {
    /// [`CACHE_VERSION`] at write time.
    pub version: u32,
    /// `qredo` version at write time (`CARGO_PKG_VERSION`).
    pub qredo_version: String,
    /// Global config fingerprint (see [`fingerprint`]).
    pub fingerprint: String,
    /// Discovery-order filenames. Exact equality enables the clean fast path
    /// and safe reuse of filename-dependent pattern validation.
    pub file_order: Vec<String>,
    /// Final globally sorted post-filter report issues.
    pub issues: Vec<crate::Issue>,
    /// OR-combined status for [`Self::issues`].
    pub exit_status: i32,
    /// Exact errors produced when this payload was built.
    pub errors: Vec<crate::RunError>,
    /// Exact invalid-syntax filenames in discovery order.
    pub skipped_invalid: Vec<String>,
    /// Project-wide `defmodule` + `def`-family count for default output.
    pub mods_funs: usize,
    /// Checks stopped by lazily reached file-pattern errors.
    pub pattern_stopped: BTreeSet<String>,
    /// Pattern errors in check order, reusable while [`Self::file_order`]
    /// stays identical.
    pub pattern_errors: Vec<crate::RunError>,
    /// Majority winner per project check (`None` = no votes).
    pub winners: BTreeMap<String, Option<String>>,
    /// Display filename (root-relative) to cached unit.
    pub files: BTreeMap<String, CachedFile>,
}

impl DiskCache {
    /// Empty cache for one fingerprint.
    #[must_use]
    pub fn empty(fingerprint: String) -> Self {
        Self {
            version: CACHE_VERSION,
            qredo_version: env!("CARGO_PKG_VERSION").to_owned(),
            fingerprint,
            file_order: Vec::new(),
            issues: Vec::new(),
            exit_status: 0,
            errors: Vec::new(),
            skipped_invalid: Vec::new(),
            mods_funs: 0,
            pattern_stopped: BTreeSet::new(),
            pattern_errors: Vec::new(),
            winners: BTreeMap::new(),
            files: BTreeMap::new(),
        }
    }
}

/// Uppercase hex SHA-256 over exact bytes, matching `SourceSnapshot`.
#[must_use]
pub fn content_hash(source: &str) -> String {
    let digest = Sha256::digest(source.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02X}");
    }
    out
}

/// Lowercase hex SHA-256 over one string (cache keys, fingerprints).
fn hex_lower(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Cache directory for one project root, or `None` when no home/XDG base
/// resolves (callers then run uncached).
#[must_use]
pub fn cache_dir(root: &Path) -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    let mut canonical = root.to_string_lossy().into_owned();
    while canonical.len() > 1 && canonical.ends_with('/') {
        canonical.pop();
    }
    let proj = hex_lower(&canonical);
    Some(base.join("qredo").join(&proj[..16]))
}

/// Cache file path for one project root, or `None` when unresolvable.
#[must_use]
pub fn cache_file(root: &Path) -> Option<PathBuf> {
    cache_dir(root).map(|dir| dir.join("cache-v3.json"))
}

/// Global fingerprint over everything that can change issue output for
/// identical file bytes: tool version, config bytes + name + env snapshot,
/// enabled checks with params in config order, CLI selection and minimum
/// priority. File contents and file lists are per-file keys, not global.
#[allow(
    clippy::too_many_arguments,
    reason = "fingerprint spans every output-determining input by design"
)]
#[must_use]
pub fn fingerprint(
    config_source: &str,
    config_name: &str,
    env_snapshot: &BTreeMap<String, Option<String>>,
    checks: &[crate::CheckEntry],
    selection: &crate::Selection,
    min_priority: i32,
) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "qredo={}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(text, "config_name={config_name}");
    let _ = writeln!(text, "config_source_len={}", config_source.len());
    text.push_str(config_source);
    text.push('\n');
    for (name, value) in env_snapshot {
        let _ = writeln!(text, "env:{name}={value:?}");
    }
    for check in checks {
        let _ = writeln!(text, "check:{}:enabled={}", check.module, check.enabled);
        for (key, value) in &check.params {
            let _ = writeln!(text, "param:{}:{key}={value}", check.module);
        }
    }
    let mut only = selection.only.clone();
    only.sort();
    let mut ignore = selection.ignore.clone();
    ignore.sort();
    let mut tags = selection.checks_with_tag.clone();
    tags.sort();
    let mut reenable = selection.enable_disabled.clone();
    reenable.sort();
    let _ = writeln!(text, "only:{only:?}\nignore:{ignore:?}");
    let _ = writeln!(text, "tags:{tags:?}\nreenable:{reenable:?}");
    let _ = writeln!(text, "min_priority={min_priority}");
    hex_lower(&text)
}

/// Load a cache hit: file must parse, match schema version, tool version
/// and fingerprint. Anything else is `None` (cold run, fail open).
#[must_use]
pub fn load(root: &Path, fingerprint: &str) -> Option<DiskCache> {
    load_file(&cache_file(root)?, fingerprint)
}

/// Persist one cache atomically (temp + rename). Errors are swallowed:
/// caching must never break linting.
pub fn save(root: &Path, cache: &DiskCache) {
    let Some(path) = cache_file(root) else {
        return;
    };
    save_file(&path, cache);
}

/// Load from an explicit path (same validation as [`load`]). Exposed so
/// tests can isolate without mutating process-global environment.
#[must_use]
pub fn load_file(path: &Path, fingerprint: &str) -> Option<DiskCache> {
    let bytes = std::fs::read(path).ok()?;
    let cache: DiskCache = serde_json::from_slice(&bytes).ok()?;
    if cache.version != CACHE_VERSION {
        return None;
    }
    if cache.qredo_version != env!("CARGO_PKG_VERSION") {
        return None;
    }
    if cache.fingerprint != fingerprint {
        return None;
    }
    Some(cache)
}

/// Persist to an explicit path atomically. Errors are swallowed.
pub fn save_file(path: &Path, cache: &DiskCache) {
    let Some(dir) = path.parent() else {
        return;
    };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec(cache) else {
        return;
    };
    let Some(filename) = path.file_name() else {
        return;
    };
    let mut tmp_name = filename.to_os_string();
    tmp_name.push(".tmp");
    let tmp = dir.join(tmp_name);
    if std::fs::write(&tmp, bytes).is_err() {
        return;
    }
    let _ = std::fs::rename(&tmp, path);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn selection() -> crate::Selection {
        crate::Selection::default()
    }

    fn checks() -> Vec<crate::CheckEntry> {
        vec![crate::CheckEntry {
            module: "Credo.Check.Warning.IoInspect".to_owned(),
            enabled: true,
            params: BTreeMap::new(),
        }]
    }

    #[test]
    fn content_hash_matches_snapshot() {
        let source = "defmodule A do\nend\n";
        assert_eq!(
            content_hash(source),
            crate::SourceSnapshot::parse(source, "lib/a.ex").hash()
        );
    }

    #[test]
    fn fingerprint_is_stable_and_sensitive() {
        let env = BTreeMap::new();
        let base = fingerprint("cfg", "default", &env, &checks(), &selection(), 0);
        assert_eq!(
            base,
            fingerprint("cfg", "default", &env, &checks(), &selection(), 0)
        );
        assert_ne!(
            base,
            fingerprint("cfg2", "default", &env, &checks(), &selection(), 0)
        );
        assert_ne!(
            base,
            fingerprint("cfg", "default", &env, &checks(), &selection(), -99)
        );
        let mut other_selection = selection();
        other_selection.only = vec!["Dbg".to_owned()];
        assert_ne!(
            base,
            fingerprint("cfg", "default", &env, &checks(), &other_selection, 0)
        );
    }

    #[test]
    fn cache_round_trips_through_explicit_path() {
        let dir = std::env::temp_dir().join("qredo-cache-test-home");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("cache-v3.json");
        let mut cache = DiskCache::empty("fp".to_owned());
        cache.files.insert(
            "lib/a.ex".to_owned(),
            CachedFile {
                hash: content_hash("x = 1\n"),
                project_counts: BTreeMap::new(),
                skipped_invalid: false,
                mods_funs: 0,
                comment_error: None,
            },
        );
        save_file(&path, &cache);
        assert_eq!(load_file(&path, "fp"), Some(cache));
        assert_eq!(load_file(&path, "other"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fast_hit_metadata_round_trips() {
        let dir = std::env::temp_dir().join("qredo-cache-test-fast-hit");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("cache-v3.json");
        let mut cache = DiskCache::empty("fp".to_owned());
        cache.file_order = vec!["lib/a.ex".to_owned()];
        cache.exit_status = 2;
        cache.errors = vec![crate::RunError::Pattern("cached-pattern".to_owned())];
        cache.pattern_stopped.insert("Check.A".to_owned());
        cache
            .pattern_errors
            .push(crate::RunError::Pattern("cached-pattern".to_owned()));
        cache.files.insert(
            "lib/a.ex".to_owned(),
            CachedFile {
                hash: content_hash("# credo:disable-for-lines:nope\n"),
                project_counts: BTreeMap::new(),
                skipped_invalid: false,
                mods_funs: 0,
                comment_error: Some(CachedCommentError {
                    line_no: 1,
                    message: "cached-comment".to_owned(),
                }),
            },
        );
        save_file(&path, &cache);
        assert_eq!(load_file(&path, "fp"), Some(cache));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_cache_fails_open() {
        let dir = std::env::temp_dir().join("qredo-cache-test-corrupt");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("cache-v3.json");
        std::fs::write(&path, b"not json").expect("write");
        assert_eq!(load_file(&path, "fp"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_dir_nests_under_xdg_or_home() {
        // Read-only env access: parallel-safe, no mutation.
        let dir = cache_dir(Path::new("/proj/app")).expect("resolvable");
        let text = dir.to_string_lossy().into_owned();
        assert!(text.contains("qredo"), "{text}");
        assert_eq!(
            cache_file(Path::new("/proj/app")).expect("resolvable"),
            dir.join("cache-v3.json")
        );
    }
}
