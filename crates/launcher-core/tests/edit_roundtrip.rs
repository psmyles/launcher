//! The editor's splices leave everything they don't touch byte-for-byte (N12). On the example
//! files: every node duplicated and the copy removed gives the file back unchanged, comments and
//! all; every node moved down and back keeps every line. A scripted session ends in a golden file.

use std::path::Path;

use kdl::KdlDocument;
use launcher_core::config::catalog::Os;
use launcher_core::config::edit::{self, Repr};
use launcher_core::config::{load_str, LoadOptions};

fn examples() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    [
        "git-build/git-build-windows.launcher",
        "git-build/git-build-macos.launcher",
        "unity-build/unity-build-windows.launcher",
        "unity-build/unity-build-macos.launcher",
    ]
    .iter()
    .map(|f| {
        (
            f.to_string(),
            std::fs::read_to_string(root.join(f)).unwrap(),
        )
    })
    .collect()
}

/// Every node's path, depth first.
fn paths(doc: &KdlDocument) -> Vec<Vec<usize>> {
    fn walk(nodes: &[kdl::KdlNode], prefix: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        for (i, n) in nodes.iter().enumerate() {
            prefix.push(i);
            out.push(prefix.clone());
            if let Some(c) = n.children() {
                walk(c.nodes(), prefix, out);
            }
            prefix.pop();
        }
    }
    let mut out = Vec::new();
    walk(doc.nodes(), &mut Vec::new(), &mut out);
    out
}

fn non_blank(t: &str) -> Vec<&str> {
    t.lines().filter(|l| !l.trim().is_empty()).collect()
}

#[test]
fn duplicating_then_removing_leaves_the_file_as_it_was() {
    for (name, text) in examples() {
        let doc = KdlDocument::parse_v2(&text).unwrap();
        for p in paths(&doc) {
            let (with_copy, copy) =
                edit::duplicate_node(&text, &p).unwrap_or_else(|e| panic!("{name} {p:?}: {e}"));
            let back = edit::remove_node(&with_copy, &copy).unwrap();
            assert_eq!(back, text, "{name}: duplicate + remove of {p:?}");
        }
    }
}

#[test]
fn moving_down_and_back_leaves_the_file_as_it_was() {
    for (name, text) in examples() {
        let doc = KdlDocument::parse_v2(&text).unwrap();
        for p in paths(&doc) {
            let (parent, last) = p.split_at(p.len() - 1);
            let parent = (!parent.is_empty()).then_some(parent);
            let siblings = match parent {
                None => doc.nodes().len(),
                Some(pp) => edit::node_at(&doc, pp)
                    .and_then(|n| n.children())
                    .map_or(0, |c| c.nodes().len()),
            };
            if last[0] + 1 >= siblings {
                continue;
            }
            // Down one (index counted before the removal), then back up.
            let (down, at) = edit::move_node(&text, &p, parent, last[0] + 2).unwrap();
            let (back, _) = edit::move_node(&down, &at, parent, last[0]).unwrap();
            // A moved block takes its notes but not the blank lines around it, and a comment that
            // belongs to no node (blank lines on both sides) may end up on its other side. No
            // line is lost or doubled.
            let (mut a, mut b) = (non_blank(&back), non_blank(&text));
            a.sort_unstable();
            b.sort_unstable();
            assert_eq!(a, b, "{name}: move of {p:?} down and back");
        }
    }
}

#[test]
fn a_scripted_session_matches_its_golden_file() {
    let (_, text) = examples()
        .into_iter()
        .find(|(n, _)| n.contains("git-build-windows"))
        .unwrap();
    // Relabel the build type, give it a note, add a control after it, and drop the check before
    // starting.
    let mut t = edit::set_prop(&text, &[1, 1], "label", Some(Repr::Str("Configuration"))).unwrap();
    t = edit::set_notes(&t, &[1, 1], "Debug or Release.").unwrap();
    let (t2, _) =
        edit::insert_node(&t, Some(&[1]), 2, "tick-box \"verbose\" label=\"Verbose\"").unwrap();
    t = edit::remove_node(&t2, &[2, 0]).unwrap();
    // The tool is for Windows: load it as Windows would, wherever the test runs.
    let opts = LoadOptions {
        os: Os::Windows,
        ..LoadOptions::default()
    };
    let l = load_str(&t, Path::new("git-build.launcher"), &opts);
    assert!(!l.has_errors(), "{:?}", l.diags);
    let golden = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/edits/git-build-session.launcher");
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).unwrap();
        std::fs::write(&golden, &t).unwrap();
    }
    let want = std::fs::read_to_string(&golden).unwrap_or_default();
    assert_eq!(
        t, want,
        "the golden file differs; rerun with UPDATE_GOLDENS=1 after checking the diff"
    );
}
