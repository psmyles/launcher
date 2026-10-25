//! Diagnostics: every problem in a file, with its place.
//!
//! `build.launcher:57:5: error[unknown-kind]: unknown node `dropdwn` (did you mean `dropdown`?)`

use super::model::NodePath;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    pub help: Option<String>,
    /// Byte offset and length in the file.
    pub offset: usize,
    pub len: usize,
    /// 1-based line and column (in characters).
    pub line: usize,
    pub col: usize,
    /// The node the problem belongs to, when there is one (the editor selects it).
    pub path: Option<NodePath>,
}

impl Diagnostic {
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// The form `launcher check` prints and the Problems list shows.
    pub fn render(&self, file: &str) -> String {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let mut s = format!(
            "{file}:{}:{}: {sev}[{}]: {}",
            self.line, self.col, self.code, self.message
        );
        if let Some(h) = &self.help {
            s.push_str(&format!("\n    help: {h}"));
        }
        s
    }
}

/// Maps byte offsets to line/column.
pub struct LineIndex<'a> {
    src: &'a str,
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub fn new(src: &'a str) -> Self {
        let mut starts = vec![0];
        for (i, b) in src.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        LineIndex { src, starts }
    }

    /// 1-based (line, column in chars).
    pub fn line_col(&self, offset: usize) -> (usize, usize) {
        let offset = offset.min(self.src.len());
        let line = match self.starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let start = self.starts[line];
        let col = self
            .src
            .get(start..offset)
            .map_or(offset - start, |s| s.chars().count());
        (line + 1, col + 1)
    }

    /// Byte offset of the start of the line containing `offset`.
    pub fn line_start(&self, offset: usize) -> usize {
        let offset = offset.min(self.src.len());
        match self.starts.binary_search(&offset) {
            Ok(i) => self.starts[i],
            Err(i) => self.starts[i - 1],
        }
    }
}
