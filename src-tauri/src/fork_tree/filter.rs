use super::types::CachedFileData;

/// Filter cached file entries to only those whose `project_dir` matches the given path.
/// Comparison is case-insensitive and slash-normalized for cross-platform robustness.
pub(crate) fn filter_files_by_project_dir(
    files: &[CachedFileData],
    target: &str,
) -> Vec<CachedFileData> {
    let target_norm = normalize_path_for_comparison(target);
    files
        .iter()
        .filter(|f| {
            f.project_dir
                .as_deref()
                .map(|pd| normalize_path_for_comparison(pd) == target_norm)
                .unwrap_or(false)
        })
        .cloned()
        .collect()
}

/// Count files matching the optional project_dir filter.
#[allow(dead_code)]
pub(crate) fn count_filtered(files: &[CachedFileData], filter: Option<&str>) -> u32 {
    match filter {
        Some(dir) => filter_files_by_project_dir(files, dir).len() as u32,
        None => files.len() as u32,
    }
}

/// Normalize a path string for comparison: lowercase, normalize backslashes.
fn normalize_path_for_comparison(path: &str) -> String {
    path.replace('/', "\\")
        .trim_end_matches('\\')
        .to_lowercase()
}
