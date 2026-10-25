//! Add-ons (N21): text files of extra steps. An add-on is one `add-on "<name>" { }` block of
//! plain text: its steps (what each needs to know, what it gives back, what it needs on the
//! computer, and what it does), and its lists (where a control's choices come from).
//!
//! * **Built in:** `addons/*.kdl` in the repository, compiled in. Their steps may be backed by the
//!   engine's own steps (`built-in="copy"`); the settings then come from the catalog.
//! * **The Add-ons folder:** `<app data>/add-ons/*.kdl`, read when a tool opens.
//! * **Carried:** a tool may hold copies of the add-ons it uses (N22), so it works anywhere. A
//!   carried copy wins over an installed one.
//!
//! Text only, on purpose: no images, no other files, scripts inline. That is what lets a tool
//! carry an add-on word for word.

mod parse;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock, RwLock};

use crate::config::catalog::{Os, Spec};
use crate::config::{Diagnostic, LoadOptions, LuaSrc, Step, Val, Version};
use crate::dynval::Dyn;

pub use parse::{parse_addon, parse_addon_node};

/// Where an add-on came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    BuiltIn,
    /// A file in the Add-ons folder.
    Folder(PathBuf),
    /// A copy inside a tool.
    Carried,
}

/// A rule that suggests an add-on for a project folder (evaluated in Rust: no trust needed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detect {
    /// A file or folder at this path, relative to the project folder.
    Exists(String),
    /// Any file matching this pattern in the project folder.
    Glob(String),
}

#[derive(Debug)]
pub struct AddOn {
    /// Its short name: the part before the dot in `git.pull`.
    pub id: String,
    pub name: String,
    pub version: Version,
    /// The oldest Launcher it works with.
    pub requires: Option<Version>,
    pub icon: Option<String>,
    pub about: Option<String>,
    pub suggest_if: Vec<Detect>,
    pub lists: Vec<Arc<ListDef>>,
    pub steps: Vec<Arc<StepDef>>,
    pub origin: Origin,
    /// The `add-on { }` block exactly as written (what "Include add-ons in this tool" copies).
    pub text: String,
}

/// What kind of value a setting holds; it picks the editor and the control "Ask each time" adds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingKind {
    Text,
    LongText,
    Number,
    TickBox,
    Choice,
    Choices,
    File,
    Folder,
    /// Several values (a program's arguments).
    List,
    /// Rows of name and value (environment variables, headers).
    Table,
}

impl SettingKind {
    pub const WORDS: &'static [&'static str] = &[
        "text",
        "long-text",
        "number",
        "tick-box",
        "choice",
        "choices",
        "file",
        "folder",
        "list",
        "table",
    ];

    pub fn parse(s: &str) -> Option<SettingKind> {
        Some(match s {
            "text" => SettingKind::Text,
            "long-text" => SettingKind::LongText,
            "number" => SettingKind::Number,
            "tick-box" => SettingKind::TickBox,
            "choice" => SettingKind::Choice,
            "choices" => SettingKind::Choices,
            "file" => SettingKind::File,
            "folder" => SettingKind::Folder,
            "list" => SettingKind::List,
            "table" => SettingKind::Table,
            _ => return None,
        })
    }

    pub fn word(self) -> &'static str {
        Self::WORDS[self as usize]
    }

    /// Whether a setting of this kind can read what people chose in a control of that kind
    /// (`folder`, `tick-box`...): a folder setting only a folder, a file only a file. Text takes
    /// any one value as its text; lists and tables don't read a control.
    pub fn takes(self, control: &str) -> bool {
        match self {
            SettingKind::Text | SettingKind::LongText => {
                matches!(control, "text" | "number" | "file" | "folder" | "choice")
            }
            SettingKind::List | SettingKind::Table => false,
            _ => control == self.word(),
        }
    }

    /// What it needs, as people say it: "a folder", "a tick box".
    pub fn needs(self) -> &'static str {
        match self {
            SettingKind::Text | SettingKind::LongText => "text",
            SettingKind::Number => "a number",
            SettingKind::TickBox => "a tick box",
            SettingKind::Choice => "one choice",
            SettingKind::Choices => "a set of choices",
            SettingKind::File => "a file",
            SettingKind::Folder => "a folder",
            SettingKind::List => "a list",
            SettingKind::Table => "a table",
        }
    }
}

/// Something a step needs to know.
#[derive(Clone, Debug)]
pub struct SettingDef {
    pub id: String,
    pub kind: SettingKind,
    pub label: String,
    pub help: Option<String>,
    /// Its value until someone sets it.
    pub starts: Option<Dyn>,
    pub required: bool,
    /// Shown in the collapsed Advanced section.
    pub advanced: bool,
    /// A choice's fixed options.
    pub options: Vec<String>,
    /// A choice whose options come from a list.
    pub list_from: Option<ListUse>,
    /// A file setting's extensions (`json`), lower case without the dot; empty takes any file.
    pub extensions: Vec<String>,
    /// A setting that also takes several: a file or folder setting a list of files an earlier
    /// step found, a text setting any list (one item per line).
    pub several: bool,
    /// For built-in steps: the catalog argument this setting fills (by position), or `rest`.
    pub position: Option<usize>,
    pub rest: bool,
    /// For built-in steps: the add-on changed the starting value, so a tool that doesn't set it
    /// still gets the add-on's (Delete moves to the Recycle Bin unless told otherwise).
    pub starts_overridden: bool,
}

impl SettingDef {
    /// Whether it can read what people chose in a control of kind `word` that takes files with
    /// `control_exts` (empty: any file) and offers `control_options` (empty: none written, a step
    /// fills them). Its kind first ([`SettingKind::takes`]); then a setting that names extensions
    /// takes only a file control limited to some of them, and one with fixed options only a list
    /// whose options are all among them.
    pub fn takes(&self, word: &str, control_exts: &[String], control_options: &[String]) -> bool {
        if self.text_lines() && word == "choices" {
            return true;
        }
        self.kind.takes(word)
            && (self.kind != SettingKind::File
                || self.extensions.is_empty()
                || (!control_exts.is_empty()
                    && control_exts.iter().all(|e| self.extensions.contains(e))))
            && (!matches!(self.kind, SettingKind::Choice | SettingKind::Choices)
                || self.options.is_empty()
                || control_options.iter().all(|o| self.options.contains(o)))
    }

    /// What's wrong with a value typed for it (no `{…}` in it), in words that follow its name
    /// ("should be a number"); None when it's fine.
    pub fn check_value(&self, text: &str) -> Option<String> {
        let t = text.trim();
        if t.is_empty() {
            return None;
        }
        match self.kind {
            SettingKind::Number if t.parse::<f64>().is_err() => {
                Some(format!("should be a number, not \"{t}\""))
            }
            SettingKind::File if !crate::paths::has_extension(t, &self.extensions) => {
                Some(format!(
                    "should be a {} file",
                    crate::paths::extension_words(&self.extensions)
                ))
            }
            SettingKind::Choice
                if !self.options.is_empty() && !self.options.iter().any(|o| o == t) =>
            {
                Some(format!("should be one of: {}", self.options.join(", ")))
            }
            _ => None,
        }
    }

    /// Whether it can use what an earlier step found, of that kind: a folder setting a folder
    /// (and a list of files when it takes several), text any one value, a number a number.
    pub fn takes_result(&self, kind: ResultKind) -> bool {
        use ResultKind as R;
        match self.kind {
            SettingKind::Text | SettingKind::LongText => {
                matches!(kind, R::Text | R::Number | R::File | R::Folder)
                    || (self.several && matches!(kind, R::Files | R::List))
            }
            SettingKind::Number => kind == R::Number,
            // Fixed options: what a step finds could be anything, so only an app's list.
            SettingKind::Choice => kind == R::Text && self.options.is_empty(),
            SettingKind::Choices => kind == R::List && self.options.is_empty(),
            SettingKind::File => kind == R::File || (self.several && kind == R::Files),
            SettingKind::Folder => kind == R::Folder || (self.several && kind == R::Files),
            SettingKind::TickBox | SettingKind::List | SettingKind::Table => false,
        }
    }

    /// What it needs, as people say it: "a folder", "a .json file".
    pub fn needs(&self) -> String {
        let one = if self.kind == SettingKind::File && !self.extensions.is_empty() {
            format!("a {} file", crate::paths::extension_words(&self.extensions))
        } else {
            self.kind.needs().to_string()
        };
        if self.text_lines() {
            "text or a list".to_string()
        } else if self.several {
            format!("{one} or a list of files")
        } else {
            one
        }
    }

    /// Text that takes a list, one item per line.
    pub fn text_lines(&self) -> bool {
        self.several && matches!(self.kind, SettingKind::Text | SettingKind::LongText)
    }
}

/// What kind of value a step gives back. Settings use only results they can take
/// ([`SettingDef::takes_result`]): a count can't be a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultKind {
    Text,
    Number,
    File,
    Folder,
    /// Several files or folders (the models an audit found to fix).
    Files,
    /// Several pieces of text (branch names).
    List,
}

impl ResultKind {
    pub const WORDS: &'static [&'static str] =
        &["text", "number", "file", "folder", "files", "list"];

    pub fn parse(s: &str) -> Option<ResultKind> {
        Some(match s {
            "text" => ResultKind::Text,
            "number" => ResultKind::Number,
            "file" => ResultKind::File,
            "folder" => ResultKind::Folder,
            "files" => ResultKind::Files,
            "list" => ResultKind::List,
            _ => return None,
        })
    }

    pub fn word(self) -> &'static str {
        Self::WORDS[self as usize]
    }

    /// Whether "Put it into" can put it into a control of kind `word`: a list fills a drop-down's
    /// or tick boxes' choices; one value goes into a control of its kind, or into text.
    pub fn fills(self, word: &str) -> bool {
        match self {
            ResultKind::List | ResultKind::Files => matches!(word, "choice" | "choices"),
            ResultKind::Number => matches!(word, "number" | "text"),
            ResultKind::Text => matches!(word, "text" | "choice"),
            ResultKind::File => matches!(word, "file" | "text"),
            ResultKind::Folder => matches!(word, "folder" | "text"),
        }
    }

    /// What it is, as people say it: "a number", "a list of files".
    pub fn words(self) -> &'static str {
        match self {
            ResultKind::Text => "text",
            ResultKind::Number => "a number",
            ResultKind::File => "a file",
            ResultKind::Folder => "a folder",
            ResultKind::Files => "a list of files",
            ResultKind::List => "a list",
        }
    }
}

/// What a step gives back: `{id.result}` for later steps.
#[derive(Clone, Debug)]
pub struct ResultDef {
    pub id: String,
    pub label: String,
    pub kind: ResultKind,
    /// What a list holds, one and several (`item="model"`): "model", "models".
    pub item: (String, String),
}

/// What a step needs on the computer.
#[derive(Clone, Debug, PartialEq)]
pub enum Need {
    /// A program found on PATH (or by its install), with what to tell people when it isn't.
    App { name: String, help: Option<String> },
    /// The step only works on this OS.
    Os(Os),
}

#[derive(Debug)]
pub enum Body {
    /// Steps in shorthand: the engine's own steps, with `{setting}` names.
    Does(Vec<Step>),
    /// Luau: gets `settings`, returns its results as a table.
    Script(LuaSrc),
    /// One of the engine's own steps (built-in add-ons only).
    BuiltIn(&'static Spec),
}

/// One thing a tool can do.
#[derive(Debug)]
pub struct StepDef {
    /// `git.pull`.
    pub id: String,
    pub name: String,
    /// The sentence on its card, with `{setting}` names: "Switch to {branch}".
    pub says: Option<String>,
    pub about: Option<String>,
    pub icon: Option<String>,
    pub search_words: Vec<String>,
    /// Steps that usually follow this one ("What comes next" in the step picker).
    pub comes_next: Vec<String>,
    pub settings: Vec<SettingDef>,
    pub results: Vec<ResultDef>,
    pub needs: Vec<Need>,
    /// A phrase in a failed step's output, and the plain advice for it.
    pub explain: Vec<(String, String)>,
    pub body: Body,
    /// It belongs in "Before starting, make sure…".
    pub check_step: bool,
    /// An instant step: no row of its own in the run view.
    pub quiet: bool,
    /// Listed under Advanced in the step picker.
    pub advanced: bool,
    pub replaced_by: Option<String>,
    /// The add-on's display name ("Git"), for the picker and cards.
    pub app: String,
}

impl StepDef {
    pub fn setting(&self, id: &str) -> Option<&SettingDef> {
        self.settings.iter().find(|s| s.id == id)
    }
}

/// Where a control's choices come from: Luau that gets `settings` and returns a list.
#[derive(Debug)]
pub struct ListDef {
    /// `git.branches`.
    pub id: String,
    pub about: Option<String>,
    pub settings: Vec<SettingDef>,
    pub script: LuaSrc,
}

/// A list, with values for its settings (templates, so they can name controls).
#[derive(Clone, Debug)]
pub struct ListUse {
    pub list: String,
    pub settings: Vec<(String, Val)>,
}

/// A problem in an add-on file.
#[derive(Clone, Debug)]
pub struct AddOnProblem {
    pub file: String,
    pub diags: Vec<Diagnostic>,
}

/// Every add-on a tool can use.
#[derive(Debug, Default, Clone)]
pub struct Library {
    pub addons: Vec<Arc<AddOn>>,
    /// Add-on files that couldn't be read (shown in the step picker; they block nothing).
    pub problems: Vec<AddOnProblem>,
}

/// The built-in add-on files: `(file name, text)`.
mod embedded {
    include!(concat!(env!("OUT_DIR"), "/addons.rs"));
}

impl Library {
    /// The add-ons compiled into Launcher.
    pub fn builtin() -> Arc<Library> {
        static LIB: OnceLock<Arc<Library>> = OnceLock::new();
        LIB.get_or_init(|| {
            let mut lib = Library::default();
            let opts = LoadOptions::default();
            for (file, text) in embedded::BUILTIN {
                let (addon, diags) = parse_addon(text, Path::new(file), Origin::BuiltIn, &opts);
                if diags.iter().any(Diagnostic::is_error) || addon.is_none() {
                    lib.problems.push(AddOnProblem {
                        file: file.to_string(),
                        diags,
                    });
                }
                if let Some(a) = addon {
                    lib.addons.push(Arc::new(a));
                }
            }
            Arc::new(lib)
        })
        .clone()
    }

    /// The add-ons tools load with: the built-in ones and the Add-ons folder's, read the first
    /// time they're needed and again after [`Library::reload`].
    pub fn current() -> Library {
        if let Some(l) = CURRENT.read().ok().and_then(|c| c.clone()) {
            return (*l).clone();
        }
        Library::reload()
    }

    /// Read the Add-ons folder again (the step picker's Reload, or a tool opening).
    pub fn reload() -> Library {
        let folder = crate::paths::app_data_subdir("add-ons");
        let lib = Library::with_folder(folder.as_deref());
        if let Ok(mut c) = CURRENT.write() {
            *c = Some(Arc::new(lib.clone()));
        }
        lib
    }

    /// The built-in add-ons plus the ones in `folder` (the Add-ons folder).
    pub fn with_folder(folder: Option<&Path>) -> Library {
        let mut lib = (*Library::builtin()).clone();
        let Some(folder) = folder else {
            return lib;
        };
        let Ok(rd) = std::fs::read_dir(folder) else {
            return lib;
        };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "kdl"))
            .collect();
        files.sort();
        let opts = LoadOptions::default();
        let mut installed: Vec<AddOn> = Vec::new();
        for path in files {
            let file = path.display().to_string();
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    lib.problems.push(AddOnProblem {
                        file,
                        diags: vec![plain_diag(
                            "unreadable",
                            format!("This add-on couldn't be read: {e}"),
                        )],
                    });
                    continue;
                }
            };
            let (addon, diags) = parse_addon(&text, &path, Origin::Folder(path.clone()), &opts);
            if !diags.is_empty() {
                lib.problems.push(AddOnProblem {
                    file: file.clone(),
                    diags: diags.clone(),
                });
            }
            let Some(a) = addon else { continue };
            if diags.iter().any(Diagnostic::is_error) {
                continue;
            }
            if lib.addon(&a.id).is_some() {
                lib.problems.push(AddOnProblem {
                    file,
                    diags: vec![plain_diag(
                        "name-taken",
                        format!(
                            "The add-on \"{}\" has the same name as one built into Launcher, so it isn't used. Give it another name.",
                            a.id
                        ),
                    )],
                });
                continue;
            }
            installed.push(a);
        }
        // Two installed add-ons with one name: the higher version wins; a tie loads neither.
        let mut by_id: BTreeMap<String, Vec<AddOn>> = BTreeMap::new();
        for a in installed {
            by_id.entry(a.id.clone()).or_default().push(a);
        }
        for (id, mut list) in by_id {
            list.sort_by_key(|a| std::cmp::Reverse(a.version));
            if list.len() > 1 && list[0].version == list[1].version {
                let files: Vec<String> = list
                    .iter()
                    .filter_map(|a| match &a.origin {
                        Origin::Folder(p) => Some(p.display().to_string()),
                        _ => None,
                    })
                    .collect();
                lib.problems.push(AddOnProblem {
                    file: files.join(", "),
                    diags: vec![plain_diag(
                        "same-add-on-twice",
                        format!(
                            "Two files are the add-on \"{id}\", version {}, so neither is used. Keep one of them.",
                            list[0].version
                        ),
                    )],
                });
                continue;
            }
            lib.addons.push(Arc::new(list.remove(0)));
        }
        lib
    }

    /// The library as seen by one tool: its carried copies replace installed add-ons of the
    /// same name (built-in ones can't be carried).
    pub fn with_carried(&self, carried: Vec<AddOn>) -> Library {
        let mut lib = self.clone();
        for a in carried {
            lib.addons
                .retain(|x| x.id != a.id || x.origin == Origin::BuiltIn);
            if lib.addon(&a.id).is_none() {
                lib.addons.push(Arc::new(a));
            }
        }
        lib
    }

    /// The same, from copies already read (a loaded tool's `carried`).
    pub fn with_carried_arcs(&self, carried: &[Arc<AddOn>]) -> Library {
        let mut lib = self.clone();
        for a in carried {
            lib.addons
                .retain(|x| x.id != a.id || x.origin == Origin::BuiltIn);
            if lib.addon(&a.id).is_none() {
                lib.addons.push(a.clone());
            }
        }
        lib
    }

    /// The add-ons whose apps are in this folder (their `suggest-if` rules).
    pub fn found_in(&self, folder: &Path) -> Vec<String> {
        self.addons
            .iter()
            .filter(|a| {
                a.suggest_if.iter().any(|d| match d {
                    // A project marker may be higher up (a tool in a repository's subfolder).
                    Detect::Exists(p) => folder.ancestors().any(|d| d.join(p).exists()),
                    Detect::Glob(g) => !crate::engine::effects::glob_paths(folder, g).is_empty(),
                })
            })
            .map(|a| a.id.clone())
            .collect()
    }

    pub fn addon(&self, id: &str) -> Option<&Arc<AddOn>> {
        self.addons.iter().find(|a| a.id == id)
    }

    /// A step by its full name, `git.pull`.
    pub fn step(&self, id: &str) -> Option<Arc<StepDef>> {
        let (addon, _) = id.split_once('.')?;
        self.addon(addon)?
            .steps
            .iter()
            .find(|s| s.id == id)
            .cloned()
    }

    pub fn list(&self, id: &str) -> Option<Arc<ListDef>> {
        let (addon, _) = id.split_once('.')?;
        self.addon(addon)?
            .lists
            .iter()
            .find(|l| l.id == id)
            .cloned()
    }

    pub fn steps(&self) -> impl Iterator<Item = &Arc<StepDef>> {
        self.addons.iter().flat_map(|a| a.steps.iter())
    }

    /// Every step name, for "did you mean".
    pub fn step_names(&self) -> Vec<&str> {
        self.steps().map(|s| s.id.as_str()).collect()
    }
}

/// `docs/step-reference.md`: every built-in step, by add-on, in the app's own words.
pub fn reference_markdown() -> String {
    use std::fmt::Write;
    let lib = Library::builtin();
    let mut out = String::from(
        "# Step reference\n\nEvery step built into Launcher, by add-on. In a tool file a step is written \
`add-on.step \"name\" setting=value`; settings marked *needed* must be set (or asked each \
time). Generated by `cargo xtask docs` from `addons/*.kdl`; don't edit by hand.\n",
    );
    for a in &lib.addons {
        let _ = write!(out, "\n## {} (`{}`)\n\n", a.name, a.id);
        if let Some(t) = &a.about {
            let _ = writeln!(out, "{t}\n");
        }
        for s in &a.steps {
            let _ = write!(out, "### {} — `{}`\n\n", s.name, s.id);
            if let Some(t) = &s.about {
                let _ = writeln!(out, "{t}\n");
            }
            if let Some(t) = &s.says {
                let _ = writeln!(out, "On its card: *{t}*\n");
            }
            if !s.settings.is_empty() {
                out.push_str("| Setting | Kind | |\n|---|---|---|\n");
                for st in &s.settings {
                    let mut notes = Vec::new();
                    if st.required && st.starts.is_none() {
                        notes.push("*needed*".to_string());
                    }
                    if let Some(v) = &st.starts {
                        notes.push(format!("starts as `{}`", v.to_text()));
                    }
                    if st.text_lines() {
                        notes.push("or a list, one item per line".into());
                    } else if st.several {
                        notes.push("or a list of files an earlier step found".into());
                    }
                    if !st.extensions.is_empty() {
                        notes.push(format!(
                            "{} files",
                            crate::paths::extension_words(&st.extensions)
                        ));
                    }
                    if st.advanced {
                        notes.push("Advanced".into());
                    }
                    if let Some(h) = &st.help {
                        notes.push(h.replace('|', "\\|"));
                    }
                    let _ = writeln!(
                        out,
                        "| `{}` ({}) | {} | {} |",
                        st.id,
                        st.label,
                        st.kind.word(),
                        notes.join(". ")
                    );
                }
                out.push('\n');
            }
            if !s.results.is_empty() {
                let r: Vec<String> = s
                    .results
                    .iter()
                    .map(|r| format!("`{{name.{}}}` ({}, {})", r.id, r.label, r.kind.words()))
                    .collect();
                let _ = writeln!(out, "Gives back: {}\n", r.join(", "));
            }
            let needs: Vec<String> = s
                .needs
                .iter()
                .map(|n| match n {
                    Need::App { name, .. } => format!("the app `{name}`"),
                    Need::Os(os) => format!("{os:?}"),
                })
                .collect();
            if !needs.is_empty() {
                let _ = writeln!(out, "Needs: {}\n", needs.join(", "));
            }
        }
    }
    out
}

/// Words people shouldn't meet in the app (N19): developer words with a plain one instead.
/// Checked as whole words, ignoring case, by the plain-words tests.
pub const NEVER_WORDS: &[&str] = &[
    "node",
    "flow",
    "canvas",
    "lane",
    "pack",
    "provider",
    "binding",
    // Not "input": "Text Input" and "Number Input" are the names people asked for.
    "output",
    "template",
    "inspector",
    "form",
    "config",
    "kdl",
    "lua",
    "argument",
    "arguments",
    "flag",
    "parameter",
    "exit code",
    "stdout",
    "stderr",
    "process",
    "execute",
    "path variable",
    "registry",
    "regex",
    "glob",
    "boolean",
    "string",
    "null",
];

/// The developer words in a text.
pub fn jargon(text: &str) -> Vec<&'static str> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '-'))
        .collect();
    NEVER_WORDS
        .iter()
        .copied()
        .filter(|w| {
            if w.contains(' ') {
                lower.contains(w)
            } else {
                words.contains(w)
            }
        })
        .collect()
}

static CURRENT: RwLock<Option<Arc<Library>>> = RwLock::new(None);

fn plain_diag(code: &'static str, message: String) -> Diagnostic {
    Diagnostic {
        severity: crate::config::Severity::Error,
        code,
        message,
        help: None,
        offset: 0,
        len: 0,
        line: 1,
        col: 1,
        path: None,
    }
}

#[cfg(test)]
mod tests;
