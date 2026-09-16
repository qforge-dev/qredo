//! Benchmark harness: batch API over a project tree or frozen snapshot.
//!
//! Usage: `bench_labqoat` <approot> [iters] [threads] [--per-rule]
//!
//! File reads happen once outside the timed region; each iteration runs all
//! `rules x files` kernel evaluations through the parallel batch API and
//! reports kernel-only seconds plus per-rule finding counts. With the
//! `hotpath` feature it additionally prints a per-rule timing attribution
//! table for optimization work.

use std::path::{Path, PathBuf};
use std::time::Instant;

/// Sorted recursive collection of `.ex`/`.exs` files.
fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, out);
        } else if path
            .extension()
            .is_some_and(|ext| ext == "ex" || ext == "exs")
        {
            out.push(path);
        }
    }
}

fn main() {
    run_inner();
}

fn run_inner() {
    let args: Vec<String> = std::env::args().collect();
    let root = args
        .get(1)
        .expect("usage: bench_labqoat <approot> [iters] [threads]");
    let iters: usize = args
        .get(2)
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let threads: usize = args
        .get(3)
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(default_threads);
    let per_rule = args.iter().any(|arg| arg == "--per-rule");
    let sources = read_sources(root);
    run_pass(&sources, iters, threads, per_rule);
}

/// Default worker count: all logical cores, eight on lookup failure.
fn default_threads() -> usize {
    std::thread::available_parallelism().map_or(8, std::num::NonZero::get)
}

/// Sorted `lib/` + `test/` sources under the snapshot root.
fn read_sources(root: &str) -> Vec<String> {
    let mut files = Vec::new();
    collect(&PathBuf::from(root).join("lib"), &mut files);
    collect(&PathBuf::from(root).join("test"), &mut files);
    files
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("source readable"))
        .collect()
}

/// Timed parallel passes over already-read sources.
#[cfg_attr(feature = "hotpath", hotpath::main)]
fn run_pass(sources: &[String], iters: usize, threads: usize, per_rule: bool) {
    let bytes: usize = sources.iter().map(String::len).sum();
    let params = std::collections::BTreeMap::new();
    // Warm the rule list once (also asserts 120 outcomes per file).
    let first = qredo::check_all_kernels(&sources[0], &params);
    println!(
        "rules={} files={} bytes={}",
        first.len(),
        sources.len(),
        bytes
    );
    for iter in 0..iters {
        let start = Instant::now();
        let mut findings = 0_usize;
        let mut rule_totals: std::collections::BTreeMap<&str, usize> =
            std::collections::BTreeMap::new();
        let refs: Vec<&str> = sources.iter().map(String::as_str).collect();
        for file in qredo::check_sources_parallel(&refs, &params, threads) {
            for outcome in &file.rules {
                findings += outcome.findings.len();
                *rule_totals.entry(outcome.rule).or_default() += outcome.findings.len();
            }
        }
        println!(
            "iter={iter} secs={:.3} findings={findings} threads={threads}",
            start.elapsed().as_secs_f64()
        );
        if per_rule {
            for (rule, count) in &rule_totals {
                println!("rule={rule} findings={count}");
            }
        }
    }
}
