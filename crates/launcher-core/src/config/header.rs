//! The header (`launcher requires="1.2" os="windows"`) is read before anything else (N2, N10): a
//! file written for a newer Launcher may use node kinds this version doesn't know, and a file for
//! the other OS shouldn't produce a page of errors. Either gets one friendly message instead.
//!
//! The header is found by its own small parse (the first node, alone), so it works even when the
//! rest of the file doesn't load.

use kdl::KdlDocument;

use super::catalog::Os;
use super::model::Version;

/// What the header asks for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sniffed {
    pub requires: Option<String>,
    pub os: Option<String>,
    /// Byte offset and length of the `launcher` line.
    pub offset: usize,
    pub len: usize,
}

/// Why the file can't be used here.
#[derive(Clone, Debug, PartialEq)]
pub enum Gate {
    /// `requires` names a newer Launcher.
    TooNew { requires: Version, running: Version },
    /// `os` names the other OS.
    WrongOs { file: Os, running: Os },
}

impl Gate {
    pub fn message(&self) -> String {
        match self {
            Gate::TooNew { requires, running } => format!(
                "this file needs Launcher {requires} or newer, and this is Launcher {running}"
            ),
            Gate::WrongOs { file, .. } => format!(
                "this file is for {}; it can't run on {}",
                os_title(*file),
                os_title(Os::current())
            ),
        }
    }

    pub fn help(&self) -> String {
        match self {
            Gate::TooNew { .. } => "install the latest Launcher, then open the file again".into(),
            Gate::WrongOs { .. } => {
                "open it on the OS it was written for, or ask its author for a version for this one"
                    .into()
            }
        }
    }
}

pub fn os_title(os: Os) -> &'static str {
    match os {
        Os::Windows => "Windows",
        Os::Macos => "macOS",
        Os::Any => "any OS",
    }
}

/// Find the header: the first node of the file, if it is `launcher`.
pub fn sniff(src: &str) -> Option<Sniffed> {
    let start = first_node_start(src)?;
    let rest = &src[start..];
    ["launcher", "tool"].into_iter().find(|w| {
        rest.starts_with(w)
            && !rest[w.len()..]
                .chars()
                .next()
                .is_some_and(|c| !(c.is_whitespace() || c == ';'))
    })?;
    let len = rest.find(['\n', ';']).unwrap_or(rest.len());
    let line = &src[start..start + len];
    let doc = KdlDocument::parse_v2(line).ok()?;
    let node = doc.nodes().first()?;
    let prop = |name: &str| {
        node.get(name)
            .and_then(|v| v.as_string())
            .map(str::to_string)
    };
    Some(Sniffed {
        requires: prop("requires"),
        os: prop("os"),
        offset: start,
        len,
    })
}

/// Byte offset of the first thing that isn't whitespace or a comment.
fn first_node_start(src: &str) -> Option<usize> {
    let mut i = 0;
    let b = src.as_bytes();
    // A byte-order mark is not part of the first node.
    if src.starts_with('\u{feff}') {
        i = 3;
    }
    while i < b.len() {
        match b[i] {
            b' ' | b'\t' | b'\r' | b'\n' => i += 1,
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i = src[i..].find('\n').map_or(b.len(), |n| i + n + 1);
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = src[i + 2..].find("*/").map_or(b.len(), |n| i + 2 + n + 2);
            }
            _ => return Some(i),
        }
    }
    None
}

/// Whether the file can be used by this Launcher on this OS.
pub fn gate(sniffed: &Sniffed, running: Version, os: Os) -> Option<Gate> {
    if let Some(requires) = sniffed.requires.as_deref().and_then(Version::parse) {
        if requires > running {
            return Some(Gate::TooNew { requires, running });
        }
    }
    let file_os = match sniffed.os.as_deref() {
        Some("windows") => Os::Windows,
        Some("macos") => Os::Macos,
        _ => return None,
    };
    (os != Os::Any && file_os != os).then_some(Gate::WrongOs {
        file: file_os,
        running: os,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_header_after_comments() {
        let src = "// A build tool\n/* block */\nlauncher requires=\"1.2\" os=\"macos\"\nwindow\n";
        let h = sniff(src).unwrap();
        assert_eq!(h.requires.as_deref(), Some("1.2"));
        assert_eq!(h.os.as_deref(), Some("macos"));
        assert_eq!(
            &src[h.offset..h.offset + h.len],
            "launcher requires=\"1.2\" os=\"macos\""
        );
        assert!(sniff("window title=\"x\"\nlauncher requires=\"1\"").is_none());
        assert!(sniff("launchers x").is_none());
    }

    #[test]
    fn gates_newer_files_and_other_oses() {
        let h = sniff("launcher requires=\"2.1\"").unwrap();
        assert_eq!(
            gate(&h, Version(1, 4, 0), Os::Windows),
            Some(Gate::TooNew {
                requires: Version(2, 1, 0),
                running: Version(1, 4, 0)
            })
        );
        assert_eq!(gate(&h, Version(2, 1, 0), Os::Windows), None);
        let h = sniff("launcher os=\"macos\"").unwrap();
        assert!(matches!(
            gate(&h, Version(1, 0, 0), Os::Windows),
            Some(Gate::WrongOs { .. })
        ));
        assert_eq!(gate(&h, Version(1, 0, 0), Os::Macos), None);
        // Even when the rest of the file isn't valid KDL.
        assert!(sniff("launcher requires=\"9\"\nlayout { {{{ ").is_some());
    }
}
