//! Format-preserving edits for the visual editor (N12; ported from master).
//!
//! The editor never prints the document: every edit is a **splice of the original text** at the
//! byte ranges the parser reported, followed by a fresh parse. Everything the edit does not touch
//! (comments, blank lines, alignment, quoting style) stays byte-for-byte as the author left it.
//! This also sidesteps a kdl 6.7 bug: re-serializing drops a `// comment` that follows a `}` on
//! the same line (LESSONS).
//!
//! Every function takes the current text and a [`NodePath`] and returns the new text (and, for
//! inserts and moves, the node's new path). A node's *block* is its lines plus the `//` comment
//! lines directly above it (its notes), so moving or deleting a node carries its notes with it.

use kdl::{KdlDocument, KdlNode};

use super::model::NodePath;

/// The node at an index path from the document root.
pub fn node_at<'a>(doc: &'a KdlDocument, path: &[usize]) -> Option<&'a KdlNode> {
    let (first, rest) = path.split_first()?;
    let mut n = doc.nodes().get(*first)?;
    for i in rest {
        n = n.children()?.nodes().get(*i)?;
    }
    Some(n)
}

pub type EditResult<T> = Result<T, String>;

fn parse(src: &str) -> EditResult<KdlDocument> {
    KdlDocument::parse_v2(src)
        .map_err(|_| "the file is not valid KDL; fix it in the Raw tab first".to_string())
}

fn node<'a>(doc: &'a KdlDocument, path: &[usize]) -> EditResult<&'a KdlNode> {
    node_at(doc, path).ok_or_else(|| format!("no node at {path:?}"))
}

/// Check the result parses (an edit must never produce a broken file).
fn checked(out: String) -> EditResult<String> {
    parse(&out)?;
    Ok(out)
}

// ---- value text ---------------------------------------------------------------------------------

/// KDL text for a string: plain `"..."` when possible, a raw `#"..."#` when it holds `\` or `"`
/// (Windows paths and JSON read as written), escaped `"..."` when it holds line breaks.
pub fn string_repr(s: &str) -> String {
    let needs_escape = s.chars().any(|c| c.is_control());
    if !needs_escape && (s.contains('\\') || s.contains('"')) {
        let mut hashes = 1;
        while s.contains(&format!("\"{}", "#".repeat(hashes))) {
            hashes += 1;
        }
        let h = "#".repeat(hashes);
        return format!("{h}\"{s}\"{h}");
    }
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
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

/// KDL text for a typed value the editor produces.
pub enum Repr<'a> {
    Str(&'a str),
    Bool(bool),
    Num(f64),
    /// Already KDL (e.g. an integer typed as text).
    Raw(&'a str),
}

impl Repr<'_> {
    pub fn text(&self) -> String {
        match self {
            Repr::Str(s) => string_repr(s),
            Repr::Bool(b) => if *b { "#true" } else { "#false" }.to_string(),
            Repr::Num(f) => crate::dynval::fmt_num(*f),
            Repr::Raw(s) => s.to_string(),
        }
    }
}

/// A node name or property key as KDL text: bare when it is a valid identifier, quoted otherwise.
pub fn ident_repr(s: &str) -> String {
    let bare = !s.is_empty()
        && !s.chars().next().is_some_and(|c| c.is_ascii_digit())
        && s.chars().all(|c| {
            c.is_alphanumeric()
                || matches!(
                    c,
                    '-' | '_'
                        | '.'
                        | '/'
                        | ':'
                        | '@'
                        | '!'
                        | '$'
                        | '%'
                        | '^'
                        | '&'
                        | '*'
                        | '+'
                        | '<'
                        | '>'
                        | '?'
                        | '|'
                        | '~'
                )
        })
        && !matches!(s, "true" | "false" | "null" | "inf" | "-inf" | "nan");
    if bare {
        s.to_string()
    } else {
        string_repr(s)
    }
}

// ---- lines ------------------------------------------------------------------------------------------

fn line_start(src: &str, at: usize) -> usize {
    src[..at].rfind('\n').map_or(0, |i| i + 1)
}

/// Offset just past the line's `\n` (or the end of the text).
fn line_end_incl(src: &str, at: usize) -> usize {
    src[at..].find('\n').map_or(src.len(), |i| at + i + 1)
}

fn indent_of(src: &str, at: usize) -> String {
    let ls = line_start(src, at);
    src[ls..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// The line break style of the file.
fn newline(src: &str) -> &'static str {
    if src.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// Is the node alone on its lines (only whitespace before it, and only whitespace, `;` or a
/// comment after it)?
fn alone(src: &str, start: usize, end: usize) -> bool {
    let before = &src[line_start(src, start)..start];
    let le = line_end_incl(src, end);
    let after = src[end..le].trim();
    let after = after.trim_start_matches(';').trim();
    before.trim().is_empty() && (after.is_empty() || after.starts_with("//"))
}

/// Byte range of a node's block: its full lines plus the comment lines directly above it.
/// For a node sharing its line with others, just the node and a following `;`.
fn block(src: &str, n: &KdlNode) -> (usize, usize) {
    let start = n.span().offset();
    let end = start + n.span().len();
    if !alone(src, start, end) {
        let mut e = end;
        let rest = &src[e..];
        let skip = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        if rest[skip..].starts_with(';') {
            e += skip + 1;
        }
        let pre = &src[..start];
        let s = start - (pre.len() - pre.trim_end_matches([' ', '\t']).len());
        return (s, e);
    }
    let mut s = line_start(src, start);
    // Attached notes: comment lines right above (no blank line in between).
    while s > 0 {
        let prev_start = line_start(src, s - 1);
        let line = src[prev_start..s].trim();
        if line.starts_with("//") {
            s = prev_start;
        } else {
            break;
        }
    }
    (s, line_end_incl(src, end))
}

// ---- properties and arguments ----------------------------------------------------------------------

/// Set (`Some`) or remove (`None`) property `name` of the node at `path`.
pub fn set_prop(src: &str, path: &[usize], name: &str, value: Option<Repr>) -> EditResult<String> {
    let doc = parse(src)?;
    let n = node(&doc, path)?;
    let existing: Vec<_> = n
        .entries()
        .iter()
        .filter(|e| e.name().is_some_and(|k| k.value() == name))
        .collect();
    let mut out = src.to_string();
    match (existing.last(), value) {
        (Some(e), Some(v)) => {
            let sp = e.span();
            out.replace_range(
                sp.offset()..sp.offset() + sp.len(),
                &format!("{}={}", ident_repr(name), v.text()),
            );
        }
        (Some(_), None) => {
            // Remove every occurrence (KDL allows repeats; the last one wins).
            let mut ranges: Vec<(usize, usize)> = existing
                .iter()
                .map(|e| {
                    let s = e.span().offset();
                    let pre = &src[..s];
                    let ws = pre.len() - pre.trim_end_matches([' ', '\t']).len();
                    (s - ws, s + e.span().len())
                })
                .collect();
            ranges.sort_by_key(|r| std::cmp::Reverse(r.0));
            for (a, b) in ranges {
                out.replace_range(a..b, "");
            }
        }
        (None, Some(v)) => {
            let at = entries_end(n);
            out.insert_str(at, &format!(" {}={}", ident_repr(name), v.text()));
        }
        (None, None) => return Ok(out),
    }
    checked(out)
}

/// Where new entries go: after the last entry, or after the name (and type annotation).
fn entries_end(n: &KdlNode) -> usize {
    let name_end = n.name().span().offset() + n.name().span().len();
    n.entries()
        .iter()
        .map(|e| e.span().offset() + e.span().len())
        .max()
        .unwrap_or(name_end)
        .max(name_end)
}

/// Set (`Some`) or remove (`None`) positional argument `index` of the node at `path`. Setting
/// one past the end appends it (after the other arguments, before the properties).
pub fn set_arg(src: &str, path: &[usize], index: usize, value: Option<Repr>) -> EditResult<String> {
    let doc = parse(src)?;
    let n = node(&doc, path)?;
    let args: Vec<_> = n.entries().iter().filter(|e| e.name().is_none()).collect();
    let mut out = src.to_string();
    match (args.get(index), value) {
        (Some(e), Some(v)) => {
            let sp = e.span();
            out.replace_range(sp.offset()..sp.offset() + sp.len(), &v.text());
        }
        (Some(e), None) => {
            let s = e.span().offset();
            let pre = &src[..s];
            let ws = pre.len() - pre.trim_end_matches([' ', '\t']).len();
            out.replace_range(s - ws..s + e.span().len(), "");
        }
        (None, Some(v)) => {
            if index > args.len() {
                return Err(format!("argument {index} would leave a gap"));
            }
            let at = args
                .last()
                .map(|e| e.span().offset() + e.span().len())
                .unwrap_or_else(|| n.name().span().offset() + n.name().span().len());
            out.insert_str(at, &format!(" {}", v.text()));
        }
        (None, None) => return Ok(out),
    }
    checked(out)
}

/// Change a node's name (a control's type, a data key).
pub fn rename_node(src: &str, path: &[usize], new_name: &str) -> EditResult<String> {
    let doc = parse(src)?;
    let n = node(&doc, path)?;
    let sp = n.name().span();
    let mut out = src.to_string();
    out.replace_range(sp.offset()..sp.offset() + sp.len(), &ident_repr(new_name));
    checked(out)
}

// ---- notes (the comment lines above a node) ------------------------------------------------------------

/// The `//` comment lines directly above a node, without the slashes.
pub fn notes(src: &str, path: &[usize]) -> EditResult<String> {
    let doc = parse(src)?;
    let n = node(&doc, path)?;
    let start = n.span().offset();
    if !alone(src, start, start + n.span().len()) {
        return Ok(String::new());
    }
    let (bs, _) = block(src, n);
    let ls = line_start(src, start);
    let lines: Vec<String> = src[bs..ls]
        .lines()
        .map(|l| {
            let t = l.trim_start().trim_start_matches("//");
            t.strip_prefix(' ').unwrap_or(t).trim_end().to_string()
        })
        .collect();
    Ok(lines.join("\n"))
}

/// Replace the notes above a node (empty text removes them).
pub fn set_notes(src: &str, path: &[usize], text: &str) -> EditResult<String> {
    let doc = parse(src)?;
    let n = node(&doc, path)?;
    let start = n.span().offset();
    if !alone(src, start, start + n.span().len()) {
        return Err("notes need the node on its own line".into());
    }
    let (bs, _) = block(src, n);
    let ls = line_start(src, start);
    let indent = indent_of(src, start);
    let nl = newline(src);
    let mut lines = String::new();
    for l in text.lines() {
        if l.trim().is_empty() {
            lines.push_str(&format!("{indent}//{nl}"));
        } else {
            lines.push_str(&format!("{indent}// {}{nl}", l.trim_end()));
        }
    }
    let mut out = src.to_string();
    out.replace_range(bs..ls, &lines);
    checked(out)
}

// ---- structure -----------------------------------------------------------------------------------------

fn children_of<'a>(
    doc: &'a KdlDocument,
    parent: Option<&[usize]>,
) -> EditResult<(&'a [KdlNode], Option<&'a KdlNode>)> {
    match parent {
        None => Ok((doc.nodes(), None)),
        Some(p) => {
            let n = node(doc, p)?;
            Ok((n.children().map_or(&[][..], |c| c.nodes()), Some(n)))
        }
    }
}

/// Indent `text` (a node, possibly with a block and notes, written from column 0) by `indent`.
fn indented(text: &str, indent: &str, nl: &str) -> String {
    let mut out = String::new();
    for l in text.trim_end_matches(['\n', '\r']).lines() {
        if l.trim().is_empty() {
            out.push_str(nl);
        } else {
            out.push_str(indent);
            out.push_str(l.trim_end_matches('\r'));
            out.push_str(nl);
        }
    }
    out
}

/// Strip the common leading indentation of a block of text.
fn dedent(text: &str) -> String {
    let min = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start_matches([' ', '\t']).len())
        .min()
        .unwrap_or(0);
    text.lines()
        .map(|l| {
            if l.len() >= min {
                &l[min..]
            } else {
                l.trim_start()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Insert `node_text` (KDL for one node, written from column 0; it may have a block and leading
/// notes) as child `index` of `parent` (`None` = top level). Returns the text and the new path.
pub fn insert_node(
    src: &str,
    parent: Option<&[usize]>,
    index: usize,
    node_text: &str,
) -> EditResult<(String, NodePath)> {
    let doc = parse(src)?;
    let (kids, pnode) = children_of(&doc, parent)?;
    let index = index.min(kids.len());
    let nl = newline(src);
    let mut out = src.to_string();
    let path_of = |i: usize| -> NodePath {
        let mut p = parent.map(|p| p.to_vec()).unwrap_or_default();
        p.push(i);
        p
    };
    if let Some(sib) = kids.get(index) {
        // Before an existing sibling (and before its notes).
        let start = sib.span().offset();
        if alone(src, start, start + sib.span().len()) {
            let (bs, _) = block(src, sib);
            let indent = indent_of(src, start);
            out.insert_str(bs, &indented(node_text, &indent, nl));
        } else {
            out.insert_str(start, &format!("{}; ", node_text.trim()));
        }
        return Ok((checked(out)?, path_of(index)));
    }
    if let Some(last) = kids.last() {
        // After the last sibling.
        let start = last.span().offset();
        let end = start + last.span().len();
        if alone(src, start, end) {
            let at = line_end_incl(src, end);
            let indent = indent_of(src, start);
            let mut text = indented(node_text, &indent, nl);
            if at == src.len() && !src.ends_with('\n') {
                text = format!("{nl}{text}");
            }
            out.insert_str(at, &text);
        } else {
            out.insert_str(end, &format!("; {}", node_text.trim()));
        }
        return Ok((checked(out)?, path_of(index)));
    }
    match pnode {
        None => {
            // An empty document.
            let mut text = indented(node_text, "", nl);
            if !src.is_empty() && !src.ends_with('\n') {
                text = format!("{nl}{text}");
            }
            out.push_str(&text);
        }
        Some(p) => {
            let pstart = p.span().offset();
            let pend = pstart + p.span().len();
            let pindent = indent_of(src, pstart);
            let child_indent = format!("{pindent}    ");
            if p.children().is_some() {
                // `{ }` or `{\n}` with nothing inside: insert before the closing brace.
                let close = src[..pend].rfind('}').ok_or("no closing brace")?;
                let open = src[pstart..close]
                    .rfind('{')
                    .map(|i| pstart + i)
                    .ok_or("no opening brace")?;
                let inner = format!("{nl}{}{pindent}", indented(node_text, &child_indent, nl));
                out.replace_range(open + 1..close, &inner);
            } else {
                // No block yet: add one after the last entry.
                let at = entries_end(p);
                let text = format!(
                    " {{{nl}{}{pindent}}}",
                    indented(node_text, &child_indent, nl)
                );
                out.insert_str(at, &text);
            }
        }
    }
    Ok((checked(out)?, path_of(index)))
}

/// Remove the node at `path` (with its notes).
pub fn remove_node(src: &str, path: &[usize]) -> EditResult<String> {
    let doc = parse(src)?;
    let n = node(&doc, path)?;
    let (a, b) = block(src, n);
    let mut out = src.to_string();
    out.replace_range(a..b, "");
    checked(out)
}

/// The node's KDL text with its notes, dedented to column 0.
pub fn node_text(src: &str, path: &[usize]) -> EditResult<String> {
    let doc = parse(src)?;
    let n = node(&doc, path)?;
    let start = n.span().offset();
    let end = start + n.span().len();
    if !alone(src, start, end) {
        return Ok(src[start..end].to_string());
    }
    let (a, _) = block(src, n);
    // The block's text up to the node's end (not the rest of its last line: a trailing comment
    // stays with the node, but a following `;` or whitespace does not matter).
    let le = line_end_incl(src, end);
    let text = src[a..le].trim_end_matches(['\n', '\r']);
    Ok(dedent(text))
}

/// Duplicate the node at `path` right after itself. Returns the text and the copy's path.
pub fn duplicate_node(src: &str, path: &[usize]) -> EditResult<(String, NodePath)> {
    let text = node_text(src, path)?;
    let (parent, index) = split(path)?;
    insert_node(src, parent, index + 1, &text)
}

fn split(path: &[usize]) -> EditResult<(Option<&[usize]>, usize)> {
    let (last, parent) = path.split_last().ok_or("empty path")?;
    Ok(((!parent.is_empty()).then_some(parent), *last))
}

/// Move the node at `from` to child `index` of `to_parent` (index counted before the removal).
/// Returns the text and the node's new path.
pub fn move_node(
    src: &str,
    from: &[usize],
    to_parent: Option<&[usize]>,
    index: usize,
) -> EditResult<(String, NodePath)> {
    let text = node_text(src, from)?;
    let to: Vec<usize> = to_parent.map(|p| p.to_vec()).unwrap_or_default();
    if to.len() >= from.len() && to[..from.len()] == *from {
        return Err("a node cannot move inside itself".into());
    }
    let (from_parent, from_index) = split(from)?;
    // Insert first (paths before the moved node are unaffected), then remove the original,
    // whose index may have shifted by the insert.
    let (with_copy, new_path) = insert_node(src, to_parent, index, &text)?;
    let same_parent = from_parent.map(|p| p.to_vec()) == to_parent.map(|p| p.to_vec());
    let mut old = from.to_vec();
    if same_parent && index <= from_index {
        let last = old.len() - 1;
        old[last] += 1;
    } else if !same_parent
        && to.len() < from.len()
        && from[..to.len()] == to[..]
        && index <= from[to.len()]
    {
        // The destination is an ancestor's child list at or before the original's branch.
        old[to.len()] += 1;
    }
    let out = remove_node(&with_copy, &old)?;
    // Removing the original shifts the new path if it came after it in the same list.
    let mut np = new_path;
    let fp: Vec<usize> = from.to_vec();
    if np.len() >= fp.len()
        && np[..fp.len() - 1] == fp[..fp.len() - 1]
        && np[fp.len() - 1] > fp[fp.len() - 1]
    {
        let k = fp.len() - 1;
        np[k] -= 1;
    }
    Ok((out, np))
}

// ---- new nodes ------------------------------------------------------------------------------------------

/// A placeholder for a required parameter the user fills in: an existing id for a reference
/// (`pick`), else the parameter's name.
fn placeholder(
    p: &super::catalog::Param,
    id: &str,
    pick: &dyn Fn(super::catalog::Ty) -> Option<String>,
) -> String {
    use super::catalog::Ty;
    if let Some(r) = pick(p.ty) {
        return string_repr(&r);
    }
    match p.ty {
        Ty::Id => string_repr(id),
        Ty::Bool => "#false".into(),
        Ty::Int | Ty::Number => "0".into(),
        Ty::Enum(words) => string_repr(words.first().copied().unwrap_or("")),
        Ty::Version => string_repr("0.1"),
        Ty::Lua => string_repr("true"),
        Ty::LuaChunk => string_repr("-- Lua"),
        Ty::LuaValue => format!("(lua){}", string_repr("{}")),
        _ => string_repr(p.name),
    }
}

/// An id for a new node of `kind` that `taken` doesn't have: `dropdown1`, `dropdown2`...
pub fn fresh_id(kind: &str, taken: &std::collections::BTreeSet<String>) -> String {
    let base = kind.replace('-', "_");
    (1..)
        .map(|n| format!("{base}{n}"))
        .find(|id| !taken.contains(id))
        .unwrap_or(base)
}

/// KDL for a new node of `kind` (the editor's Add): its required arguments and properties as
/// placeholders, a label for controls and containers that show one, and the children it can't
/// be useful without (two options in a list, a tab, two columns). `pick` offers an existing id
/// for a reference (a control, an action, a data block, a run step).
pub fn new_node_text(
    kind: &str,
    taken: &std::collections::BTreeSet<String>,
    pick: &dyn Fn(super::catalog::Ty) -> Option<String>,
) -> Option<String> {
    use super::catalog::{self, Category};
    let spec = catalog::get(kind)?;
    let id = fresh_id(kind, taken);
    let mut s = ident_repr(kind);
    for a in spec.args.iter().filter(|a| a.required) {
        s.push(' ');
        s.push_str(&placeholder(a, &id, pick));
    }
    // A computed value needs its value.
    if kind == "value" {
        s.push_str(" \"\"");
    }
    for p in spec.all_props().filter(|p| p.required) {
        s.push_str(&format!(
            " {}={}",
            ident_repr(p.name),
            placeholder(p, &id, pick)
        ));
    }
    let title = {
        let mut c = kind.chars();
        c.next()
            .map(|f| f.to_uppercase().collect::<String>() + c.as_str())
            .unwrap_or_default()
            .replace('-', " ")
    };
    let labelled = spec.cat == Category::Control || kind == "action";
    if labelled
        && spec.prop("label").is_some()
        && !spec.all_props().any(|p| p.required && p.name == "label")
    {
        s.push_str(&format!(" label={}", string_repr(&title)));
    }
    let children = match kind {
        "dropdown" | "radio" | "checklist" => "\n    option \"A\"\n    option \"B\"\n",
        "tabs" => "\n    tab \"Tab\" {\n    }\n",
        "columns" => "\n    column {\n    }\n    column {\n    }\n",
        _ => "",
    };
    if !children.is_empty() {
        s.push_str(&format!(" {{{children}}}"));
    }
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a kind is added: its parent's text, and the path to insert under.
    fn home_of(kind: &str) -> (&'static str, Option<Vec<usize>>) {
        match kind {
            "tab" => ("layout {\n    tabs {\n    }\n}\n", Some(vec![0, 0])),
            "column" => ("layout {\n    columns {\n    }\n}\n", Some(vec![0, 0])),
            "option" | "preset" | "on-change" => (
                "layout {\n    dropdown \"d\" label=\"D\" {\n        option \"x\"\n    }\n}\n",
                Some(vec![0, 0]),
            ),
            "env" => ("action \"a\" {\n    run \"git\"\n}\n", Some(vec![0, 0])),
            "list-from" => (
                "layout {\n    dropdown \"d\" label=\"D\"\n}\n",
                Some(vec![0, 0]),
            ),
            "with" => (
                "action \"a\" {\n    use \"git.pull\"\n}\n",
                Some(vec![0, 0]),
            ),
            "extract" | "header" => (
                "action \"a\" {\n    http-get \"https://example.com\" id=\"h\"\n}\n",
                Some(vec![0, 0]),
            ),
            "format" | "replace" | "dedupe" | "raw" | "pair" | "pairs" => (
                "layout {\n    text \"t\" label=\"T\"\n}\naction \"a\" {\n    args \"x\"\n}\n",
                Some(vec![1, 0]),
            ),
            _ => ("", None),
        }
    }

    #[test]
    fn every_kind_can_be_added_and_loads() {
        use crate::config::catalog::{Category, SPECS};
        use crate::config::{load_str, LoadOptions};
        let mut failed = Vec::new();
        for spec in SPECS {
            let taken = Default::default();
            let text = new_node_text(spec.name, &taken, &|_| None).unwrap();
            let (base, parent) = match (spec.cat, home_of(spec.name)) {
                (_, (b, Some(p))) => (b.to_string(), Some(p)),
                (Category::Top, _) => (String::new(), None),
                (Category::Layout | Category::Control, _) => {
                    ("layout {\n}\n".into(), Some(vec![0]))
                }
                (Category::Step, _) => ("action \"a\" {\n}\n".into(), Some(vec![0])),
                (Category::Part, _) => panic!("no home for {}", spec.name),
            };
            // The one-per-file nodes stand alone.
            let (out, _) = insert_node(&base, parent.as_deref(), usize::MAX, &text)
                .unwrap_or_else(|e| panic!("{}: {e}", spec.name));
            // Windows, so the Windows-only kinds load too.
            let l = load_str(
                &out,
                std::path::Path::new("x.launcher"),
                &LoadOptions {
                    os: crate::config::catalog::Os::Windows,
                    ..LoadOptions::default()
                },
            );
            let errors: Vec<String> = l
                .diags
                .iter()
                // With nothing to refer to, a reference stays a placeholder to fill in.
                .filter(|d| d.is_error() && !d.code.starts_with("unknown-"))
                .map(|d| d.message.clone())
                .collect();
            if !errors.is_empty() {
                failed.push(format!("{}: {text}: {errors:?}", spec.name));
            }
        }
        assert!(failed.is_empty(), "{failed:#?}");
    }

    const SRC: &str = "\
launcher version=1

// The window.
window title=\"A\"

layout {
    // Pick one.
    dropdown \"role\" label=\"Role\" {
        option \"P1\" {
            SeatId \"1\"
        }
        option \"P2\" { SeatId \"2\"; }   // inline
    }
    checkbox \"x\"
}
";

    #[test]
    fn set_prop_touches_one_entry() {
        let out = set_prop(SRC, &[2, 0], "label", Some(Repr::Str("Your role"))).unwrap();
        assert_eq!(out, SRC.replace("label=\"Role\"", "label=\"Your role\""));
        let out = set_prop(SRC, &[2, 1], "default", Some(Repr::Bool(true))).unwrap();
        assert!(out.contains("checkbox \"x\" default=#true\n"));
        let out = set_prop(SRC, &[2, 0], "label", None).unwrap();
        assert!(out.contains("    dropdown \"role\" {\n"));
    }

    #[test]
    fn a_new_property_lands_before_the_block() {
        let out = set_prop(SRC, &[2, 0], "width", Some(Repr::Num(150.0))).unwrap();
        assert!(out.contains("dropdown \"role\" label=\"Role\" width=150 {"));
    }

    #[test]
    fn args() {
        let out = set_arg(SRC, &[2, 0, 0], 0, Some(Repr::Str("Player 1"))).unwrap();
        assert!(out.contains("option \"Player 1\" {"));
        let out = set_arg(SRC, &[2, 1], 0, Some(Repr::Str(r"C:\x"))).unwrap();
        assert!(out.contains(r##"checkbox #"C:\x"#"##), "{out}");
    }

    #[test]
    fn insert_keeps_indentation_and_comments() {
        let (out, p) = insert_node(SRC, Some(&[2]), 1, "text \"t\" label=\"T\"").unwrap();
        assert_eq!(p, vec![2, 1]);
        assert!(
            out.contains("    }\n    text \"t\" label=\"T\"\n    checkbox \"x\"\n"),
            "{out}"
        );
        // Before the first child: above its notes.
        let (out, _) = insert_node(SRC, Some(&[2]), 0, "separator").unwrap();
        assert!(
            out.contains("layout {\n    separator\n    // Pick one.\n"),
            "{out}"
        );
        // Into a node without a block.
        let (out, p) =
            insert_node(SRC, Some(&[2, 1]), 0, "on-change {\n    reset \"role\"\n}").unwrap();
        assert_eq!(p, vec![2, 1, 0]);
        assert!(out.contains("    checkbox \"x\" {\n        on-change {\n            reset \"role\"\n        }\n    }\n"), "{out}");
        // Into an inline block.
        let (out, _) = insert_node(SRC, Some(&[2, 0, 1]), 1, "RoomId \"4\"").unwrap();
        assert!(out.contains("SeatId \"2\"; RoomId \"4\"; }"), "{out}");
    }

    #[test]
    fn remove_takes_notes_and_keeps_neighbours() {
        let out = remove_node(SRC, &[2, 0]).unwrap();
        assert_eq!(out, "launcher version=1\n\n// The window.\nwindow title=\"A\"\n\nlayout {\n    checkbox \"x\"\n}\n");
        let out = remove_node(SRC, &[2, 0, 1, 0]).unwrap();
        assert!(out.contains("option \"P2\" { }   // inline"), "{out}");
    }

    #[test]
    fn move_and_duplicate() {
        let (out, p) = move_node(SRC, &[2, 1], Some(&[2]), 0).unwrap();
        assert_eq!(p, vec![2, 0]);
        assert!(
            out.contains("layout {\n    checkbox \"x\"\n    // Pick one.\n    dropdown"),
            "{out}"
        );
        let (out, p) = move_node(SRC, &[2, 0, 0], Some(&[2, 0]), 2).unwrap();
        assert_eq!(p, vec![2, 0, 1]);
        let doc = KdlDocument::parse_v2(&out).unwrap();
        let names: Vec<String> = node_at(&doc, &[2, 0])
            .unwrap()
            .children()
            .unwrap()
            .nodes()
            .iter()
            .map(|n| n.entries()[0].value().as_string().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["P2", "P1"]);
        let (out, p) = duplicate_node(SRC, &[2, 0, 0]).unwrap();
        assert_eq!(p, vec![2, 0, 1]);
        assert_eq!(out.matches("option \"P1\"").count(), 2);
        assert!(move_node(SRC, &[2], Some(&[2, 0]), 0).is_err());
    }

    #[test]
    fn notes_round_trip() {
        assert_eq!(notes(SRC, &[2, 0]).unwrap(), "Pick one.");
        let out = set_notes(SRC, &[2, 0], "Line one\nLine two").unwrap();
        assert!(
            out.contains("    // Line one\n    // Line two\n    dropdown"),
            "{out}"
        );
        let out = set_notes(SRC, &[2, 0], "").unwrap();
        assert!(out.contains("layout {\n    dropdown"), "{out}");
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let crlf = SRC.replace('\n', "\r\n");
        let (out, _) = insert_node(&crlf, Some(&[2]), 2, "spacer").unwrap();
        assert!(out.contains("    spacer\r\n"));
        assert!(!out.replace("\r\n", "").contains('\n'));
    }

    #[test]
    fn strings_are_written_readably() {
        assert_eq!(string_repr("plain"), "\"plain\"");
        assert_eq!(string_repr(r"C:\a"), r##"#"C:\a"#"##);
        assert_eq!(
            string_repr(r#"{key}="{value}""#),
            r###"#"{key}="{value}""#"###
        );
        assert_eq!(string_repr("two\nlines"), "\"two\\nlines\"");
        assert_eq!(ident_repr("ServerIP"), "ServerIP");
        assert_eq!(ident_repr("My Key"), "\"My Key\"");
    }
}
