//! The typed model of a `.launcher` file. The KDL text stays the source of truth (the editor
//! splices it, N12); this model is rebuilt from it after every change and is what the form, the
//! engine and validation read.
//!
//! Every element keeps a [`NodeRef`]: its index path in the document (for the editor) and its byte
//! span (for diagnostics).

use std::path::PathBuf;

use super::catalog::Os;
use super::template::Template;

/// Index path from the document root: `[2, 0, 5]` is the 6th child of the 1st child of the 3rd
/// top-level node.
pub type NodePath = Vec<usize>;

/// A byte range in the file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub offset: usize,
    pub len: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodeRef {
    pub path: NodePath,
    pub span: Span,
}

/// Luau source from the file, with where it came from (M3 maps Lua errors back to file lines).
#[derive(Clone, Debug, PartialEq)]
pub struct LuaSrc {
    /// The code (the KDL string's value: escapes applied, multi-line strings dedented).
    pub code: String,
    /// The value as written in the file (quotes, `#` and all).
    pub repr: String,
    /// Where `repr` starts and ends in the file.
    pub span: Span,
    /// 1-based line and column of `repr` in the file.
    pub line: usize,
    pub col: usize,
}

/// A value that is a template or `(lua)"expr"`.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    Text(Template),
    Lua(LuaSrc),
}

impl Val {
    pub fn literal(s: &str) -> Val {
        Val::Text(Template::literal(s))
    }

    /// The text when it is a literal with no placeholders.
    pub fn as_literal(&self) -> Option<&str> {
        match self {
            Val::Text(t) if t.is_literal() => Some(&t.raw),
            _ => None,
        }
    }
}

/// A control's default: a literal of the control's kind, a template, or `(lua)`.
#[derive(Clone, Debug, PartialEq)]
pub enum DefaultVal {
    Bool(bool),
    Num(f64),
    Val(Val),
}

/// The header (`launcher requires="1.2" os="windows"`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Header {
    pub requires: Option<Version>,
    pub os: Option<Os>,
    /// Each run's output is saved to files (`save-logs`); otherwise it only shows in the window.
    pub save_logs: bool,
    pub node: Option<NodeRef>,
}

/// A version: major.minor.patch (missing parts are 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let mut parts = s.trim().split('.');
        let mut next = |required: bool| -> Option<u32> {
            match parts.next() {
                Some(p) => p.parse().ok(),
                None if required => None,
                None => Some(0),
            }
        };
        let v = Version(next(true)?, next(false)?, next(false)?);
        parts.next().is_none().then_some(v)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.2 == 0 {
            write!(f, "{}.{}", self.0, self.1)
        } else {
            write!(f, "{}.{}.{}", self.0, self.1, self.2)
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    /// The file, and its folder (what relative paths resolve against, N3).
    pub path: PathBuf,
    pub dir: PathBuf,
    pub header: Header,
    pub window: WindowSpec,
    pub scripts: Vec<LuaSrc>,
    pub values: Vec<ValueDef>,
    pub data: Vec<DataDef>,
    pub layout: Vec<Item>,
    /// Every control, in document order. Layout items refer to them by index.
    pub controls: Vec<Control>,
    pub actions: Vec<Action>,
    /// The `layout` node, if there is one (the editor adds into it).
    pub layout_node: Option<NodeRef>,
    /// 1 for a `launcher` file (the first format), 2 for a `tool` file.
    pub format: u32,
    /// The add-ons the file carries (N22).
    pub carried: Vec<std::sync::Arc<crate::addons::AddOn>>,
}

impl Config {
    pub fn empty(path: PathBuf) -> Config {
        let dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Config {
            path,
            dir,
            header: Header::default(),
            window: WindowSpec::default(),
            scripts: Vec::new(),
            values: Vec::new(),
            data: Vec::new(),
            layout: Vec::new(),
            controls: Vec::new(),
            actions: Vec::new(),
            layout_node: None,
            format: 1,
            carried: Vec::new(),
        }
    }

    pub fn control(&self, id: &str) -> Option<&Control> {
        self.controls.iter().find(|c| c.id == id)
    }

    pub fn action(&self, id: &str) -> Option<&Action> {
        self.actions.iter().find(|a| a.id == id)
    }

    pub fn data_block(&self, id: &str) -> Option<&DataDef> {
        self.data.iter().find(|d| d.id == id)
    }

    pub fn value(&self, id: &str) -> Option<&ValueDef> {
        self.values.iter().find(|v| v.id == id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Theme {
    System,
    Dark,
    Light,
}

#[derive(Clone, Debug)]
pub struct WindowSpec {
    pub title: Val,
    /// Fixed client size (logical px); `None` = fit the content.
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub resizable: bool,
    pub theme: Theme,
    pub label_width: f32,
    pub field_width: f32,
    pub center: bool,
    pub node: Option<NodeRef>,
}

impl Default for WindowSpec {
    fn default() -> Self {
        WindowSpec {
            title: Val::literal("Launcher"),
            width: None,
            height: None,
            resizable: true,
            theme: Theme::System,
            label_width: 140.0,
            field_width: 260.0,
            center: false,
            node: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ValueDef {
    pub id: String,
    pub source: ValueSource,
    pub node: NodeRef,
}

#[derive(Clone, Debug)]
pub enum ValueSource {
    Val(Val),
    Find(FindFile),
}

#[derive(Clone, Debug)]
pub struct FindFile {
    pub pattern: String,
    pub dir: Val,
    pub recursive: bool,
    pub exclude_self: bool,
    pub require_sibling: Option<Val>,
}

/// A `Key "value"` list. Keys compare case-insensitively.
pub type Bag = Vec<(String, String)>;

#[derive(Clone, Debug)]
pub struct DataDef {
    pub id: String,
    pub bag: Bag,
    pub node: NodeRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteStyle {
    Normal,
    Muted,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub enum Item {
    Group {
        title: Option<Val>,
        visible_if: Option<LuaSrc>,
        items: Vec<Item>,
        node: NodeRef,
    },
    Tabs {
        tabs: Vec<Tab>,
        node: NodeRef,
    },
    Columns {
        columns: Vec<Vec<Item>>,
        node: NodeRef,
    },
    Row {
        visible_if: Option<LuaSrc>,
        items: Vec<Item>,
        node: NodeRef,
    },
    Separator {
        title: Option<Val>,
        node: NodeRef,
    },
    Spacer {
        height: f32,
        node: NodeRef,
    },
    Note {
        text: Val,
        style: NoteStyle,
        visible_if: Option<LuaSrc>,
        node: NodeRef,
    },
    /// Index into [`Config::controls`].
    Control(usize),
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub title: Val,
    pub visible_if: Option<LuaSrc>,
    pub items: Vec<Item>,
    pub node: NodeRef,
}

/// A field's width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Width {
    /// The form's field column (the default).
    Field,
    /// To the form's right edge.
    Fill,
    /// As wide as the content needs.
    Auto,
    Px(f32),
}

#[derive(Clone, Debug)]
pub struct Control {
    pub id: String,
    pub kind: ControlKind,
    pub label: Option<String>,
    pub tooltip: Option<Val>,
    pub default: Option<DefaultVal>,
    pub persist: bool,
    pub visible_if: Option<LuaSrc>,
    pub enabled_if: Option<LuaSrc>,
    pub disabled_tooltip: Option<Val>,
    pub required: bool,
    pub width: Width,
    pub on_change: Vec<Step>,
    pub node: NodeRef,
}

impl Control {
    pub fn kind_name(&self) -> &'static str {
        self.kind.name()
    }

    /// Holds a value (buttons and displays do not).
    pub fn has_value(&self) -> bool {
        !matches!(
            self.kind,
            ControlKind::Display { .. } | ControlKind::Button { .. }
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChoiceStyle {
    Dropdown,
    Radio { horizontal: bool },
    Checklist,
}

#[derive(Clone, Debug)]
pub struct OptionDef {
    pub name: String,
    pub value: String,
    pub label: Option<Val>,
    pub bag: Bag,
    pub node: NodeRef,
}

#[derive(Clone, Debug)]
pub struct Preset {
    pub value: Val,
    pub match_field: Option<String>,
    pub node: NodeRef,
}

#[derive(Clone, Debug)]
pub enum ControlKind {
    Choice {
        style: ChoiceStyle,
        options: Vec<OptionDef>,
        /// `options=(lua)"..."`: computed when the file loads.
        options_lua: Option<LuaSrc>,
        /// `list-from "git.branches"`: an add-on's list (N21), computed when the file loads.
        list: Option<ListRef>,
        option_label: Option<Val>,
        presets: Vec<Preset>,
    },
    Checkbox,
    Slider {
        min: f64,
        max: f64,
        step: f64,
        suffix: Option<String>,
    },
    Number {
        min: Option<f64>,
        max: Option<f64>,
        step: f64,
        decimals: u32,
    },
    Text {
        multiline: bool,
        lines: u32,
        password: bool,
        placeholder: Option<String>,
        pattern: Option<String>,
        pattern_message: Option<String>,
    },
    File {
        filter: Option<String>,
        save: bool,
        must_exist: bool,
        relative: bool,
        placeholder: Option<String>,
    },
    Folder {
        must_exist: bool,
        relative: bool,
        placeholder: Option<String>,
    },
    Display {
        text: Val,
        copyable: bool,
    },
    Button {
        action: String,
        confirm: Option<Val>,
        primary: bool,
    },
}

impl ControlKind {
    pub fn name(&self) -> &'static str {
        match self {
            ControlKind::Choice { style, .. } => match style {
                ChoiceStyle::Dropdown => "dropdown",
                ChoiceStyle::Radio { .. } => "radio",
                ChoiceStyle::Checklist => "checklist",
            },
            ControlKind::Checkbox => "checkbox",
            ControlKind::Slider { .. } => "slider",
            ControlKind::Number { .. } => "number",
            ControlKind::Text { .. } => "text",
            ControlKind::File { .. } => "file",
            ControlKind::Folder { .. } => "folder",
            ControlKind::Display { .. } => "display",
            ControlKind::Button { .. } => "button",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum After {
    Stay,
    Exit,
    Minimize,
}

#[derive(Clone, Debug)]
pub struct Action {
    pub id: String,
    pub label: Option<String>,
    pub after: After,
    pub lock: Option<Val>,
    pub steps: Vec<Step>,
    pub node: NodeRef,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnError {
    Abort,
    Warn,
    Continue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Debug,
    Info,
    Warning,
    Error,
}

/// An add-on list where a control uses it, with values for its settings.
#[derive(Clone, Debug)]
pub struct ListRef {
    pub def: std::sync::Arc<crate::addons::ListDef>,
    pub settings: Vec<(String, Val)>,
}

/// An add-on step where a tool uses it.
#[derive(Clone, Debug)]
pub struct UseStep {
    pub def: std::sync::Arc<crate::addons::StepDef>,
    /// Its settings as the tool sets them (the rest take their starting values).
    pub settings: Vec<(String, Val)>,
}

#[derive(Clone, Debug)]
pub struct Step {
    pub kind: StepKind,
    /// "Before starting, make sure…": runs before every other step, like a `check`.
    pub first: bool,
    pub id: Option<String>,
    pub label: Option<Val>,
    pub if_: Option<LuaSrc>,
    pub on_error: OnError,
    pub message: Option<Val>,
    pub node: NodeRef,
}

/// Exit codes a `run` or `check` accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpectExit {
    Any,
    Codes(Vec<i64>),
}

impl ExpectExit {
    pub fn accepts(&self, code: i64) -> bool {
        match self {
            ExpectExit::Any => true,
            ExpectExit::Codes(c) => c.contains(&code),
        }
    }

    pub fn parse(s: &str) -> Option<ExpectExit> {
        if s.trim().eq_ignore_ascii_case("any") {
            return Some(ExpectExit::Any);
        }
        let codes: Option<Vec<i64>> = s.split(',').map(|c| c.trim().parse::<i64>().ok()).collect();
        codes.filter(|c| !c.is_empty()).map(ExpectExit::Codes)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Io {
    Auto,
    Pty,
    Pipe,
}

#[derive(Clone, Debug)]
pub struct RunStep {
    pub program: Val,
    pub args: Vec<Val>,
    pub command_line: Option<Val>,
    pub cwd: Option<Val>,
    pub stdin: Option<Val>,
    pub wait: bool,
    pub timeout_ms: Option<u64>,
    pub expect_exit: ExpectExit,
    pub io: Io,
    pub fail_if_output: Option<String>,
    pub env: Vec<(String, Val)>,
}

#[derive(Clone, Debug)]
pub struct Extract {
    pub name: String,
    pub path: String,
    pub required: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteFormat {
    Text,
    JsonPretty,
    JsonCompact,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Newline {
    Keep,
    Lf,
    Crlf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dedupe {
    First,
    Last,
    None,
}

/// The `args` step: a command line built from pieces.
#[derive(Clone, Debug)]
pub struct ArgsSpec {
    pub name: String,
    pub format: Option<Template>,
    pub replace: Vec<(String, String)>,
    pub dedupe: Dedupe,
    pub dedupe_ignore_case: bool,
    pub parts: Vec<ArgPart>,
}

#[derive(Clone, Debug)]
pub enum ArgPart {
    Raw {
        text: Val,
        if_: Option<LuaSrc>,
    },
    Pair {
        key: String,
        value: Val,
        if_: Option<LuaSrc>,
    },
    Pairs {
        source: String,
        exclude: Vec<String>,
        if_: Option<LuaSrc>,
    },
}

#[derive(Clone, Debug)]
pub enum StepKind {
    Run(Box<RunStep>),
    /// Run a program quietly and keep its output: {id.output}, {id.exit_code}.
    Capture {
        program: Val,
        args: Vec<Val>,
        cwd: Option<Val>,
        expect_exit: ExpectExit,
    },
    /// A step from an add-on (N21): its settings, worked out where it's used, then its body in
    /// a scope of its own.
    Use(Box<UseStep>),
    Check {
        program: Val,
        args: Vec<Val>,
        cwd: Option<Val>,
        expect_exit: ExpectExit,
    },
    EnvFrom {
        script: Val,
        args: Vec<Val>,
    },
    Set {
        name: String,
        value: Val,
    },
    /// A step's result into a control: a list into a choice's choices, anything else as its
    /// value (tools' `put "result" into="control"`).
    Put {
        control: String,
        step: String,
        result: String,
    },
    SetControl {
        control: String,
        value: Val,
    },
    Reset {
        control: String,
    },
    Copy {
        from: Val,
        to: Val,
        overwrite: bool,
        ask_first: bool,
    },
    Move {
        from: Val,
        to: Val,
        overwrite: bool,
    },
    Delete {
        path: Val,
        missing_ok: bool,
        to_trash: bool,
    },
    Mkdir {
        path: Val,
    },
    FindFile(FindFile),
    ReadFile {
        path: Val,
    },
    WriteFile {
        path: Val,
        content: Val,
        format: WriteFormat,
        newline: Newline,
        create_dirs: bool,
        /// A line above the content.
        heading: Option<Val>,
        /// Add to the file; the run's first write to it still starts it, unless `keep`.
        append: bool,
        keep: bool,
        /// What a list in the content holds, one and several: "model", "models".
        item: (String, String),
    },
    HttpGet {
        url: Val,
        timeout_ms: u64,
        extract: Vec<Extract>,
        headers: Vec<(String, Val)>,
    },
    Ping {
        host: Val,
        timeout_ms: u64,
    },
    Open {
        target: Val,
    },
    Reveal {
        path: Val,
    },
    Message {
        text: Val,
        level: Level,
    },
    Confirm {
        text: Val,
        yes: String,
        no: String,
    },
    Ask {
        name: String,
        prompt: Val,
        default: Option<Val>,
        password: bool,
    },
    Wait {
        ms: u64,
    },
    Log {
        text: Val,
        level: Level,
    },
    Fail {
        text: Val,
    },
    Stop,
    Lua {
        code: LuaSrc,
    },
    Args(ArgsSpec),
    SetProcessVolume {
        run: String,
        volume: Val,
        timeout_ms: u64,
    },
}

impl StepKind {
    pub fn name(&self) -> &'static str {
        match self {
            StepKind::Run(_) => "run",
            StepKind::Capture { .. } => "capture",
            StepKind::Use(_) => "use",
            StepKind::Check { .. } => "check",
            StepKind::EnvFrom { .. } => "env-from",
            StepKind::Set { .. } => "set",
            StepKind::SetControl { .. } => "set-control",
            StepKind::Put { .. } => "put",
            StepKind::Reset { .. } => "reset",
            StepKind::Copy { .. } => "copy",
            StepKind::Move { .. } => "move",
            StepKind::Delete { .. } => "delete",
            StepKind::Mkdir { .. } => "mkdir",
            StepKind::FindFile(_) => "find-file",
            StepKind::ReadFile { .. } => "read-file",
            StepKind::WriteFile { .. } => "write-file",
            StepKind::HttpGet { .. } => "http-get",
            StepKind::Ping { .. } => "ping",
            StepKind::Open { .. } => "open",
            StepKind::Reveal { .. } => "reveal",
            StepKind::Message { .. } => "message",
            StepKind::Confirm { .. } => "confirm",
            StepKind::Ask { .. } => "ask",
            StepKind::Wait { .. } => "wait",
            StepKind::Log { .. } => "log",
            StepKind::Fail { .. } => "fail",
            StepKind::Stop => "stop",
            StepKind::Lua { .. } => "lua",
            StepKind::Args(_) => "args",
            StepKind::SetProcessVolume { .. } => "set-process-volume",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_parse_and_compare() {
        assert_eq!(Version::parse("1"), Some(Version(1, 0, 0)));
        assert_eq!(Version::parse("1.2"), Some(Version(1, 2, 0)));
        assert_eq!(Version::parse("1.2.3"), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("1.x"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert!(Version(1, 10, 0) > Version(1, 9, 5));
        assert_eq!(Version(1, 2, 0).to_string(), "1.2");
    }

    #[test]
    fn exit_codes_parse() {
        assert_eq!(ExpectExit::parse("0"), Some(ExpectExit::Codes(vec![0])));
        assert_eq!(
            ExpectExit::parse("0, 1"),
            Some(ExpectExit::Codes(vec![0, 1]))
        );
        assert_eq!(ExpectExit::parse("any"), Some(ExpectExit::Any));
        assert_eq!(ExpectExit::parse("x"), None);
        assert!(ExpectExit::Codes(vec![0, 2]).accepts(2));
    }
}
