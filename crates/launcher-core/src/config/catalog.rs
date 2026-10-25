//! **The single registry** of every node kind a `.launcher` file may contain (N15: one registry
//! feeds the loader, validation, the editor, the generated docs and the CLI help). A new kind or
//! property is added here once; `docs/config-reference.md` is generated from it and a test keeps
//! the file current.

/// What a parameter holds. The loader checks values against it; the editor picks its widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    /// Plain text, used as written.
    Text,
    /// Text with `{name}`, `{name.field}` and `{env.NAME}` placeholders. Also accepts `(lua)`.
    Template,
    /// A Lua expression (conditions: `if=`, `enabled-if=`, `visible-if=`).
    Lua,
    /// Lua statements (a `script` block, a `lua` step).
    LuaChunk,
    /// A Lua expression that must be written `(lua)"..."` (computed option lists).
    LuaValue,
    Int,
    Number,
    Bool,
    /// One of a fixed set of words.
    Enum(&'static [&'static str]),
    /// A name this node defines (control id, value id, action id, step result...).
    Id,
    /// The id of a control.
    ControlRef,
    /// The id of an action.
    ActionRef,
    /// The id of a control or a `data` block (the source of `pairs`).
    BagRef,
    /// The `id` of an earlier `run` step.
    RunRef,
    /// A control's value: text, number or #true/#false. Also accepts `(lua)`.
    Any,
    /// A version: "1", "1.2" or "1.2.3".
    Version,
    /// A field width: a number of logical pixels, "fill" or "auto".
    Width,
}

impl Ty {
    /// Whether a `(lua)"expr"` value is accepted in place of a literal.
    pub fn accepts_lua(self) -> bool {
        matches!(
            self,
            Ty::Template | Ty::Any | Ty::LuaValue | Ty::Number | Ty::Int
        )
    }
}

/// Where a node or property is available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Any,
    Windows,
    Macos,
}

impl Os {
    pub fn name(self) -> &'static str {
        match self {
            Os::Any => "any",
            Os::Windows => "windows",
            Os::Macos => "macos",
        }
    }

    /// The OS this build runs on.
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else {
            Os::Macos
        }
    }

    pub fn allows(self, target: Os) -> bool {
        self == Os::Any || target == Os::Any || self == target
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Param {
    pub name: &'static str,
    pub ty: Ty,
    pub required: bool,
    /// Shown in the editor and docs when the parameter is absent ("" = none).
    pub default: &'static str,
    pub doc: &'static str,
    pub os: Os,
    /// Worked out from the rest of a format-2 tool, never a setting people see (what Write to a
    /// text file's list holds).
    pub filled: bool,
}

const fn p(name: &'static str, ty: Ty, doc: &'static str) -> Param {
    Param {
        name,
        ty,
        required: false,
        default: "",
        doc,
        os: Os::Any,
        filled: false,
    }
}

const fn pd(name: &'static str, ty: Ty, default: &'static str, doc: &'static str) -> Param {
    Param {
        name,
        ty,
        required: false,
        default,
        doc,
        os: Os::Any,
        filled: false,
    }
}

const fn req(name: &'static str, ty: Ty, doc: &'static str) -> Param {
    Param {
        name,
        ty,
        required: true,
        default: "",
        doc,
        os: Os::Any,
        filled: false,
    }
}

const fn win(p: Param) -> Param {
    Param {
        os: Os::Windows,
        ..p
    }
}

const fn filled(p: Param) -> Param {
    Param { filled: true, ..p }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Category {
    /// Only at the top of the file.
    Top,
    /// Layout containers and decorations.
    Layout,
    Control,
    Step,
    /// Only inside a specific parent (option, preset, extract...).
    Part,
}

/// What a node's `{ ... }` block may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Children {
    None,
    /// Layout items: containers, decorations and controls.
    Layout,
    /// Steps.
    Steps,
    /// A data bag: `Key "value"` lines.
    Bag,
    /// Only these node kinds.
    Only(&'static [&'static str]),
}

/// Parameters shared by kinds, appended to their own (see [`Spec::all_props`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Common {
    None,
    Control,
    Step,
}

#[derive(Clone, Copy, Debug)]
pub struct Spec {
    pub name: &'static str,
    pub cat: Category,
    pub doc: &'static str,
    /// Positional arguments, in order.
    pub args: &'static [Param],
    /// Any number of further positional arguments (a program's arguments), all of this kind.
    pub rest: Option<Param>,
    pub props: &'static [Param],
    pub children: Children,
    pub common: Common,
    pub os: Os,
    /// Steps: allowed in `on-change` (instant, no I/O).
    pub on_change: bool,
    /// Controls: holds a value (buttons and displays do not).
    pub has_value: bool,
}

impl Spec {
    /// Own properties followed by the shared ones.
    pub fn all_props(&self) -> impl Iterator<Item = &'static Param> {
        let common: &'static [Param] = match self.common {
            Common::None => &[],
            Common::Control => CONTROL_COMMON,
            Common::Step => STEP_COMMON,
        };
        self.props.iter().chain(common.iter())
    }

    pub fn prop(&self, name: &str) -> Option<&'static Param> {
        self.all_props().find(|p| p.name == name)
    }
}

const fn spec(name: &'static str, cat: Category, doc: &'static str) -> Spec {
    Spec {
        name,
        cat,
        doc,
        args: &[],
        rest: None,
        props: &[],
        children: Children::None,
        common: Common::None,
        os: Os::Any,
        on_change: false,
        has_value: false,
    }
}

pub const AFTER: &[&str] = &["stay", "exit", "minimize"];
pub const ON_ERROR: &[&str] = &["abort", "warn", "continue"];
pub const LEVELS: &[&str] = &["info", "warning", "error"];
pub const LOG_LEVELS: &[&str] = &["debug", "info", "warn", "error"];
pub const WRITE_FORMATS: &[&str] = &["text", "json-pretty", "json-compact"];
pub const NEWLINES: &[&str] = &["keep", "lf", "crlf"];
pub const DEDUPE: &[&str] = &["first", "last", "none"];
pub const THEMES: &[&str] = &["system", "dark", "light"];
pub const POSITIONS: &[&str] = &["remember", "center"];
pub const NOTE_STYLES: &[&str] = &["normal", "muted", "warning", "error"];
pub const OSES: &[&str] = &["windows", "macos"];
pub const IO: &[&str] = &["auto", "pty", "pipe"];
pub const MISSING: &[&str] = &["ok", "error"];

pub const CONTROL_COMMON: &[Param] = &[
    p("label", Ty::Text, "Text in the label column."),
    p("tooltip", Ty::Template, "Shown when hovering the control."),
    p(
        "default",
        Ty::Any,
        "The value before the user changes it. `(lua)\"...\"` works it out when the file loads.",
    ),
    pd(
        "persist",
        Ty::Bool,
        "#true",
        "Remember the value between launches.",
    ),
    p("visible-if", Ty::Lua, "Only show the control when this is true."),
    p(
        "enabled-if",
        Ty::Lua,
        "Grey the control out unless this is true.",
    ),
    p(
        "disabled-tooltip",
        Ty::Template,
        "Shown when hovering the control while it is greyed out.",
    ),
    pd(
        "required",
        Ty::Bool,
        "#false",
        "Actions can't run while the control is empty; the Run button says why.",
    ),
    p(
        "width",
        Ty::Width,
        "Field width: \"fill\" (to the form's right edge), \"auto\", or a number of logical pixels.",
    ),
];

pub const STEP_COMMON: &[Param] = &[
    p(
        "id",
        Ty::Id,
        "Names the step's result for later steps: {id.field} or values.id in Lua.",
    ),
    p(
        "label",
        Ty::Template,
        "What the step list shows for this step.",
    ),
    p("if", Ty::Lua, "Skip the step unless this is true."),
    pd(
        "on-error",
        Ty::Enum(ON_ERROR),
        "abort",
        "What a failure does: stop the run, warn and continue, or continue silently.",
    ),
    p(
        "message",
        Ty::Template,
        "The text shown when the step fails. {error} is the underlying error.",
    ),
    pd(
        "first",
        Ty::Bool,
        "#false",
        "Run before every other step (\"Before starting, make sure…\"), like a check.",
    ),
];

const ID: Param = req("id", Ty::Id, "The name other parts of the file refer to.");

const PROGRAM: Param = req(
    "program",
    Ty::Template,
    "The program: a name found on PATH, or a path (relative to the file's folder).",
);
const PROGRAM_ARG: Param = p(
    "arguments",
    Ty::Template,
    "The program's arguments, one per value, passed exactly (no shell).",
);

pub const SPECS: &[Spec] = &[
    // ---- top level ----------------------------------------------------------------------------
    Spec {
        doc: "The header, first in the file. The launcher reads it before anything else.",
        props: &[
            p(
                "requires",
                Ty::Version,
                "The oldest Launcher version this file works with (e.g. \"1.2\").",
            ),
            p(
                "os",
                Ty::Enum(OSES),
                "The one OS this file is for. Leave it out for a file that works on both.",
            ),
            pd(
                "save-logs",
                Ty::Bool,
                "#false",
                "Save what each run printed to files on this computer (\"Log files\" opens them). \
                 Without it, a run's output only shows in the window.",
            ),
        ],
        ..spec("launcher", Category::Top, "")
    },
    Spec {
        props: &[
            pd("title", Ty::Template, "Launcher", "The window title."),
            p(
                "width",
                Ty::Number,
                "Fixed width (logical px). Without it the window fits its content.",
            ),
            p(
                "height",
                Ty::Number,
                "Fixed height (logical px). Without it the window fits its content.",
            ),
            pd(
                "resizable",
                Ty::Bool,
                "#true",
                "Let the user resize the window.",
            ),
            pd(
                "theme",
                Ty::Enum(THEMES),
                "system",
                "Dark or light colours, or follow the system.",
            ),
            pd(
                "label-width",
                Ty::Number,
                "140",
                "Width of the label column (logical px).",
            ),
            pd(
                "field-width",
                Ty::Number,
                "260",
                "Width of the field column while the window fits its content (logical px).",
            ),
            pd(
                "position",
                Ty::Enum(POSITIONS),
                "remember",
                "Open where it was last closed, or centred.",
            ),
        ],
        ..spec("window", Category::Top, "The window.")
    },
    Spec {
        args: &[req(
            "code",
            Ty::LuaChunk,
            "Luau code: functions and values the rest of the file can use.",
        )],
        ..spec(
            "script",
            Category::Top,
            "Luau code loaded before anything is evaluated. Several `script` blocks run in order.",
        )
    },
    Spec {
        args: &[
            ID,
            p(
                "value",
                Ty::Template,
                "What the value is: a template, or `(lua)\"...\"`. Leave out to use a find-file block.",
            ),
        ],
        children: Children::Only(&["find-file"]),
        ..spec(
            "value",
            Category::Top,
            "A value worked out when the file loads: a template, a Lua expression, or a find-file search.",
        )
    },
    Spec {
        args: &[ID],
        children: Children::Bag,
        ..spec(
            "data",
            Category::Top,
            "A named list of key/value pairs (for example, arguments sent on every launch).",
        )
    },
    Spec {
        children: Children::Layout,
        ..spec("layout", Category::Top, "What the window shows, top to bottom.")
    },
    Spec {
        args: &[ID],
        props: &[
            p("label", Ty::Text, "The name shown for the action."),
            pd(
                "after",
                Ty::Enum(AFTER),
                "stay",
                "After a successful run: keep the window, close it, or minimize it.",
            ),
            p(
                "lock",
                Ty::Template,
                "A file path guarding a shared resource: two runs holding the same lock never overlap, even from different files.",
            ),
        ],
        children: Children::Steps,
        ..spec(
            "action",
            Category::Top,
            "Steps a button (or `launcher run --action`) carries out, in order.",
        )
    },
    // ---- layout -------------------------------------------------------------------------------
    Spec {
        args: &[p("title", Ty::Template, "The group's title.")],
        props: &[p(
            "visible-if",
            Ty::Lua,
            "Only show the group when this is true.",
        )],
        children: Children::Layout,
        ..spec("group", Category::Layout, "A titled box around related controls.")
    },
    Spec {
        children: Children::Only(&["tab"]),
        ..spec("tabs", Category::Layout, "Tab pages; each `tab` is one page.")
    },
    Spec {
        args: &[req("title", Ty::Template, "The tab's title.")],
        props: &[p("visible-if", Ty::Lua, "Only show the tab when this is true.")],
        children: Children::Layout,
        ..spec("tab", Category::Part, "One tab page.")
    },
    Spec {
        children: Children::Only(&["column"]),
        ..spec(
            "columns",
            Category::Layout,
            "Side-by-side columns; each `column` is one.",
        )
    },
    Spec {
        children: Children::Layout,
        ..spec("column", Category::Part, "One column.")
    },
    Spec {
        props: &[p("visible-if", Ty::Lua, "Only show the row when this is true.")],
        children: Children::Layout,
        ..spec(
            "row",
            Category::Layout,
            "Several controls on one line. The first one's label sits in the label column.",
        )
    },
    Spec {
        args: &[p("title", Ty::Template, "Text on the line.")],
        ..spec(
            "separator",
            Category::Layout,
            "A horizontal line, optionally with a title.",
        )
    },
    Spec {
        props: &[pd("height", Ty::Number, "8", "Height in logical px.")],
        ..spec("spacer", Category::Layout, "Empty vertical space.")
    },
    Spec {
        args: &[req("text", Ty::Template, "The text.")],
        props: &[
            pd(
                "style",
                Ty::Enum(NOTE_STYLES),
                "normal",
                "How the text looks.",
            ),
            p("visible-if", Ty::Lua, "Only show the note when this is true."),
        ],
        ..spec("note", Category::Layout, "A line of text.")
    },
    // ---- controls -----------------------------------------------------------------------------
    Spec {
        args: &[ID],
        props: &[
            p(
                "options",
                Ty::LuaValue,
                "`(lua)\"...\"`: a list worked out when the file loads (strings, or tables with name and data fields), instead of `option` lines. The list has a refresh button.",
            ),
            p(
                "option-label",
                Ty::Template,
                "How each option is shown: {name} and its data fields.",
            ),
        ],
        children: Children::Only(&["option", "preset", "on-change", "list-from"]),
        common: Common::Control,
        has_value: true,
        ..spec(
            "dropdown",
            Category::Control,
            "Pick one option from a list. Each option can carry key/value data.",
        )
    },
    Spec {
        args: &[ID],
        props: &[
            pd(
                "horizontal",
                Ty::Bool,
                "#true",
                "Lay the buttons out on one line.",
            ),
            p(
                "options",
                Ty::LuaValue,
                "`(lua)\"...\"`: a computed list of options instead of `option` lines.",
            ),
            p("option-label", Ty::Template, "How each option is shown."),
        ],
        children: Children::Only(&["option", "preset", "on-change", "list-from"]),
        common: Common::Control,
        has_value: true,
        ..spec("radio", Category::Control, "Pick one option with radio buttons.")
    },
    Spec {
        args: &[ID],
        props: &[
            p(
                "options",
                Ty::LuaValue,
                "`(lua)\"...\"`: a computed list of options instead of `option` lines.",
            ),
            p("option-label", Ty::Template, "How each option is shown."),
        ],
        children: Children::Only(&["option", "on-change", "list-from"]),
        common: Common::Control,
        has_value: true,
        ..spec(
            "checklist",
            Category::Control,
            "Tick any number of options. The value is the list of ticked option values.",
        )
    },
    Spec {
        args: &[ID],
        children: Children::Only(&["on-change"]),
        common: Common::Control,
        has_value: true,
        ..spec(
            "checkbox",
            Category::Control,
            "On or off. The label is shown beside the box.",
        )
    },
    Spec {
        args: &[ID],
        props: &[
            pd("min", Ty::Number, "0", "Lowest value."),
            pd("max", Ty::Number, "100", "Highest value."),
            pd("step", Ty::Number, "1", "Values snap to multiples of this."),
            p("suffix", Ty::Text, "Shown after the number, e.g. \"%\"."),
        ],
        children: Children::Only(&["on-change"]),
        common: Common::Control,
        has_value: true,
        ..spec("slider", Category::Control, "A number picked by dragging.")
    },
    Spec {
        args: &[ID],
        props: &[
            p("min", Ty::Number, "Lowest value."),
            p("max", Ty::Number, "Highest value."),
            pd(
                "step",
                Ty::Number,
                "1",
                "Dragging or the arrow keys change the value by this.",
            ),
            pd("decimals", Ty::Int, "0", "Digits after the decimal point."),
        ],
        children: Children::Only(&["on-change"]),
        common: Common::Control,
        has_value: true,
        ..spec(
            "number",
            Category::Control,
            "A number typed in, with optional limits.",
        )
    },
    Spec {
        args: &[ID],
        props: &[
            pd("multiline", Ty::Bool, "#false", "Several lines."),
            pd(
                "lines",
                Ty::Int,
                "4",
                "Height of a multiline field, in lines.",
            ),
            pd("password", Ty::Bool, "#false", "Hide what is typed."),
            p("placeholder", Ty::Text, "Grey hint shown while empty."),
            p(
                "pattern",
                Ty::Text,
                "A regular expression the whole value must match before actions can run.",
            ),
            p(
                "pattern-message",
                Ty::Text,
                "What to say when the value doesn't match `pattern`.",
            ),
        ],
        children: Children::Only(&["on-change"]),
        common: Common::Control,
        has_value: true,
        ..spec("text", Category::Control, "Free text.")
    },
    Spec {
        args: &[ID],
        props: &[
            p(
                "filter",
                Ty::Text,
                "File types, e.g. \"Programs|*.exe;*.bat\".",
            ),
            pd(
                "save",
                Ty::Bool,
                "#false",
                "Pick a file to create rather than an existing one.",
            ),
            pd(
                "must-exist",
                Ty::Bool,
                "#false",
                "Actions can't run until the file exists.",
            ),
            pd(
                "relative",
                Ty::Bool,
                "#true",
                "Store paths inside the file's folder as relative paths.",
            ),
            p("placeholder", Ty::Text, "Grey hint shown while empty."),
        ],
        children: Children::Only(&["on-change"]),
        common: Common::Control,
        has_value: true,
        ..spec("file", Category::Control, "A file path with a Browse button.")
    },
    Spec {
        args: &[ID],
        props: &[
            pd(
                "must-exist",
                Ty::Bool,
                "#false",
                "Actions can't run until the folder exists.",
            ),
            pd(
                "relative",
                Ty::Bool,
                "#true",
                "Store paths inside the file's folder as relative paths.",
            ),
            p("placeholder", Ty::Text, "Grey hint shown while empty."),
        ],
        children: Children::Only(&["on-change"]),
        common: Common::Control,
        has_value: true,
        ..spec(
            "folder",
            Category::Control,
            "A folder path with a Browse button.",
        )
    },
    Spec {
        args: &[ID],
        props: &[
            req("text", Ty::Template, "What to show."),
            pd(
                "copyable",
                Ty::Bool,
                "#true",
                "Let the user select and copy the text.",
            ),
        ],
        common: Common::Control,
        ..spec(
            "display",
            Category::Control,
            "Read-only text worked out from other values.",
        )
    },
    Spec {
        args: &[ID],
        props: &[
            req("action", Ty::ActionRef, "The action to run."),
            p("confirm", Ty::Template, "Ask this question first; No cancels."),
            pd("primary", Ty::Bool, "#false", "Enter presses this button, and it is drawn in the accent colour."),
        ],
        common: Common::Control,
        ..spec("button", Category::Control, "Runs an action.")
    },
    // ---- control parts ------------------------------------------------------------------------
    Spec {
        args: &[req(
            "name",
            Ty::Text,
            "What the list shows (and what is remembered).",
        )],
        props: &[
            p(
                "value",
                Ty::Text,
                "The value when chosen (default: the name).",
            ),
            p(
                "label",
                Ty::Template,
                "Display text overriding the name and option-label.",
            ),
        ],
        children: Children::Bag,
        ..spec(
            "option",
            Category::Part,
            "One choice. The lines in its block are its data: `Key \"value\"`.",
        )
    },
    Spec {
        args: &[req(
            "value",
            Ty::Template,
            "The value to select, usually a field of another control.",
        )],
        props: &[p(
            "match",
            Ty::Text,
            "Match against this data field instead of the option's value.",
        )],
        ..spec(
            "preset",
            Category::Part,
            "Selects the option this names whenever it is not empty (a role that fixes the server, say). While a preset applies, the user's choice is not remembered.",
        )
    },
    Spec {
        children: Children::Steps,
        ..spec(
            "on-change",
            Category::Part,
            "Steps run the moment the control changes. Only instant steps: set, set-control, reset, log, lua.",
        )
    },
    // ---- steps --------------------------------------------------------------------------------
    Spec {
        args: &[PROGRAM],
        rest: Some(PROGRAM_ARG),
        props: &[
            win(p(
                "command-line",
                Ty::Template,
                "Windows: the arguments as one raw command line, passed exactly as written (instead of separate arguments).",
            )),
            p(
                "cwd",
                Ty::Template,
                "Working folder (default: the file's folder).",
            ),
            p(
                "stdin",
                Ty::Template,
                "Text fed to the program's input, then the input is closed.",
            ),
            pd(
                "wait",
                Ty::Bool,
                "#true",
                "Wait for the program to exit. With #false it is started and left running (a game, an editor).",
            ),
            p(
                "timeout",
                Ty::Int,
                "Give up (and stop the program) after this many milliseconds.",
            ),
            pd(
                "expect-exit",
                Ty::Text,
                "0",
                "Exit codes that mean success, e.g. \"0\" or \"0,1\". \"any\" accepts every code.",
            ),
            pd(
                "io",
                Ty::Enum(IO),
                "auto",
                "How output reaches the console: a pseudo-terminal (colours, progress bars), pipes, or auto (pipes only for GUI programs).",
            ),
            p(
                "fail-if-output",
                Ty::Text,
                "A regular expression: the step fails if any output line matches it.",
            ),
        ],
        children: Children::Only(&["env"]),
        common: Common::Step,
        ..spec(
            "run",
            Category::Step,
            "Start a program and show its output in the console. Result: {id.exit_code}, {id.pid}.",
        )
    },
    Spec {
        args: &[req("name", Ty::Text, "Variable name."), req("value", Ty::Template, "Its value.")],
        ..spec("env", Category::Part, "An environment variable for the program.")
    },
    Spec {
        args: &[PROGRAM],
        rest: Some(PROGRAM_ARG),
        props: &[
            p("cwd", Ty::Template, "Working folder (default: the file's folder)."),
            pd(
                "expect-exit",
                Ty::Text,
                "0",
                "Exit codes that mean the check passed.",
            ),
        ],
        common: Common::Step,
        ..spec(
            "check",
            Category::Step,
            "A precondition: run a program quietly; any other exit code stops the run with `message`. Checks run before any other step.",
        )
    },
    Spec {
        args: &[PROGRAM],
        rest: Some(PROGRAM_ARG),
        props: &[
            p("cwd", Ty::Template, "Working folder (default: the file's folder)."),
            pd(
                "expect-exit",
                Ty::Text,
                "0",
                "Exit codes that count as working; any other fails the step.",
            ),
        ],
        common: Common::Step,
        ..spec(
            "capture",
            Category::Step,
            "Run a program quietly and keep what it printed. Result: {id.output}, {id.exit_code}.",
        )
    },
    Spec {
        args: &[req(
            "step",
            Ty::Text,
            "The add-on step, as add-on.step (git.pull).",
        )],
        children: Children::Only(&["with"]),
        common: Common::Step,
        ..spec(
            "use",
            Category::Step,
            "A step from an add-on. Its results are {id.result}.",
        )
    },
    Spec {
        args: &[req(
            "list",
            Ty::Text,
            "The add-on list, as add-on.list (git.branches).",
        )],
        children: Children::Only(&["with"]),
        ..spec(
            "list-from",
            Category::Part,
            "Fill the choices from an add-on's list (Git branches, installed versions...).",
        )
    },
    Spec {
        args: &[
            req("setting", Ty::Text, "The setting's name."),
            req("value", Ty::Any, "Its value: text with {names}, a number, #true or #false."),
        ],
        ..spec("with", Category::Part, "A setting of an add-on step.")
    },
    Spec {
        args: &[
            req(
                "script",
                Ty::Template,
                "The setup script (vcvarsall.bat, a conda activate script, ...).",
            ),
        ],
        rest: Some(p(
            "arguments",
            Ty::Template,
            "Arguments for the script.",
        )),
        common: Common::Step,
        ..spec(
            "env-from",
            Category::Step,
            "Run a setup script and keep the environment variables it sets, for every later step.",
        )
    },
    Spec {
        args: &[
            req("name", Ty::Id, "The variable."),
            req("value", Ty::Template, "Its value: a template or `(lua)\"...\"`."),
        ],
        common: Common::Step,
        on_change: true,
        ..spec(
            "set",
            Category::Step,
            "Give a name to a value for later steps: {name}, or values.name in Lua.",
        )
    },
    Spec {
        args: &[
            req("control", Ty::ControlRef, "The control."),
            req("value", Ty::Template, "The new value."),
        ],
        common: Common::Step,
        on_change: true,
        ..spec("set-control", Category::Step, "Change a control's value.")
    },
    Spec {
        args: &[req("control", Ty::ControlRef, "The control.")],
        common: Common::Step,
        on_change: true,
        ..spec("reset", Category::Step, "Put a control back to its default.")
    },
    Spec {
        args: &[
            req("control", Ty::ControlRef, "The control it goes into."),
            req("step", Ty::Text, "The step that found it (its id)."),
            req("result", Ty::Text, "Which of the step's results."),
        ],
        common: Common::Step,
        ..spec(
            "put",
            Category::Step,
            "Put an earlier step's result into a control: a list becomes a choice's choices, anything else its value.",
        )
    },
    Spec {
        args: &[
            req("from", Ty::Template, "The file or folder to copy."),
            req("to", Ty::Template, "Where the copy goes (a file path, or a folder ending in / or \\)."),
        ],
        props: &[
            pd(
                "overwrite",
                Ty::Bool,
                "#true",
                "Replace files that are already there.",
            ),
            pd(
                "ask-first",
                Ty::Bool,
                "#false",
                "When the copy is already there, ask before replacing it.",
            ),
        ],
        common: Common::Step,
        ..spec(
            "copy",
            Category::Step,
            "Copy a file, or a folder with everything in it.",
        )
    },
    Spec {
        args: &[
            req("from", Ty::Template, "The file or folder to move."),
            req("to", Ty::Template, "Where it goes."),
        ],
        props: &[pd(
            "overwrite",
            Ty::Bool,
            "#false",
            "Replace a file that is already there.",
        )],
        common: Common::Step,
        ..spec("move", Category::Step, "Move or rename a file or folder.")
    },
    Spec {
        args: &[req("path", Ty::Template, "The file or folder.")],
        props: &[
            pd(
                "missing",
                Ty::Enum(MISSING),
                "ok",
                "Whether a path that isn't there is fine or an error.",
            ),
            pd(
                "to-trash",
                Ty::Bool,
                "#false",
                "Move it to the Recycle Bin or Trash instead of deleting it for good.",
            ),
        ],
        common: Common::Step,
        ..spec(
            "delete",
            Category::Step,
            "Delete a file, or a folder with everything in it.",
        )
    },
    Spec {
        args: &[req("path", Ty::Template, "The folder.")],
        common: Common::Step,
        ..spec(
            "mkdir",
            Category::Step,
            "Create a folder (and any missing parents).",
        )
    },
    Spec {
        args: &[req(
            "pattern",
            Ty::Text,
            "File name pattern, e.g. \"*.exe\".",
        )],
        props: &[
            pd("in", Ty::Template, ".", "Folder to look in."),
            pd(
                "recursive",
                Ty::Bool,
                "#false",
                "Look in sub-folders too.",
            ),
            pd(
                "exclude-self",
                Ty::Bool,
                "#true",
                "Never match the launcher's own executable.",
            ),
            p(
                "require-sibling",
                Ty::Template,
                "Only match when this also exists next to the file ({stem}, {name}, {ext}).",
            ),
        ],
        common: Common::Step,
        ..spec(
            "find-file",
            Category::Step,
            "Find the first file matching a pattern. Result: its path, or nothing.",
        )
    },
    Spec {
        args: &[req("path", Ty::Template, "The file.")],
        common: Common::Step,
        ..spec(
            "read-file",
            Category::Step,
            "Read a text file. Result: {id.text}; Lua also gets values.id.json when it is JSON.",
        )
    },
    Spec {
        args: &[req("path", Ty::Template, "The file.")],
        props: &[
            p(
                "heading",
                Ty::Template,
                "A line written above the content. When it is added to a file, a blank line comes first.",
            ),
            req(
                "content",
                Ty::Template,
                "What to write: a template or `(lua)\"...\"`.",
            ),
            pd(
                "format",
                Ty::Enum(WRITE_FORMATS),
                "text",
                "Write as-is, or re-format JSON.",
            ),
            pd("newline", Ty::Enum(NEWLINES), "keep", "Line endings."),
            pd(
                "create-dirs",
                Ty::Bool,
                "#true",
                "Create missing folders.",
            ),
            pd(
                "append",
                Ty::Bool,
                "#false",
                "Add to the end of the file instead of replacing it, each write on lines of its own. The first write to the file in a run still starts it afresh (unless `keep`), so several steps can make one file together.",
            ),
            pd(
                "keep",
                Ty::Bool,
                "#false",
                "With `append`: keep what earlier runs wrote too, so the file grows run after run.",
            ),
            filled(pd(
                "item",
                Ty::Text,
                "item",
                "What the content's list holds, for the count beside the heading: `item=\"model\"` makes \"Models to fix (54 models)\". An empty list is written as \"none\".",
            )),
            filled(p(
                "items",
                Ty::Text,
                "More than one `item`, when it isn't just `item` with an s.",
            )),
        ],
        common: Common::Step,
        ..spec(
            "write-file",
            Category::Step,
            "Write a file: replace it (atomically), or add to it. A list in the content goes in one item per line.",
        )
    },
    Spec {
        args: &[req("url", Ty::Template, "The URL (https:// or http://).")],
        props: &[pd(
            "timeout",
            Ty::Int,
            "10000",
            "Milliseconds before giving up.",
        )],
        children: Children::Only(&["extract", "header"]),
        common: Common::Step,
        ..spec(
            "http-get",
            Category::Step,
            "Fetch a URL. Result: {id.status}, {id.body}. `extract` lines pull fields out of a JSON response.",
        )
    },
    Spec {
        args: &[
            req("name", Ty::Id, "The name."),
            req("path", Ty::Text, "Where the field is: a.b[0].c"),
        ],
        props: &[pd(
            "required",
            Ty::Bool,
            "#false",
            "Fail the step when the field is missing.",
        )],
        ..spec(
            "extract",
            Category::Part,
            "Name a field of the JSON response: `extract \"session_id\" \"data[0].id\"`.",
        )
    },
    Spec {
        args: &[
            req("name", Ty::Text, "Header name."),
            req("value", Ty::Template, "Header value."),
        ],
        ..spec("header", Category::Part, "A request header.")
    },
    Spec {
        args: &[req("host", Ty::Template, "Host name or IP address.")],
        props: &[pd(
            "timeout",
            Ty::Int,
            "1500",
            "Milliseconds to wait for the reply.",
        )],
        common: Common::Step,
        ..spec(
            "ping",
            Category::Step,
            "Check that a host answers (ICMP echo). Result: {id.ms}.",
        )
    },
    Spec {
        args: &[req("target", Ty::Template, "A file, folder or URL.")],
        common: Common::Step,
        ..spec(
            "open",
            Category::Step,
            "Open a file, folder or URL with its default program.",
        )
    },
    Spec {
        args: &[req("path", Ty::Template, "The file or folder.")],
        common: Common::Step,
        ..spec(
            "reveal",
            Category::Step,
            "Show a file or folder in Explorer or Finder.",
        )
    },
    Spec {
        args: &[req("text", Ty::Template, "The message.")],
        props: &[pd("level", Ty::Enum(LEVELS), "info", "Icon and colour.")],
        common: Common::Step,
        ..spec("message", Category::Step, "Show a message and wait for OK.")
    },
    Spec {
        args: &[req("text", Ty::Template, "The question.")],
        props: &[
            pd("yes", Ty::Text, "Yes", "Yes button text."),
            pd("no", Ty::Text, "No", "No button text."),
        ],
        common: Common::Step,
        ..spec(
            "confirm",
            Category::Step,
            "Ask a yes/no question; No ends the run quietly.",
        )
    },
    Spec {
        args: &[
            req("name", Ty::Id, "The variable the answer goes into."),
            req("prompt", Ty::Template, "The question."),
        ],
        props: &[
            p("default", Ty::Template, "The answer already filled in."),
            pd("password", Ty::Bool, "#false", "Hide what is typed."),
        ],
        common: Common::Step,
        ..spec(
            "ask",
            Category::Step,
            "Ask for a line of text; Cancel ends the run quietly.",
        )
    },
    Spec {
        args: &[req("ms", Ty::Int, "Milliseconds.")],
        common: Common::Step,
        ..spec("wait", Category::Step, "Pause for a number of milliseconds.")
    },
    Spec {
        args: &[req("text", Ty::Template, "The line.")],
        props: &[pd(
            "level",
            Ty::Enum(LOG_LEVELS),
            "info",
            "Log level.",
        )],
        common: Common::Step,
        on_change: true,
        ..spec("log", Category::Step, "Write a line to the console and the log.")
    },
    Spec {
        args: &[req("text", Ty::Template, "The error message.")],
        common: Common::Step,
        ..spec("fail", Category::Step, "End the run here with an error.")
    },
    Spec {
        common: Common::Step,
        ..spec("stop", Category::Step, "End the run here, successfully.")
    },
    Spec {
        args: &[req(
            "code",
            Ty::LuaChunk,
            "Luau code. It can call run(), capture(), copy() and the rest of the Lua API.",
        )],
        common: Common::Step,
        on_change: true,
        ..spec("lua", Category::Step, "Run Luau code as a step.")
    },
    Spec {
        args: &[req("name", Ty::Id, "The name the command line is stored under.")],
        children: Children::Only(&["format", "replace", "dedupe", "raw", "pair", "pairs"]),
        common: Common::Step,
        ..spec(
            "args",
            Category::Step,
            "Build a command line from pieces. Result: the command line as text.",
        )
    },
    Spec {
        args: &[req(
            "template",
            Ty::Template,
            "The pattern, with {key} and {value}.",
        )],
        ..spec(
            "format",
            Category::Part,
            "How each key/value pair is written. Default: {key}={value}",
        )
    },
    Spec {
        args: &[req("from", Ty::Text, "Text to find."), req("to", Ty::Text, "Replacement.")],
        ..spec(
            "replace",
            Category::Part,
            "Replace text inside every pair value before it is written.",
        )
    },
    Spec {
        args: &[req("keep", Ty::Enum(DEDUPE), "Which to keep.")],
        props: &[pd(
            "ignore-case",
            Ty::Bool,
            "#true",
            "Treat keys that differ only in case as the same.",
        )],
        ..spec(
            "dedupe",
            Category::Part,
            "When two pairs have the same key, keep the first, the last, or both.",
        )
    },
    Spec {
        args: &[req("text", Ty::Template, "The text.")],
        props: &[p("if", Ty::Lua, "Only when this is true.")],
        ..spec(
            "raw",
            Category::Part,
            "Text added to the command line as-is (skipped when empty).",
        )
    },
    Spec {
        args: &[req("key", Ty::Text, "The key."), req("value", Ty::Template, "The value.")],
        props: &[p("if", Ty::Lua, "Only when this is true.")],
        ..spec("pair", Category::Part, "One key/value pair.")
    },
    Spec {
        args: &[req("source", Ty::BagRef, "A data id or a control id.")],
        props: &[
            p(
                "exclude",
                Ty::Text,
                "Keys to leave out, separated by commas.",
            ),
            p("if", Ty::Lua, "Only when this is true."),
        ],
        ..spec(
            "pairs",
            Category::Part,
            "Every key/value of a data block, or of the option selected in a control.",
        )
    },
    Spec {
        args: &[req("run", Ty::RunRef, "The id of the run step.")],
        props: &[
            req("volume", Ty::Template, "0 to 100."),
            pd(
                "timeout",
                Ty::Int,
                "10000",
                "Milliseconds to wait for the program's audio.",
            ),
        ],
        common: Common::Step,
        os: Os::Windows,
        ..spec(
            "set-process-volume",
            Category::Step,
            "Set the audio volume of a program an earlier run step started (waits for it to open its audio).",
        )
    },
];

pub fn get(name: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.name == name)
}

/// Kinds allowed in a block of `children`, for "unknown node" hints and the editor's Add menu.
pub fn allowed(children: Children) -> Vec<&'static str> {
    match children {
        Children::None | Children::Bag => Vec::new(),
        Children::Layout => SPECS
            .iter()
            .filter(|s| matches!(s.cat, Category::Layout | Category::Control))
            .map(|s| s.name)
            .collect(),
        Children::Steps => SPECS
            .iter()
            .filter(|s| s.cat == Category::Step)
            .map(|s| s.name)
            .collect(),
        Children::Only(k) => k.to_vec(),
    }
}

/// Which kinds take an id as their first argument.
pub fn takes_id(kind: &str) -> bool {
    get(kind).is_some_and(|s| s.args.first().is_some_and(|a| a.ty == Ty::Id))
}

pub fn ty_name(t: Ty) -> &'static str {
    match t {
        Ty::Text => "text",
        Ty::Template => "template",
        Ty::Lua => "Lua expression",
        Ty::LuaChunk => "Lua code",
        Ty::LuaValue => "(lua) expression",
        Ty::Int => "integer",
        Ty::Number => "number",
        Ty::Bool => "#true|#false",
        Ty::Enum(_) => "word",
        Ty::Id => "name",
        Ty::ControlRef => "control id",
        Ty::ActionRef => "action id",
        Ty::BagRef => "data or control id",
        Ty::RunRef => "run step id",
        Ty::Any => "value",
        Ty::Version => "version",
        Ty::Width => "width",
    }
}

fn category_title(c: Category) -> &'static str {
    match c {
        Category::Top => "Top level",
        Category::Layout => "Layout",
        Category::Control => "Controls",
        Category::Step => "Steps",
        Category::Part => "Parts (inside a specific block)",
    }
}

fn children_text(c: Children) -> Option<String> {
    match c {
        Children::None => None,
        Children::Bag => Some("`Key \"value\"` lines".to_string()),
        Children::Layout => Some("layout items and controls".to_string()),
        Children::Steps => Some("steps".to_string()),
        Children::Only(k) => Some(
            k.iter()
                .map(|k| format!("`{k}`"))
                .collect::<Vec<_>>()
                .join(", "),
        ),
    }
}

fn param_line(pr: &Param) -> String {
    let mut s = format!("`{}` ({}", pr.name, ty_name(pr.ty));
    if let Ty::Enum(c) = pr.ty {
        s.push_str(&format!(": {}", c.join(" | ")));
    }
    if pr.required {
        s.push_str(", required");
    }
    if !pr.default.is_empty() {
        s.push_str(&format!(", default `{}`", pr.default));
    }
    if pr.os != Os::Any {
        s.push_str(&format!(", {} only", pr.os.name()));
    }
    s.push_str(&format!("): {}", pr.doc));
    s
}

/// `docs/config-reference.md`: the whole catalog as Markdown.
pub fn reference_markdown() -> String {
    let mut out = String::from(
        "# `.launcher` file reference\n\n\
         Generated from `crates/launcher-core/src/config/catalog.rs` (`cargo xtask docs`); do not \
         edit by hand.\n\n\
         A `.launcher` file is KDL v2. Values are plain text, templates with `{name}`, \
         `{name.field}` and `{env.NAME}` placeholders, or Luau written `(lua)\"...\"`. Conditions \
         (`if=`, `enabled-if=`, `visible-if=`) are Luau expressions. Relative paths resolve \
         against the file's folder.\n",
    );
    for cat in [
        Category::Top,
        Category::Layout,
        Category::Control,
        Category::Step,
        Category::Part,
    ] {
        out.push_str(&format!("\n## {}\n", category_title(cat)));
        for s in SPECS.iter().filter(|s| s.cat == cat) {
            out.push_str(&format!("\n### `{}`\n\n{}", s.name, s.doc));
            if s.os != Os::Any {
                out.push_str(&format!(" ({} only)", s.os.name()));
            }
            out.push('\n');
            let mut lines = Vec::new();
            for a in s.args {
                lines.push(format!("- argument {}", param_line(a)));
            }
            if let Some(r) = &s.rest {
                lines.push(format!("- further arguments {}", param_line(r)));
            }
            for pr in s.all_props() {
                lines.push(format!("- {}", param_line(pr)));
            }
            if let Some(k) = children_text(s.children) {
                lines.push(format!("- block: {k}"));
            }
            if s.on_change {
                lines.push("- allowed in `on-change`".to_string());
            }
            if !lines.is_empty() {
                out.push('\n');
                out.push_str(&lines.join("\n"));
                out.push('\n');
            }
        }
    }
    out
}

/// `launcher --help-config`: the catalog as plain text.
pub fn reference_text() -> String {
    let mut out = String::from(".launcher file reference\n========================\n");
    for cat in [
        Category::Top,
        Category::Layout,
        Category::Control,
        Category::Step,
        Category::Part,
    ] {
        let title = category_title(cat);
        out.push_str(&format!("\n{title}\n{}\n", "-".repeat(title.len())));
        for s in SPECS.iter().filter(|s| s.cat == cat) {
            out.push_str(&format!("\n{}: {}\n", s.name, s.doc));
            for a in s.args.iter().chain(s.rest.iter()) {
                out.push_str(&format!("    <{}>  {}\n", a.name, a.doc));
            }
            for pr in s.all_props() {
                let d = if pr.default.is_empty() {
                    String::new()
                } else {
                    format!(" [default {}]", pr.default)
                };
                out.push_str(&format!(
                    "    {}={}{}  {}\n",
                    pr.name,
                    ty_name(pr.ty),
                    d,
                    pr.doc
                ));
            }
            if let Some(k) = children_text(s.children) {
                out.push_str(&format!("    {{ {} }}\n", k.replace('`', "")));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for s in SPECS {
            assert!(seen.insert(s.name), "duplicate spec {}", s.name);
            assert!(!s.doc.is_empty(), "{} has no doc", s.name);
        }
    }

    #[test]
    fn every_named_child_kind_exists() {
        for s in SPECS {
            if let Children::Only(kids) = s.children {
                for k in kids {
                    assert!(get(k).is_some(), "{} names unknown child {k}", s.name);
                }
            }
        }
    }

    #[test]
    fn property_names_are_unique_per_kind() {
        for s in SPECS {
            let mut seen = std::collections::HashSet::new();
            for pr in s.all_props() {
                assert!(seen.insert(pr.name), "{} repeats {}", s.name, pr.name);
            }
        }
    }
}
