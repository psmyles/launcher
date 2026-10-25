//! Recently opened files, for the home screen: `<app data>/recent.kdl`, newest first.
//!
//! ```kdl
//! file "D:/tools/build.launcher"
//! ```

use std::path::{Path, PathBuf};

use kdl::KdlDocument;

/// How many files the list keeps.
const KEEP: usize = 12;

fn store() -> Option<PathBuf> {
    Some(crate::paths::app_data_dir()?.join("recent.kdl"))
}

pub fn parse(text: &str) -> Vec<PathBuf> {
    let Ok(doc) = KdlDocument::parse_v2(text) else {
        return Vec::new();
    };
    doc.nodes()
        .iter()
        .filter(|n| n.name().value() == "file")
        .filter_map(|n| n.entries().first()?.value().as_string().map(PathBuf::from))
        .collect()
}

pub fn render(files: &[PathBuf]) -> String {
    let mut s = String::from("// Recently opened files, newest first. Written by Launcher.\n");
    for f in files {
        s.push_str(&format!(
            "file {}\n",
            crate::state::quote(&f.display().to_string())
        ));
    }
    s
}

pub fn load() -> Vec<PathBuf> {
    store()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|t| parse(&t))
        .unwrap_or_default()
}

fn save(files: &[PathBuf]) {
    let Some(path) = store() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = crate::replace_file::write_bytes_replacing(&path, render(files)) {
        crate::log_warn!("cannot save the recent files: {e}");
    }
}

/// The list with `file` moved to the front.
pub fn with_added(mut files: Vec<PathBuf>, file: &Path) -> Vec<PathBuf> {
    let key = crate::paths::config_key(file);
    files.retain(|f| crate::paths::config_key(f) != key);
    files.insert(0, file.to_path_buf());
    files.truncate(KEEP);
    files
}

/// A file was opened.
pub fn add(file: &Path) {
    save(&with_added(load(), file));
}

/// "Remove from the list".
pub fn remove(file: &Path) {
    let key = crate::paths::config_key(file);
    let mut files = load();
    files.retain(|f| crate::paths::config_key(f) != key);
    save(&files);
}

/// "Clear recent".
pub fn clear() {
    save(&[]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_newest_comes_first_once() {
        let a = PathBuf::from("C:/x/a.launcher");
        let b = PathBuf::from("C:/x/b.launcher");
        let list = with_added(with_added(with_added(Vec::new(), &a), &b), &a);
        assert_eq!(list, [a.clone(), b.clone()]);
        assert_eq!(parse(&render(&list)), list);
        let many = (0..20).fold(Vec::new(), |l, i| {
            with_added(l, &PathBuf::from(format!("C:/x/{i}.launcher")))
        });
        assert_eq!(many.len(), KEEP);
        assert_eq!(many[0], PathBuf::from("C:/x/19.launcher"));
    }
}
