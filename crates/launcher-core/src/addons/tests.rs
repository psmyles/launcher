use super::*;

fn parse(src: &str, origin: Origin) -> (Option<AddOn>, Vec<Diagnostic>) {
    parse_addon(src, Path::new("x.kdl"), origin, &LoadOptions::default())
}

fn messages(d: &[Diagnostic]) -> Vec<String> {
    d.iter()
        .map(|d| match &d.help {
            Some(h) => format!("{}:{} {} ({h})", d.line, d.col, d.message),
            None => format!("{}:{} {}", d.line, d.col, d.message),
        })
        .collect()
}

#[test]
fn the_built_in_add_ons_read_cleanly() {
    let lib = Library::builtin();
    let problems: Vec<String> = lib
        .problems
        .iter()
        .map(|p| format!("{}: {:?}", p.file, messages(&p.diags)))
        .collect();
    assert!(problems.is_empty(), "{problems:#?}");
    for id in ["core", "files", "ask-tell", "internet", "git", "dotnet"] {
        assert!(lib.addon(id).is_some(), "{id} is missing");
    }
    let pull = lib.step("git.pull").unwrap();
    assert_eq!(pull.name, "Pull");
    assert_eq!(pull.app, "Git");
    assert!(matches!(pull.body, Body::Does(ref s) if s.len() == 4));
    assert!(lib.list("git.branches").is_some());
}

#[test]
fn built_in_steps_take_their_settings_from_the_catalog() {
    let lib = Library::builtin();
    let copy = lib.step("files.copy").unwrap();
    let ids: Vec<&str> = copy.settings.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, ["from", "to", "overwrite", "ask-first"]);
    assert!(copy.settings[3].starts_overridden);
    assert_eq!(copy.settings[0].label, "Copy what");
    assert_eq!(copy.settings[0].position, Some(0));
    assert!(copy.settings[0].required);
    assert_eq!(copy.settings[2].kind, SettingKind::TickBox);
    assert_eq!(copy.settings[2].starts, Some(Dyn::Bool(true)));
    let run = lib.step("core.run").unwrap();
    let args = run.setting("arguments").unwrap();
    assert!(args.rest);
    assert_eq!(args.kind, SettingKind::List);
}

#[test]
fn problems_are_plain_and_point_at_the_line() {
    let (_, d) = parse(
        r#"add-on "demo" version="1.0" {
    step "hello" nmae="Hello" {
        setting "who" type="txt"
        doez { }
    }
}"#,
        Origin::Folder("x.kdl".into()),
    );
    assert_eq!(
        messages(&d),
        [
            "2:5 The step \"demo.hello\" needs `does { }` or `script`",
            "2:18 `step` doesn't have a `nmae` (did you mean `name`?)",
            "3:23 \"txt\" isn't a kind of setting (did you mean \"text\"?)",
            "4:9 `doez` doesn't belong in `step` (did you mean `does`?)",
        ]
    );
}

#[test]
fn only_built_in_add_ons_use_built_in_steps() {
    let (_, d) = parse(
        r#"add-on "sneaky" version="1.0" {
    step "copy" built-in="copy"
}"#,
        Origin::Folder("x.kdl".into()),
    );
    assert!(
        d[0].message.contains("Only add-ons built into Launcher"),
        "{d:?}"
    );
}

#[test]
fn the_add_ons_folder_adds_steps_and_reports_clashes() {
    let dir = std::env::temp_dir().join(format!("launcher-addons-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let addon = |id: &str, v: &str| {
        format!(
            "add-on \"{id}\" version=\"{v}\" {{\n    step \"hi\" {{\n        does {{\n            log \"hi\"\n        }}\n    }}\n}}\n"
        )
    };
    std::fs::write(dir.join("a.kdl"), addon("acme", "1.0")).unwrap();
    std::fs::write(dir.join("b.kdl"), addon("acme", "1.2")).unwrap();
    std::fs::write(dir.join("c.kdl"), addon("git", "9.0")).unwrap();
    std::fs::write(dir.join("d.kdl"), addon("twin", "1.0")).unwrap();
    std::fs::write(dir.join("e.kdl"), addon("twin", "1.0")).unwrap();
    std::fs::write(dir.join("f.kdl"), "add-on \"broken\" {").unwrap();
    let lib = Library::with_folder(Some(&dir));
    assert_eq!(lib.addon("acme").unwrap().version, Version(1, 2, 0));
    assert_eq!(lib.addon("git").unwrap().origin, Origin::BuiltIn);
    assert!(lib.addon("twin").is_none());
    assert!(lib.step("acme.hi").is_some());
    let codes: Vec<&str> = lib
        .problems
        .iter()
        .flat_map(|p| p.diags.iter().map(|d| d.code))
        .collect();
    assert!(codes.contains(&"name-taken"), "{codes:?}");
    assert!(codes.contains(&"same-add-on-twice"), "{codes:?}");
    assert!(codes.contains(&"syntax"), "{codes:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_tools_copy_wins_over_an_installed_add_on() {
    let installed = parse(
        r#"add-on "acme" version="2.0" {
    step "hi" name="Installed" { does { log "a"; }; }
}"#,
        Origin::Folder("a.kdl".into()),
    )
    .0
    .unwrap();
    let carried = parse(
        r#"add-on "acme" version="1.0" {
    step "hi" name="Carried" { does { log "b"; }; }
}"#,
        Origin::Carried,
    )
    .0
    .unwrap();
    let mut lib = (*Library::builtin()).clone();
    lib.addons.push(Arc::new(installed));
    let lib = lib.with_carried(vec![carried]);
    assert_eq!(lib.step("acme.hi").unwrap().name, "Carried");
    // Built-in add-ons can't be replaced by a copy.
    let fake_git = parse(
        r#"add-on "git" version="9.0" {
    step "pull" { does { log "x"; }; }
}"#,
        Origin::Carried,
    )
    .0
    .unwrap();
    let lib = lib.with_carried(vec![fake_git]);
    assert_eq!(lib.step("git.pull").unwrap().name, "Pull");
}

#[test]
fn an_add_on_keeps_its_own_text_for_carrying() {
    let src =
        "// note\nadd-on \"acme\" version=\"1.0\" {\n    step \"hi\" { does { log \"a\"; }; }\n}\n";
    let a = parse(src, Origin::Folder("a.kdl".into())).0.unwrap();
    assert!(a.text.starts_with("add-on \"acme\""), "{}", a.text);
    assert!(a.text.ends_with('}'));
}

#[test]
fn add_ons_speak_plainly() {
    let lib = Library::builtin();
    let mut found = Vec::new();
    let mut check = |where_: String, text: &str| {
        let j = jargon(text);
        if !j.is_empty() {
            found.push(format!("{where_}: {j:?} in \"{text}\""));
        }
    };
    for a in &lib.addons {
        check(format!("{} name", a.id), &a.name);
        if let Some(t) = &a.about {
            check(format!("{} about", a.id), t);
        }
        for s in &a.steps {
            check(format!("{} name", s.id), &s.name);
            for t in s.says.iter().chain(&s.about) {
                check(format!("{} says/about", s.id), t);
            }
            for w in &s.search_words {
                let _ = w; // search words may hold the jargon people type
            }
            for st in &s.settings {
                check(format!("{} setting {} label", s.id, st.id), &st.label);
                if let Some(h) = &st.help {
                    check(format!("{} setting {} help", s.id, st.id), h);
                }
            }
            for r in &s.results {
                check(format!("{} result {}", s.id, r.id), &r.label);
            }
            for (_, advice) in &s.explain {
                check(format!("{} explain", s.id), advice);
            }
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}

#[test]
fn a_file_setting_names_the_extensions_it_takes() {
    let (a, d) = parse(
        r#"add-on "demo" version="1.0" {
    step "load" name="Load" {
        setting "preset" type="file" extensions=".JSON, yaml"
        setting "name" type="text" extensions="json"
        setting "other" type="file" extensions=" , "
        script "return {}"
    }
}"#,
        Origin::Folder("x.kdl".into()),
    );
    assert_eq!(
        messages(&d),
        [
            "4:36 Only a file setting takes `extensions` (take it out, or make the setting type=\"file\")",
            "5:37 `extensions` names no extension (write them like extensions=\"json, fbx\")",
        ]
    );
    let step = &a.unwrap().steps[0];
    let preset = step.setting("preset").unwrap();
    assert_eq!(preset.extensions, ["json", "yaml"]);
    assert_eq!(preset.needs(), "a .json or .yaml file");
    assert!(preset.takes("file", &["json".into()], &[]));
    assert!(!preset.takes("file", &["fbx".into()], &[]));
    assert!(!preset.takes("file", &[], &[]));
    assert!(!preset.takes("folder", &[], &[]));
}

#[test]
fn results_say_what_kind_of_value_they_are() {
    let (a, d) = parse(
        r#"add-on "demo" version="1.0" {
    step "scan" name="Scan" {
        setting "models" type="folder" several=#true
        setting "name" type="text"
        setting "notes" type="long-text" several=#true
        setting "on" type="tick-box" several=#true
        result "found" type="files"
        result "count" type="number"
        result "note"
        result "bad" type="nubmer"
        script "return {}"
    }
}"#,
        Origin::Folder("x.kdl".into()),
    );
    assert_eq!(
        messages(&d),
        [
            "6:38 Only a file, folder or text setting takes `several` (take it out, or make the setting type=\"folder\", \"file\" or \"long-text\")",
            "10:22 \"nubmer\" isn't a kind of result (did you mean \"number\"?)",
        ]
    );
    let step = &a.unwrap().steps[0];
    let kinds: Vec<ResultKind> = step.results.iter().map(|r| r.kind).collect();
    assert_eq!(
        kinds,
        [
            ResultKind::Files,
            ResultKind::Number,
            ResultKind::Text,
            ResultKind::Text
        ]
    );
    // A folder that takes several takes a list of files, never a count.
    let models = step.setting("models").unwrap();
    assert_eq!(models.needs(), "a folder or a list of files");
    assert!(models.takes_result(ResultKind::Folder));
    assert!(models.takes_result(ResultKind::Files));
    assert!(!models.takes_result(ResultKind::Number));
    assert!(!models.takes_result(ResultKind::Text));
    // Text takes any one value, but not a list.
    let name = step.setting("name").unwrap();
    assert!(name.takes_result(ResultKind::Number));
    assert!(!name.takes_result(ResultKind::Files));
    assert!(!name.takes("choices", &[], &[]));
    // Text that takes several takes a list too, one item per line.
    let notes = step.setting("notes").unwrap();
    assert_eq!(notes.needs(), "text or a list");
    assert!(notes.takes_result(ResultKind::Files));
    assert!(notes.takes_result(ResultKind::List));
    assert!(notes.takes_result(ResultKind::Number));
    assert!(notes.takes("choices", &[], &[]));
}

#[test]
fn a_built_in_step_can_take_a_list_in_its_text() {
    let lib = Library::builtin();
    let write = lib.step("files.write").unwrap();
    assert!(!write.advanced);
    let content = write.setting("content").unwrap();
    assert_eq!(content.kind, SettingKind::LongText);
    assert!(content.takes_result(ResultKind::Files));
    // Not the file's name, though.
    assert!(!write
        .setting("path")
        .unwrap()
        .takes_result(ResultKind::Files));
    // Its settings read top to bottom: where, a heading, what, then whether it adds.
    let shown: Vec<&str> = write
        .settings
        .iter()
        .filter(|s| !s.advanced)
        .map(|s| s.id.as_str())
        .collect();
    assert_eq!(shown, ["path", "heading", "content", "append"]);

    // Only text becomes long text, and only what can take several does.
    let (_, d) = parse(
        r#"add-on "files" version="1.0" {
    step "write" name="Write" built-in="write-file" {
        setting "create-dirs" type="long-text" several=#true
    }
}"#,
        Origin::BuiltIn,
    );
    assert_eq!(
        messages(&d),
        [
            "3:31 The built-in step's \"create-dirs\" is tick-box (only text can become long-text)",
            "3:48 The built-in step's \"create-dirs\" can't take several",
        ]
    );
}

#[test]
fn settings_take_only_values_they_can_hold() {
    let lib = Library::builtin();
    let audit = lib.step("review3d.audit").unwrap();
    let preset = lib
        .step("review3d.run-preset")
        .unwrap()
        .setting("preset")
        .unwrap()
        .clone();
    // Typed values.
    assert_eq!(preset.check_value("a.json"), None);
    assert_eq!(
        preset.check_value("a.fbx").as_deref(),
        Some("should be a .json file")
    );
    let when = audit.setting("needs-fixing").unwrap();
    assert_eq!(when.check_value("Errors"), None);
    assert!(when
        .check_value("Sometimes")
        .unwrap()
        .starts_with("should be one of: Errors"));
    // A drop-down with fixed options takes a drop-down whose options are among them, or one a
    // step fills; never what a step found.
    let some: Vec<String> = vec!["Errors".into()];
    let other: Vec<String> = vec!["Errors".into(), "Always".into()];
    assert!(when.takes("choice", &[], &some));
    assert!(when.takes("choice", &[], &[]));
    assert!(!when.takes("choice", &[], &other));
    assert!(!when.takes_result(ResultKind::Text));
    // Put it into: a list fills a drop-down's choices; a count goes into a number or text.
    assert!(ResultKind::Files.fills("choice"));
    assert!(!ResultKind::Files.fills("number"));
    assert!(ResultKind::Number.fills("number"));
    assert!(!ResultKind::Number.fills("folder"));
}
