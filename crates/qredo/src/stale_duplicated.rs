//! Duplicate groups are global, but unchanged files need no parse on edits.
use crate::project::collect_duplicated::{self as duplicated, Summary};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Cached {
    summary: Summary,
    scopes: crate::Scopes,
    bonuses: BTreeMap<String, i32>,
}

impl Cached {
    pub(crate) fn collect(prepared: &crate::batch::Prepared<'_>) -> Self {
        Self {
            summary: prepared.duplicated().clone(),
            scopes: crate::Scopes::collect_on_facts(prepared.facts()),
            bonuses: crate::scope::scope_priorities_on_facts(prepared.facts()),
        }
    }

    pub(crate) fn valid(&self) -> bool {
        self.summary.valid()
    }
}

/// Recompute global grouping and all peer issues using cached syntax/scopes.
/// `selected` and `summaries` are aligned in discovery order.
pub(crate) fn run(
    entry: &crate::CheckEntry,
    files: &[crate::RunnerFile],
    config: &crate::RunnerConfig,
    selected: &[usize],
    summaries: &[&Cached],
) -> Vec<crate::Issue> {
    let names: Vec<&str> = selected
        .iter()
        .map(|i| files[*i].filename.as_str())
        .collect();
    let structures: Vec<&Summary> = summaries.iter().map(|c| &c.summary).collect();
    let found = duplicated::run_summaries(&names, &structures, &entry.params);
    let metas: Vec<crate::FileMeta<'_>> = selected
        .iter()
        .zip(summaries)
        .map(|(index, cached)| {
            let file = &files[*index];
            crate::FileMeta {
                filename: &file.filename,
                source: &file.source,
                lines: file.source.split('\n').collect(),
                scopes: cached.scopes.clone(),
                bonuses: cached.bonuses.clone(),
            }
        })
        .collect();
    let refs: Vec<&crate::FileMeta<'_>> = metas.iter().collect();
    let Ok(issues) = crate::project::build_project_issues(
        duplicated::RULE,
        &found,
        &entry.params,
        &config.general,
        &refs,
    ) else {
        return Vec::new();
    };
    let comments: Vec<_> = selected
        .iter()
        .map(|i| crate::suppression::config_comments(&files[*i].source))
        .collect();
    issues
        .into_iter()
        .filter_map(|(index, issue)| {
            (issue.priority >= config.min_priority
                && !comments[index]
                    .iter()
                    .any(|c| c.ignores(&issue.check, issue.line_no.unwrap_or(0))))
            .then_some(issue)
        })
        .collect()
}
