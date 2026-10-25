//! Where the launcher keeps per-user files, how a config is keyed, and how config-relative paths
//! resolve.
//!
//! App data is `%LOCALAPPDATA%\Launcher` on Windows and `~/Library/Application Support/Launcher`
//! on macOS (`LAUNCHER_DATA_DIR` overrides both, for tests). Every relative path a config names
//! resolves against the config's folder, never the working directory (N3).

use std::path::{Component, Path, PathBuf};

/// The config file extension (N1).
pub const EXTENSION: &str = "launcher";

/// The per-user app-data folder.
pub fn app_data_dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("LAUNCHER_DATA_DIR") {
        return Some(PathBuf::from(d));
    }
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(|d| PathBuf::from(d).join("Launcher"))
    }
    #[cfg(target_os = "macos")]
    {
        std::env::var_os("HOME").map(|h| {
            PathBuf::from(h)
                .join("Library")
                .join("Application Support")
                .join("Launcher")
        })
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share/launcher"))
    }
}

/// Tests: one app-data folder for the whole test binary, set the first time it's asked for.
/// `LAUNCHER_DATA_DIR` belongs to the process and tests run at once, so a test that set its own
/// would move another test's run locks and trust tokens while it ran.
#[cfg(test)]
pub(crate) fn use_test_data_dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("launcher-tests-{}", std::process::id()));
        std::env::set_var("LAUNCHER_DATA_DIR", &d);
        d
    })
    .clone()
}

/// A folder under app data, created if missing.
pub fn app_data_subdir(name: &str) -> Option<PathBuf> {
    let d = app_data_dir()?.join(name);
    std::fs::create_dir_all(&d).ok()?;
    Some(d)
}

/// FNV-1a 64. Not std's `DefaultHasher`, which may change between Rust releases and would orphan
/// every state file.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// The resolved full path of a config as the key its state, cache, lock and run history hang off:
/// canonical where possible, without the `\\?\` prefix, and lowercased on Windows (whose file
/// system is case-insensitive).
pub fn config_key(config: &Path) -> String {
    let full = std::fs::canonicalize(config).unwrap_or_else(|_| normalize(&absolute(config)));
    let s = full.display().to_string();
    let s = s.strip_prefix(r"\\?\").unwrap_or(&s).to_string();
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

/// 16 hex digits of the config key's hash.
pub fn config_hash(config: &Path) -> String {
    format!("{:016x}", fnv1a64(config_key(config).as_bytes()))
}

/// The config's file stem, reduced to characters safe in a file name (for state file names that a
/// person can still recognize).
pub fn config_stem(config: &Path) -> String {
    let stem = config
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".into());
    stem.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(40)
        .collect()
}

/// `p` made absolute against the current directory (only for keys and display; configs never
/// resolve their own paths against it).
pub fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|d| d.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    }
}

/// Resolve `p` against `base` (the config's folder) and tidy `.`/`..` lexically. Absolute inputs
/// are only tidied.
pub fn resolve(base: &Path, p: &str) -> PathBuf {
    let raw = Path::new(p);
    let joined = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        base.join(raw)
    };
    normalize(&joined)
}

/// Lexical normalization: drops `.` and applies `..` without touching the file system (so it
/// works for paths that do not exist yet and never resolves symlinks).
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// `p` relative to `base` when it is inside it (for display), else `p` unchanged.
pub fn display_relative(base: &Path, p: &Path) -> String {
    match p.strip_prefix(base) {
        Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
        _ => p.display().to_string(),
    }
}

/// The extensions a list names, lower case and without the dot: `json, .FBX` or `*.json;*.fbx`
/// gives `json`, `fbx`. Also takes the older `Name|*.a;*.b|Name|*.c` form (its patterns only).
/// A `*.*` anywhere allows any file: no extensions.
pub fn extensions(list: &str) -> Vec<String> {
    let pats = if list.contains('|') {
        list.split('|')
            .skip(1)
            .step_by(2)
            .collect::<Vec<_>>()
            .join(";")
    } else {
        list.to_string()
    };
    let mut out: Vec<String> = Vec::new();
    for p in pats.split([',', ';', ' ', '\t']) {
        let e = p.trim().trim_start_matches('*').trim_start_matches('.');
        let e = e.to_lowercase();
        if e == "*" {
            return Vec::new();
        }
        if !e.is_empty() && !out.contains(&e) {
            out.push(e);
        }
    }
    out
}

/// The kinds of file a list allows, as a file dialog groups them: (name, extensions). The
/// older `Name|*.a;*.b` form keeps its names; a plain list is one group, "JSON or FBX files".
pub fn file_types(list: &str) -> Vec<(String, Vec<String>)> {
    if list.contains('|') {
        let parts: Vec<&str> = list.split('|').collect();
        return parts
            .chunks(2)
            .filter_map(|pair| match pair {
                [name, pats] => {
                    let exts = extensions(pats);
                    (!exts.is_empty()).then(|| (name.trim().to_string(), exts))
                }
                _ => None,
            })
            .collect();
    }
    let exts = extensions(list);
    if exts.is_empty() {
        return Vec::new();
    }
    vec![(
        format!("{} files", or_list(&exts, |e| e.to_uppercase())),
        exts,
    )]
}

/// Whether a path has one of the extensions (any, when there are none). Ignores case.
pub fn has_extension(path: &str, exts: &[String]) -> bool {
    exts.is_empty()
        || Path::new(path.trim())
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// Extensions as people read them: ".json", ".json or .fbx", ".json, .fbx or .obj".
pub fn extension_words(exts: &[String]) -> String {
    or_list(exts, |e| format!(".{e}"))
}

fn or_list(items: &[String], f: impl Fn(&str) -> String) -> String {
    let words: Vec<String> = items.iter().map(|i| f(i)).collect();
    match words.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} or {last}", rest.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_lists_read_both_ways() {
        assert_eq!(extensions("json, .FBX"), ["json", "fbx"]);
        assert_eq!(extensions("*.json;*.fbx json"), ["json", "fbx"]);
        assert_eq!(extensions("Programs|*.exe;*.bat"), ["exe", "bat"]);
        assert!(extensions("Programs|*.exe;*.bat|All|*.*").is_empty());
        assert!(extensions("").is_empty());
        assert_eq!(
            file_types("json, fbx"),
            [(
                "JSON or FBX files".to_string(),
                vec!["json".into(), "fbx".into()]
            )]
        );
        assert_eq!(
            file_types("Programs|*.exe;*.bat|Scripts|*.ps1"),
            [
                ("Programs".to_string(), vec!["exe".into(), "bat".into()]),
                ("Scripts".to_string(), vec!["ps1".into()]),
            ]
        );
        assert!(file_types("All|*.*").is_empty());
        let json = vec!["json".to_string()];
        assert!(has_extension("a/b.JSON", &json));
        assert!(!has_extension("a/b.fbx", &json));
        assert!(has_extension("a/b.fbx", &[]));
        assert_eq!(
            extension_words(&["json".into(), "fbx".into(), "obj".into()]),
            ".json, .fbx or .obj"
        );
    }

    #[test]
    fn resolves_relative_to_the_config_folder() {
        if cfg!(windows) {
            let base = Path::new(r"C:\builds\game");
            assert_eq!(
                resolve(base, r".\config.json"),
                PathBuf::from(r"C:\builds\game\config.json")
            );
            assert_eq!(
                resolve(base, r"..\Videos"),
                PathBuf::from(r"C:\builds\Videos")
            );
            assert_eq!(resolve(base, r"D:\x\y.exe"), PathBuf::from(r"D:\x\y.exe"));
        } else {
            let base = Path::new("/builds/game");
            assert_eq!(
                resolve(base, "./config.json"),
                PathBuf::from("/builds/game/config.json")
            );
            assert_eq!(resolve(base, "../x"), PathBuf::from("/builds/x"));
        }
    }

    #[test]
    fn the_hash_is_stable() {
        // Pinned: a change here orphans every user's state file.
        assert_eq!(fnv1a64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(
            fnv1a64(b"c:\\builds\\game\\launcher.kdl"),
            0x750b_db7e_5937_bde8
        );
    }

    #[test]
    fn keys_are_case_insensitive_on_windows() {
        let a = config_key(Path::new("Some/Dir/Build.launcher"));
        let b = config_key(Path::new("some/dir/build.launcher"));
        assert_eq!(a == b, cfg!(windows));
        assert_eq!(config_stem(Path::new("x/My Build.launcher")), "My_Build");
    }
}
