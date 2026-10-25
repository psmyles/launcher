//! **The Lua API registry** (N15, M3): every global a `.launcher` file's Luau code can use. The
//! bindings (`lua/bind.rs`), `docs/lua-api.md`, the lint definitions (M8) and the editor's
//! autocomplete all come from here; a test fails when a binding and this list disagree.

/// When a function may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// While the file loads (computed values, option lists, defaults, conditions) and during runs.
    Any,
    /// Only during a run (an action's steps; in on-change only `set` and `log`): it changes
    /// something or talks to the user, which loading a file must never do.
    Run,
}

#[derive(Clone, Copy, Debug)]
pub struct Item {
    /// The global's name; `path.join` is a field of the `path` table.
    pub name: &'static str,
    /// How it is called, for docs and autocomplete.
    pub signature: &'static str,
    pub doc: &'static str,
    pub phase: Phase,
    /// A function (false: a value or table).
    pub function: bool,
}

const fn f(name: &'static str, signature: &'static str, phase: Phase, doc: &'static str) -> Item {
    Item {
        name,
        signature,
        doc,
        phase,
        function: true,
    }
}

const fn v(name: &'static str, signature: &'static str, doc: &'static str) -> Item {
    Item {
        name,
        signature,
        doc,
        phase: Phase::Any,
        function: false,
    }
}

pub const API: &[Item] = &[
    // ---- the form and the run --------------------------------------------------------------------
    v(
        "values",
        "values.name",
        "The current value of each control and computed value, and each run variable (set, ask, step ids). Read-only: change them with set() or set-control.",
    ),
    v(
        "fields",
        "fields.name.Key",
        "Fields: of the option chosen in a control, of a data block, or of a step's result (fields.api.status). The Lua side of {name.Key}.",
    ),
    v(
        "settings",
        "settings.name",
        "In an add-on step's or list's script: the values of its settings. Read-only.",
    ),
    v("config_dir", "config_dir", "The folder of the .launcher file (relative paths resolve against it)."),
    v("config_file", "config_file", "The full path of the .launcher file."),
    v("env", "env.NAME", "The run's environment variables (read-only; env-from and run steps' env change them)."),
    v("os.name", "os.name", "\"windows\" or \"macos\". The rest of Luau's `os` (os.time, os.clock, os.date) is there too."),
    // ---- programs ------------------------------------------------------------------------------
    f(
        "capture",
        "capture(program, arg...) -> string",
        Phase::Any,
        "Run a program without a console and return its output, with the trailing newline removed. A non-zero exit raises an error; pass a table last for options: {check=false, cwd=..., env={...}, stdin=..., timeout=ms}.",
    ),
    f(
        "run",
        "run(program, arg...) -> {exit_code}",
        Phase::Run,
        "Run a program in the console, like a run step. A non-zero exit raises an error unless the last argument is {check=false}; other options: cwd, env, stdin, timeout, io.",
    ),
    f(
        "step",
        "step(label, function)",
        Phase::Run,
        "Show the work the function does as its own step in the step list.",
    ),
    // ---- files ---------------------------------------------------------------------------------
    f("exists", "exists(path) -> boolean", Phase::Any, "Whether a file or folder exists. Relative paths resolve against config_dir."),
    f(
        "glob",
        "glob(pattern) -> {path...}",
        Phase::Any,
        "Paths matching a pattern (* and ? in names, ** for any folders), sorted. Relative to config_dir.",
    ),
    f("read_file", "read_file(path) -> string", Phase::Any, "A text file's contents."),
    f(
        "write_file",
        "write_file(path, text)",
        Phase::Run,
        "Write a text file, replacing it atomically and creating missing folders.",
    ),
    f("copy", "copy(from, to)", Phase::Run, "Copy a file, or a folder with everything in it."),
    f("move", "move(from, to)", Phase::Run, "Move or rename a file or folder."),
    f("delete", "delete(path)", Phase::Run, "Delete a file, or a folder with everything in it. A missing path is fine."),
    f("mkdir", "mkdir(path)", Phase::Run, "Create a folder and any missing parents."),
    f("path.join", "path.join(part...) -> string", Phase::Any, "Join path parts with the OS's separator."),
    f("path.dirname", "path.dirname(path) -> string", Phase::Any, "The folder a path is in."),
    f("path.basename", "path.basename(path) -> string", Phase::Any, "The last part of a path (file name with extension)."),
    f("path.stem", "path.stem(path) -> string", Phase::Any, "The file name without its extension."),
    f("path.ext", "path.ext(path) -> string", Phase::Any, "The extension without the dot, or \"\"."),
    f(
        "path.absolute",
        "path.absolute(path) -> string",
        Phase::Any,
        "The path made absolute against config_dir, with . and .. resolved.",
    ),
    // ---- data ----------------------------------------------------------------------------------
    f("json.encode", "json.encode(value) -> string", Phase::Any, "A Lua value as JSON text."),
    f("json.decode", "json.decode(text) -> value", Phase::Any, "JSON text as a Lua value."),
    f(
        "http_get",
        "http_get(url, {timeout=ms, headers={...}}?) -> {status, body}",
        Phase::Run,
        "Fetch a URL (https:// or http://).",
    ),
    f("trim", "trim(text) -> string", Phase::Any, "Text without leading and trailing whitespace."),
    f("split", "split(text, separator) -> {string...}", Phase::Any, "Text split at each separator (plain text, not a pattern)."),
    // ---- talking to the user -------------------------------------------------------------------
    f("log", "log(value...)", Phase::Any, "Write a line to the console (in a run) and the log."),
    f("message", "message(text)", Phase::Run, "Show a message and wait for OK."),
    f("confirm", "confirm(text) -> boolean", Phase::Run, "Ask a yes/no question."),
    f(
        "ask",
        "ask(prompt, default?) -> string?",
        Phase::Run,
        "Ask for a line of text; nil when cancelled.",
    ),
    f(
        "set",
        "set(name, value)",
        Phase::Run,
        "Set a run variable (values.name, {name}) or, when a control has that id, the control's value.",
    ),
    f("fail", "fail(message)", Phase::Any, "Stop with an error message (no Lua traceback)."),
];

pub fn get(name: &str) -> Option<&'static Item> {
    API.iter().find(|i| i.name == name)
}

/// `docs/lua-api.md`.
pub fn reference_markdown() -> String {
    let mut out = String::from(
        "# Lua API reference\n\n\
         Generated from `crates/launcher-core/src/lua/api.rs` (`cargo xtask docs`); do not edit by \
         hand.\n\n\
         A `.launcher` file's Luau code (`script` blocks, `(lua)\"...\"` values, conditions and `lua` \
         steps) runs in a sandbox: Luau's standard library without `io` or process control, plus \
         the globals below. Code that runs while the file loads (computed values, option lists, \
         defaults, conditions) has a time budget and can only use the functions marked *load*; the \
         rest work in actions.\n\n\
         | Name | When | What |\n|---|---|---|\n",
    );
    for i in API {
        let when = match (i.function, i.phase) {
            (false, _) => "value",
            (true, Phase::Any) => "load, run",
            (true, Phase::Run) => "run",
        };
        out.push_str(&format!(
            "| `{}` | {when} | {} |\n",
            i.signature,
            i.doc.replace('|', "\\|")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique_and_documented() {
        let mut seen = std::collections::HashSet::new();
        for i in API {
            assert!(seen.insert(i.name), "duplicate {}", i.name);
            assert!(
                !i.doc.is_empty() && i.signature.starts_with(i.name.split('.').next().unwrap())
            );
        }
    }
}
