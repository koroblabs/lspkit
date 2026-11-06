use std::path::PathBuf;

/// Every file under `paths`, sorted. Skips git-ignored files and does not descend into hidden
/// directories or build output. A path that names a file is returned as is.
pub(crate) fn under(paths: &[String]) -> Vec<PathBuf> {
    lspkit_walk::Walk::new(paths.iter().map(PathBuf::from))
        .prune(|entry| {
            entry.is_dir
                && matches!(
                    entry.path.file_name().and_then(|name| name.to_str()),
                    Some("target" | "node_modules")
                )
        })
        .files()
}
