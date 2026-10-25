//! Printing a tool the one way the editor writes it (N24: the editor owns the file): four-space
//! indents, one node per line, a blank line between the parts of the file, and every value as it
//! was written (a script stays a readable raw string). A comment just above a node is its note
//! and is kept; a stray comment elsewhere may be dropped.
//!
//! Printing is stable: printing a printed file gives the same text, so saving an unchanged tool
//! changes nothing in git.

use kdl::{FormatConfig, KdlDocument, KdlEntryFormat, KdlValue};

/// A string as KDL writes it in quotes.
fn quoted(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Every string value in quotes (KDL would also allow some bare; the editor's files don't), and
/// raw strings (scripts) kept as written.
fn quote_strings(d: &mut KdlDocument) {
    for n in d.nodes_mut() {
        for e in n.entries_mut() {
            let KdlValue::String(s) = e.value() else {
                continue;
            };
            let kept = e
                .format()
                .is_some_and(|f| f.value_repr.starts_with(['"', '#']));
            if !kept {
                let repr = quoted(s);
                e.set_format(KdlEntryFormat {
                    value_repr: repr,
                    leading: " ".into(),
                    autoformat_keep: true,
                    ..Default::default()
                });
            }
        }
        if let Some(c) = n.children_mut().as_mut() {
            quote_strings(c);
        }
    }
}

/// The file's text, in the editor's layout.
pub fn print(doc: &KdlDocument) -> String {
    let mut d = doc.clone();
    quote_strings(&mut d);
    let mut config = FormatConfig::default();
    config.entry_autoformate_keep = true;
    d.autoformat_config(&config);
    // A blank line between the parts of the file (tool, window, each set of steps...).
    for (i, n) in d.nodes_mut().iter_mut().enumerate() {
        if i == 0 {
            continue;
        }
        if let Some(f) = n.format_mut() {
            if !f.leading.starts_with('\n') {
                f.leading.insert(0, '\n');
            }
        }
    }
    let mut out = d.to_string();
    // No trailing spaces, no blank line inside an empty block, exactly one newline at the end.
    let lines: Vec<&str> = out.lines().map(str::trim_end).collect();
    let mut kept: Vec<&str> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        let empty_block = l.is_empty()
            && kept.last().is_some_and(|p| p.ends_with('{'))
            && lines.get(i + 1).is_some_and(|n| n.trim_start() == "}");
        if !empty_block {
            kept.push(l);
        }
    }
    out = kept.join("\n");
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}

/// Parse and print: the canonical form of a tool's text (None when it isn't valid KDL).
pub fn tidy(src: &str) -> Option<String> {
    KdlDocument::parse_v2(src).ok().map(|d| print(&d))
}

/// Whether a tool's text is already in the editor's layout (else opening it says "Saving will
/// tidy this file's layout").
pub fn is_tidy(src: &str) -> bool {
    tidy(src).is_some_and(|t| t == src.replace("\r\n", "\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MESSY: &str = r##"// Builds the game.
tool "Build" format=2
window   {
  // Which branch.
  choice "branch" label="Branch"   {
        list-from "git.branches" folder="."
  }
  button "go" label="Go"
}
steps "go" {
git.pull "pull"
    core.script "s" code=#"""
        log("hi")
        """#
}
"##;

    #[test]
    fn printing_is_stable_and_keeps_notes_and_scripts() {
        let once = tidy(MESSY).unwrap();
        assert_eq!(
            once,
            r##"// Builds the game.
tool "Build" format=2

window {
    // Which branch.
    choice "branch" label="Branch" {
        list-from "git.branches" folder="."
    }
    button "go" label="Go"
}

steps "go" {
    git.pull "pull"
    core.script "s" code=#"""
        log("hi")
        """#
}
"##
        );
        assert_eq!(tidy(&once).unwrap(), once);
        assert!(is_tidy(&once));
        assert!(!is_tidy(MESSY));
        // An empty block has no blank line inside.
        let mut d = KdlDocument::parse_v2("tool \"T\"\nsteps \"go\" {\n}\n").unwrap();
        d.nodes_mut()[1].ensure_children();
        assert_eq!(print(&d), "tool \"T\"\n\nsteps \"go\" {\n}\n");
    }
}
