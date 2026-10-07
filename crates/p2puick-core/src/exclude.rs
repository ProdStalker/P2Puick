//! Directory names skipped while walking trees for transfer.

/// Default folder names ignored during indexing (dev/build junk).
pub fn default_exclude_dir_names() -> Vec<String> {
    DEFAULT_EXCLUDE_DIR_NAMES
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

pub const DEFAULT_EXCLUDE_DIR_NAMES: &[&str] = &[
    "node_modules",
    "vendor",
    "var",
    ".git",
    ".svn",
    ".hg",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    ".output",
    ".cache",
    ".turbo",
    ".parcel-cache",
    "__pycache__",
    ".venv",
    "venv",
    ".idea",
    ".vscode",
    "coverage",
    "tmp",
    "temp",
    "bower_components",
    "Pods",
    "DerivedData",
    ".gradle",
    ".dart_tool",
];

/// True if this path component (folder name) should be skipped.
pub fn is_excluded_dir_name(name: &str, excludes: &[String]) -> bool {
    let name = name.trim();
    if name.is_empty() {
        return false;
    }
    excludes
        .iter()
        .any(|ex| ex.trim().eq_ignore_ascii_case(name))
}

/// Normalize user-provided exclude list (trim, drop empty, dedupe case-insensitively).
pub fn normalize_excludes(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for item in raw {
        let t = item.trim();
        if t.is_empty() {
            continue;
        }
        if out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(t))
        {
            continue;
        }
        out.push(t.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skips_node_modules() {
        let ex = default_exclude_dir_names();
        assert!(is_excluded_dir_name("node_modules", &ex));
        assert!(is_excluded_dir_name("Node_Modules", &ex));
        assert!(!is_excluded_dir_name("src", &ex));
    }
}
