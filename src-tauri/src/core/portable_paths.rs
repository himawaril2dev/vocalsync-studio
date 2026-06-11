use crate::error::AppError;
use std::path::{Component, Path, PathBuf};

const PORTABLE_PATH_PREFIX: &str = "portable:";

pub fn root_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn path(relative_path: impl AsRef<Path>) -> PathBuf {
    root_dir().join(relative_path)
}

pub fn ensure_dir(relative_path: impl AsRef<Path>) -> Result<PathBuf, AppError> {
    let dir = path(relative_path);
    std::fs::create_dir_all(&dir).map_err(AppError::Io)?;
    Ok(dir)
}

pub fn settings_path() -> PathBuf {
    path("settings.json")
}

pub fn display_path(path: &Path) -> String {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        text.strip_prefix(r"\\?\").unwrap_or(&text).to_string()
    }

    #[cfg(not(windows))]
    {
        path.to_string_lossy().to_string()
    }
}

fn portable_relative_text(path: &Path) -> Option<String> {
    let canonical_path = std::fs::canonicalize(path).ok()?;
    let canonical_root = std::fs::canonicalize(root_dir()).ok()?;
    let relative = canonical_path.strip_prefix(canonical_root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

pub fn is_inside_root(path: &Path) -> bool {
    let Ok(canonical_path) = std::fs::canonicalize(path) else {
        return false;
    };
    let Ok(canonical_root) = std::fs::canonicalize(root_dir()) else {
        return false;
    };
    canonical_path.starts_with(canonical_root)
}

pub fn encode_path_for_storage(path: &Path) -> String {
    portable_relative_text(path)
        .map(|relative| format!("{PORTABLE_PATH_PREFIX}{relative}"))
        .unwrap_or_else(|| display_path(path))
}

pub fn resolve_stored_path_text(value: &str) -> Option<PathBuf> {
    if let Some(relative) = value.strip_prefix(PORTABLE_PATH_PREFIX) {
        let relative_path = Path::new(relative);
        if relative_path.is_absolute()
            || relative_path
                .components()
                .any(|component| !matches!(component, Component::Normal(_)))
        {
            return None;
        }
        return Some(path(relative_path));
    }
    Some(PathBuf::from(value))
}
