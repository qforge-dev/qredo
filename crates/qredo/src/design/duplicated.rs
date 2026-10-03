use crate::Finding;
use std::collections::BTreeMap;

pub(crate) fn check_prepared(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
) -> Vec<Finding> {
    check_prepared_with_filename(prepared, params, "file")
}

/// The same structural engine serves both direct kernels and project runs.
pub(crate) fn check_prepared_with_filename(
    prepared: &crate::batch::Prepared<'_>,
    params: &BTreeMap<String, String>,
    filename: &str,
) -> Vec<Finding> {
    crate::project::collect_duplicated::run_summaries(&[filename], &[prepared.duplicated()], params)
        .iter()
        .map(crate::project::project_finding)
        .collect()
}
