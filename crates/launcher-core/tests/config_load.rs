//! Loading `.launcher` text into the model: a file using every feature loads without a single
//! diagnostic, and the model holds what the file says.

use std::path::Path;

use launcher_core::config::catalog::Os;
use launcher_core::config::{self, *};

const FULL: &str = r##"
// A file that uses every feature.
launcher requires="0.1" os="windows"
window title="Build {branch}" label-width=150 theme="dark"

script #"""
    function branches()
        return { "main", "develop" }
    end
    """#

value "app" {
    find-file "*.exe" in="." require-sibling="{stem}_Data"
}
value "greeting" "Hello {env.USERNAME}"
value "computed" (lua)"1 + 2"

data "common" {
    UseGpu "true"
    MediaDir #"..\Videos"#
}

layout {
    dropdown "branch" label="Branch" options=(lua)"branches()" default=(lua)"'main'"
    radio "mode" label="Mode" {
        option "Debug"
        option "Release" value="release"
    }
    dropdown "server" label="Server" option-label="{name} - {Host}" {
        preset "{role.Host}" match="Host"
        option "Local" {
            Host "127.0.0.1"
        }
    }
    dropdown "role" label="Role" {
        option "Player" {
            Host "10.0.0.1"
        }
    }
    group "Options" visible-if="values.mode == 'Debug'" {
        row {
            checkbox "tests" label="Run tests" default=#true {
                on-change {
                    reset "volume" if="not values.tests"
                }
            }
            slider "volume" min=0 max=100 default=50 enabled-if="values.tests" suffix="%"
        }
        text "args" label="Arguments" placeholder="Extra arguments" pattern="^[^;]*$"
        number "jobs" label="Jobs" min=1 max=64 default=8
        file "log" label="Log file" save=#true
        folder "out" label="Output" must-exist=#true required=#true
        checklist "targets" label="Targets" {
            option "win64"
            option "linux"
        }
    }
    tabs {
        tab "More" {
            note "Some text" style="muted"
            separator "Below"
            spacer height=12
        }
    }
    columns {
        column {
            display "summary" label="Summary" text="{branch} in {mode}"
        }
    }
    button "build" label="Build" action="build" primary=#true width="fill"
}

action "build" label="Build" after="stay" lock="{config_dir}/.build.lock" {
    check "git" "diff" "--quiet" message="You have local changes. Commit or stash them first."
    env-from #"C:\VS\vcvarsall.bat"# "x64"
    run "git" "fetch" "--prune" "origin" label="Fetching"
    run "git" "switch" "{branch}"
    run "dotnet" "build" "-c" "{mode}" timeout=600000 expect-exit="0,1" io="pty" fail-if-output="error CS\\d+" {
        env "DOTNET_CLI_TELEMETRY_OPTOUT" "1"
    }
    run "dotnet" "test" if="values.tests" on-error="warn"
    run "game.exe" id="game" command-line="-screen {volume}" wait=#false
    set-process-volume "game" volume="{volume}"
    set "stamp" (lua)"os.time()"
    set-control "args" "x"
    copy "a.txt" "b/" overwrite=#false
    move "b/a.txt" "c.txt"
    delete "c.txt" missing="error"
    mkdir "build/out"
    find-file "*.log" id="found" recursive=#true
    read-file "{found}" id="logtext"
    write-file "config.json" content="{logtext.text}" format="json-pretty" newline="lf"
    http-get "https://example.com/api" id="api" timeout=5000 {
        header "Accept" "application/json"
        extract "session" "data[0].id" required=#true
    }
    ping "{server.Host}" timeout=1000
    open "https://example.com/{session}"
    reveal "{out}"
    message "Done" level="info"
    confirm "Continue?" yes="Go" no="Stop"
    ask "name" "Your name?" default="{env.USERNAME}"
    wait 100
    log "Hello {name}" level="debug"
    lua #"""
        log("from lua")
        """#
    args "game_args" {
        format #"{key}="{value}""#
        replace "\"" "'"
        dedupe "first" ignore-case=#true
        raw "-screen-width 1920" if="values.tests"
        pair "Host" "{server.Host}"
        pairs "role"
        pairs "common" exclude="MediaDir"
    }
    fail "Nope" if="false"
    stop
}
"##;

fn load(src: &str) -> Loaded {
    config::load_str(
        src,
        Path::new("C:/work/build.launcher"),
        &LoadOptions {
            runtime: Version(0, 1, 0),
            os: Os::Windows,
        },
    )
}

fn render(l: &Loaded) -> String {
    l.diags
        .iter()
        .map(|d| d.render("build.launcher"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn a_file_using_every_feature_loads_cleanly() {
    let l = load(FULL);
    assert!(l.diags.is_empty(), "{}", render(&l));
    let c = &l.config;
    assert_eq!(c.header.os, Some(Os::Windows));
    assert_eq!(c.header.requires, Some(Version(0, 1, 0)));
    assert_eq!(c.window.label_width, 150.0);
    assert_eq!(c.window.theme, Theme::Dark);
    assert_eq!(c.scripts.len(), 1);
    assert!(c.scripts[0].code.contains("function branches()"));
    assert_eq!(c.values.len(), 3);
    assert!(matches!(c.values[0].source, ValueSource::Find(_)));
    assert!(matches!(c.values[2].source, ValueSource::Val(Val::Lua(_))));
    assert_eq!(c.data[0].bag[1], ("MediaDir".into(), r"..\Videos".into()));
    assert_eq!(c.controls.len(), 13);
    let branch = c.control("branch").unwrap();
    match &branch.kind {
        ControlKind::Choice { options_lua, .. } => {
            assert_eq!(options_lua.as_ref().unwrap().code, "branches()")
        }
        _ => panic!("branch is a choice"),
    }
    assert!(matches!(branch.default, Some(DefaultVal::Val(Val::Lua(_)))));
    let mode = c.control("mode").unwrap();
    match &mode.kind {
        ControlKind::Choice { options, .. } => {
            assert_eq!(options[1].name, "Release");
            assert_eq!(options[1].value, "release");
        }
        _ => panic!("mode is a choice"),
    }
    let tests = c.control("tests").unwrap();
    assert_eq!(tests.default, Some(DefaultVal::Bool(true)));
    assert_eq!(tests.on_change.len(), 1);
    assert!(tests.on_change[0].if_.is_some());
    let build = c.action("build").unwrap();
    assert_eq!(build.steps.len(), 30);
    match &build.steps[4].kind {
        StepKind::Run(r) => {
            assert_eq!(r.args.len(), 3);
            assert_eq!(r.timeout_ms, Some(600_000));
            assert_eq!(r.expect_exit, ExpectExit::Codes(vec![0, 1]));
            assert_eq!(r.io, Io::Pty);
            assert_eq!(r.env.len(), 1);
        }
        other => panic!("step 5 is a run, not {}", other.name()),
    }
    assert!(
        matches!(&build.steps[6].kind, StepKind::Run(r) if !r.wait && r.command_line.is_some())
    );
    assert_eq!(build.steps[5].on_error, OnError::Warn);
    match &build
        .steps
        .iter()
        .find(|s| s.kind.name() == "args")
        .unwrap()
        .kind
    {
        StepKind::Args(a) => {
            assert_eq!(a.dedupe, Dedupe::First);
            assert_eq!(a.parts.len(), 4);
            assert_eq!(a.replace, vec![("\"".to_string(), "'".to_string())]);
        }
        _ => unreachable!(),
    }
}

#[test]
fn lua_values_know_where_they_are() {
    let l = load(FULL);
    let v = &l.config.values[2];
    let ValueSource::Val(Val::Lua(src)) = &v.source else {
        panic!("a lua value")
    };
    assert_eq!(src.code, "1 + 2");
    assert_eq!(src.repr, "\"1 + 2\"");
    assert_eq!(
        &FULL[src.span.offset..src.span.offset + src.span.len],
        "\"1 + 2\""
    );
    let line = FULL[..src.span.offset].matches('\n').count() + 1;
    assert_eq!(src.line, line);
}

fn codes(src: &str) -> Vec<&'static str> {
    load(src).diags.iter().map(|d| d.code).collect()
}

#[test]
fn problems_are_reported_with_codes() {
    assert_eq!(codes("layout { dropdwn \"x\" }"), ["unknown-kind"]);
    assert_eq!(codes("layout { option \"x\" }"), ["misplaced"]);
    assert_eq!(
        codes("layout { checkbox \"x\" lable=\"X\" }"),
        ["unknown-property"]
    );
    assert_eq!(codes("layout { checkbox }"), ["missing-argument"]);
    assert_eq!(codes("layout { checkbox \"a\" \"b\" }"), ["extra-argument"]);
    assert_eq!(
        codes("layout { checkbox \"a\" persist=\"true\" }"),
        ["wrong-type"]
    );
    assert_eq!(
        codes("layout { checkbox \"a\" label=(lua)\"x\" }"),
        ["lua-not-allowed"]
    );
    assert_eq!(
        codes("layout { checkbox \"a\" tooltip=\"{nope\" }"),
        ["bad-template"]
    );
    assert_eq!(
        codes("layout { checkbox \"a\"\ncheckbox \"a\" }"),
        ["duplicate-id"]
    );
    assert_eq!(
        codes("layout { button \"b\" action=\"go\" }\naction \"og\" { stop }"),
        ["unknown-action"]
    );
    assert_eq!(
        codes("action \"a\" { reset \"nothing\" }"),
        ["unknown-control"]
    );
    assert_eq!(codes("action \"a\" { log \"{whom}\" }"), ["unknown-name"]);
    assert_eq!(
        codes("action \"a\" { log \"%PATH%\" }"),
        ["percent-variable"]
    );
    // Also os-only: a file without os= may run on macOS, where the step doesn't exist.
    assert_eq!(
        codes("action \"a\" { set-process-volume \"g\" volume=\"1\" }"),
        ["unknown-run", "os-only"]
    );
    assert_eq!(
        codes("layout { checkbox \"a\" { on-change { wait 10 } } }"),
        ["not-instant"]
    );
    assert_eq!(codes("window\nlauncher"), ["header-first"]);
    assert_eq!(codes("layout {}\nlayout {}"), ["duplicate-node"]);
    assert!(codes("layout { checkbox \"a\" label=\"x\" {} }").is_empty());
    assert_eq!(
        codes("action \"a\" { copy \".\\rigid.json\" \"x\" }"),
        ["escaped-backslash"]
    );
    assert_eq!(codes("value \"v\""), ["missing-argument"]);
    assert_eq!(codes("layout { slider \"s\" min=5 max=1 }"), ["bad-range"]);
}

#[test]
fn syntax_errors_come_with_hints() {
    let l = load("layout {\n    checkbox \"a\" default=true\n}\n");
    assert_eq!(l.diags[0].code, "syntax");
    assert_eq!(l.diags[0].line, 2);
    assert!(
        l.diags[0].help.as_deref().unwrap().contains("#true"),
        "{}",
        render(&l)
    );
    let l = load("action \"a\" {\n    run \"C:\\Windows\\x.exe\"\n}\n");
    assert_eq!(l.diags[0].code, "syntax");
    assert!(
        l.diags[0].help.as_deref().unwrap().contains("raw string"),
        "{}",
        render(&l)
    );
}

#[test]
fn the_header_gates_newer_files_and_other_oses() {
    let l = load("launcher requires=\"9.0\"\nlayout { a-node-from-the-future }\n");
    assert_eq!(l.diags.len(), 1, "{}", render(&l));
    assert_eq!(l.diags[0].code, "requires-newer");
    assert!(l.gate.is_some());
    let l = load("launcher os=\"macos\"\n");
    assert_eq!(l.diags[0].code, "wrong-os");
    let l = config::load_str(
        "launcher os=\"macos\"\n",
        Path::new("x.launcher"),
        &LoadOptions {
            runtime: Version(1, 0, 0),
            os: Os::Macos,
        },
    );
    assert!(l.diags.is_empty());
}

#[test]
fn os_only_things_are_flagged() {
    // A file for both OSes using a Windows-only step and property: warnings.
    let src = "action \"a\" {\n  run \"g\" id=\"g\" command-line=\"x\"\n  set-process-volume \"g\" volume=\"5\"\n}";
    assert_eq!(codes(src), ["os-only", "os-only"]);
    // A macOS file using them: errors.
    let l = config::load_str(
        &format!("launcher os=\"macos\"\n{src}"),
        Path::new("x.launcher"),
        &LoadOptions {
            runtime: Version(1, 0, 0),
            os: Os::Macos,
        },
    );
    assert_eq!(l.errors(), 2, "{}", render(&l));
}
