# Lua API reference

Generated from `crates/launcher-core/src/lua/api.rs` (`cargo xtask docs`); do not edit by hand.

A `.launcher` file's Luau code (`script` blocks, `(lua)"..."` values, conditions and `lua` steps) runs in a sandbox: Luau's standard library without `io` or process control, plus the globals below. Code that runs while the file loads (computed values, option lists, defaults, conditions) has a time budget and can only use the functions marked *load*; the rest work in actions.

| Name | When | What |
|---|---|---|
| `values.name` | value | The current value of each control and computed value, and each run variable (set, ask, step ids). Read-only: change them with set() or set-control. |
| `fields.name.Key` | value | Fields: of the option chosen in a control, of a data block, or of a step's result (fields.api.status). The Lua side of {name.Key}. |
| `settings.name` | value | In an add-on step's or list's script: the values of its settings. Read-only. |
| `config_dir` | value | The folder of the .launcher file (relative paths resolve against it). |
| `config_file` | value | The full path of the .launcher file. |
| `env.NAME` | value | The run's environment variables (read-only; env-from and run steps' env change them). |
| `os.name` | value | "windows" or "macos". The rest of Luau's `os` (os.time, os.clock, os.date) is there too. |
| `capture(program, arg...) -> string` | load, run | Run a program without a console and return its output, with the trailing newline removed. A non-zero exit raises an error; pass a table last for options: {check=false, cwd=..., env={...}, stdin=..., timeout=ms}. |
| `run(program, arg...) -> {exit_code}` | run | Run a program in the console, like a run step. A non-zero exit raises an error unless the last argument is {check=false}; other options: cwd, env, stdin, timeout, io. |
| `step(label, function)` | run | Show the work the function does as its own step in the step list. |
| `exists(path) -> boolean` | load, run | Whether a file or folder exists. Relative paths resolve against config_dir. |
| `glob(pattern) -> {path...}` | load, run | Paths matching a pattern (* and ? in names, ** for any folders), sorted. Relative to config_dir. |
| `read_file(path) -> string` | load, run | A text file's contents. |
| `write_file(path, text)` | run | Write a text file, replacing it atomically and creating missing folders. |
| `copy(from, to)` | run | Copy a file, or a folder with everything in it. |
| `move(from, to)` | run | Move or rename a file or folder. |
| `delete(path)` | run | Delete a file, or a folder with everything in it. A missing path is fine. |
| `mkdir(path)` | run | Create a folder and any missing parents. |
| `path.join(part...) -> string` | load, run | Join path parts with the OS's separator. |
| `path.dirname(path) -> string` | load, run | The folder a path is in. |
| `path.basename(path) -> string` | load, run | The last part of a path (file name with extension). |
| `path.stem(path) -> string` | load, run | The file name without its extension. |
| `path.ext(path) -> string` | load, run | The extension without the dot, or "". |
| `path.absolute(path) -> string` | load, run | The path made absolute against config_dir, with . and .. resolved. |
| `json.encode(value) -> string` | load, run | A Lua value as JSON text. |
| `json.decode(text) -> value` | load, run | JSON text as a Lua value. |
| `http_get(url, {timeout=ms, headers={...}}?) -> {status, body}` | run | Fetch a URL (https:// or http://). |
| `trim(text) -> string` | load, run | Text without leading and trailing whitespace. |
| `split(text, separator) -> {string...}` | load, run | Text split at each separator (plain text, not a pattern). |
| `log(value...)` | load, run | Write a line to the console (in a run) and the log. |
| `message(text)` | run | Show a message and wait for OK. |
| `confirm(text) -> boolean` | run | Ask a yes/no question. |
| `ask(prompt, default?) -> string?` | run | Ask for a line of text; nil when cancelled. |
| `set(name, value)` | run | Set a run variable (values.name, {name}) or, when a control has that id, the control's value. |
| `fail(message)` | load, run | Stop with an error message (no Lua traceback). |
