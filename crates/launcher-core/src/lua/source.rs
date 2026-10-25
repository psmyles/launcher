//! Where a chunk of Lua came from: turning "line 3 of this chunk" into "line 17 of the file" (N6:
//! Lua errors report `.launcher` file lines).
//!
//! * A single-line string (`"..."` or `#"..."#`) is on one file line; every chunk line maps to it
//!   (a `\n` escape inside it can't be told apart).
//! * A multi-line string (`"""` or `#"""`) starts its content on the line after the opening quotes,
//!   and KDL v2 removes the closing line's indentation from every line (the dedent): chunk line L
//!   is file line `opening + L`, shifted right by the dedent.

use crate::config::LuaSrc;

/// A chunk's position: the file line and column (1-based) of its line 1, column 1.
pub fn file_position(src: &LuaSrc, chunk_line: usize, chunk_col: usize) -> (usize, usize) {
    let body = src.repr.trim_start_matches('#');
    if body.starts_with("\"\"\"") {
        let dedent = src
            .repr
            .rfind('\n')
            .map(|nl| {
                src.repr[nl + 1..]
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .count()
            })
            .unwrap_or(0);
        (src.line + chunk_line, dedent + chunk_col)
    } else {
        let hashes = src.repr.len() - body.len();
        // Past the opening `#"` / `"`.
        (src.line, src.col + hashes + 1 + chunk_col.saturating_sub(1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Span;

    fn src(repr: &str, line: usize, col: usize) -> LuaSrc {
        LuaSrc {
            code: String::new(),
            repr: repr.into(),
            span: Span::default(),
            line,
            col,
        }
    }

    #[test]
    fn maps_multi_line_strings_with_their_dedent() {
        // script #"""        (file line 10)
        //     local x = 1    (file line 11, chunk line 1)
        //     """#
        let s = src("#\"\"\"\n    local x = 1\n    \"\"\"#", 10, 8);
        assert_eq!(file_position(&s, 1, 1), (11, 5));
        assert_eq!(file_position(&s, 3, 2), (13, 6));
    }

    #[test]
    fn maps_single_line_strings() {
        // `if="values.x"` with the value at column 20.
        let s = src("\"values.x\"", 4, 20);
        assert_eq!(file_position(&s, 1, 1), (4, 21));
        let raw = src("#\"a\"#", 4, 20);
        assert_eq!(file_position(&raw, 1, 1), (4, 22));
    }
}
