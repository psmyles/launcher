# Launcher - Work Plan

**Status, 5 November 2026:** Asset Check's step 3, Launcher's part, is built: a results table, window
`--set`, and the window's exit code. All of it is checked on Windows, the hook flow included (see
*The gate*). Next is Asset Check's step 4, `asset-check.launcher` in the rules repo, which uses the
syntax below. The features are generic; Asset Check is their first user.

The 3D Review add-on now runs `3d-review-cli`, 3D Review's console tool, which was renamed from
`review-cli`.

## Context

**Asset Check** checks art assets committed to a Unity project against a technical artist's rules. When
something is wrong, the artist sees it in a **Launcher window**:
- one group per file, one row per problem, with what / why / how;
- first, a group for problems that concern the whole check rather than one file, such as "Update
  3D Review to version 0.9.0 or later";
- **Show in Explorer** / **Open** buttons on each file;
- at the bottom, **Don't commit** and **Commit anyway**, whose choice goes back to the git hook that
  opened the window.

The design is `asset-check-plan.md` in [psmyles/fire-decode](https://github.com/psmyles/fire-decode)
(private; it moves to the rules repo later). Launcher's part is decision **AC14**, plus AC4 for the exit
code.

**Release order:**
- The add-on now finds only `3d-review-cli`, which first ships in the 3D Review release after 0.8.0.
  Release Launcher after that 3D Review release; otherwise 3D Review 0.8.0 users get "3D Review isn't
  installed… or it is too old".
- `product.json` still says 0.2.1. Bump it for the release, and have `asset-check.launcher` require
  that version: an older Launcher doesn't know `table`.

## What step 4 writes

`tests/fixtures/asset-check/asset-check.launcher` is a working stand-in, with the two real results
files beside it:

```kdl
window close-code=1 width=720 {
    text "fire" label="fire-cli results" show-if="false"
    text "review" label="3d-review-cli results" show-if="false"
    table "findings" rows=(lua)"findings()" row-label="{title}" group-by="path" group-label="{heading}" icon-from="severity" details="{details}" open=#true height=360 {
        group-button "reveal" label="Show in Explorer" show-if="values.path ~= ''"
        group-button "open" label="Open" show-if="values.path ~= ''"
        row-button "explain" label="Explain"
    }
    row {
        button "dont" label="Don't commit - I'll fix these"
        button "commit" label="Commit anyway"
    }
}
steps "dont" when-done="close" exit-code=1 { … }
steps "commit" when-done="close" exit-code=0 { … }
```

- **`table`** (a `choice` underneath):
  - **`rows=(lua)`**: a list of tables with any fields. The tool's Lua (`findings()` in the stand-in)
    reads both results files, puts the top-level `findings` first with `path = ""`, and copies each
    file's `path`, `profile_label` and `stage` onto its findings. A row's value is its number from 1,
    unless it has a `value` field.
  - **The rest is presentation, all from a row's fields:**
    - `group-by`: groups, in the order they first appear;
    - `group-label`: the group's header, from its first row;
    - `row-label`: a row's title;
    - `icon-from`: `error`, `warning` or `info`;
    - `details`: the text that opens under a row; `open=#true` starts rows opened;
    - `height`: past this, the rows scroll.

    Each takes a template or `(lua)`.
  - **Buttons:** `group-button` and `row-button` run the `steps` of the same name, like `button`.
    - Pressing one makes its row (a group's first row) the table's value, so its steps read
      `{findings.path}`.
    - `show-if` reads the row's fields as `values.field`.
  - **The rows are worked out again after every run**, so **Check again** shows new results.
  - **The table's value is never remembered.**
- **`launcher FILE --set ID=VALUE`**, repeatable:
  - the values are set before the file's Lua runs, so `rows=(lua)` sees them;
  - they win over remembered values and aren't remembered themselves;
  - `launcher run` now does the same (it used to work out computed lists before applying `--set`);
  - Lua's `exists` and `read_file` resolve a relative path against the `.launcher` file's folder,
    not the hook's working folder, so the hook passes the results files as absolute paths.
- **Exit codes:**
  - `steps … when-done="close" exit-code=N`: a successful run closes the window and Launcher exits N;
  - `window close-code=N`: closing the window any other way (title bar, Alt+F4) exits N;
  - `launcher run --action` exits with the action's `exit-code` too;
  - a window that can't open at all exits 1;
  - codes are 0 to 255.

## Done

1. **Results table:**
   - `ChoiceStyle::Table` with a `TableSpec`: `config/model.rs`, `catalog.rs`, `load.rs`, `format2.rs`
     (word `table`);
   - rows, groups and marks: `launcher-core/src/table.rs`;
   - per-row `(lua)` texts and button conditions (`Which::Row`): `eval.rs`;
   - drawing: `ui/form.rs` `table()`;
   - refresh after a run, and `has_button`: `ui/document.rs`;
   - the info mark's colour: theme extra `info`.
2. **Window `--set`:** `cli.rs`, `Start::Document`, `Form::set_for_launch`, `Document::evaluate`.
3. **Exit code:** `Action::exit_code`, `WindowSpec::close_code`, `after_run` and `exit_code` in
   `ui/document.rs`, then `App` and `Shell` to `main`.
4. **Tests:**
   - `launcher-core/tests/table.rs`: the model, the two real files as one table, per-row Lua,
     button runs seeing their row's fields, `--set` not remembered, exit-code checks;
   - `ui/form.rs`: clicks on a group button and a row button in a drawn table;
   - `ui/document.rs`: `after_run`;
   - `cli.rs`: parsing;
   - `cargo xtask e2e`, three new cases:
     - rows from the real files reach the steps;
     - `dont` / `commit` exit 1 / 0;
     - the real window, closed, exits with its close-code (Windows only).

### The gate

- **Done:** a pair of real results files renders as one table, with the run-level group first and no
  file buttons on it. Checked in the window, from a screenshot.
- **Done:** row and group buttons see their row's fields. Tested in core, in the UI and in e2e.
- **Done:** a `pre-commit` hook waits for the window and exits with its code. Checked on Windows on
  5 November 2026 with the release build, Git's `sh` and the stand-in tool, in a scratch repository.
  The hook ran `launcher asset-check.launcher --set fire=… --set review=…`, with paths holding spaces:
  - **Commit anyway** (a real mouse click): exit 0, and the commit landed;
  - **Don't commit** (a real mouse click): exit 1, and the commit was stopped with the change still
    staged;
  - closing the window: exit 1, and the commit was stopped.

## Still open

- **Asset Check's spike S1, with Fork itself** (see *Platform*).
- **`crates/launcher/tests/cli.rs` `version_and_help` fails.** `product.json` says 0.2.1 (since
  `efc42e5`, before this work) but the workspace `Cargo.toml` still says 0.2.0, and the test looks
  for Cargo's version in `--version`. Bring the two into line at the release bump.
- **The visual editor shows a table and keeps it on save, but can't edit it.** Its button tabs list
  only the window's buttons, so a table button's steps can't be reached there.
- **`docs/theme-reference.kdl` fails its test on Windows.** egui's
  `ime_composition.legacy_visuals` defaults to `true` on Windows only, and the file was generated
  on macOS. This was true before this work.
- **macOS:** not run there yet.

## Test data

`tests/fixtures/asset-check/results-fire-cli.json` and `results-3d-review-cli.json` were made with the
two checkers on 5 November 2026, as below. To make new ones on the Windows machine, where both
checkers are built (`D:\Dev\fire\target\release\fire-cli.exe` and
`D:\Dev\3d-review-rs\target\release\3d-review-cli.exe`; rebuild them with
`cargo build --release -p fire-cli` / `-p review-cli` if they're missing):

1. Make a scratch git repository. Copy asset-rules' `tests/fixtures/project` into it as `rules/`.
2. Add files named for the fixture's profiles:
   - models from `D:\Dev\3d-review-rs\assets\test_models` (e.g. `meter_cube.fbx`, renamed
     `SM_Crate.fbx`);
   - any PNG as `T_Crate_D.png`;
   - one name matching no profile, e.g. `ComfyUI_00042_.png`.
3. Stage them on a feature branch, then run:

   ```sh
   fire-cli check --staged --rules rules/rules.json --results .git/asset-check/results-fire-cli.json
   3d-review-cli audit --staged --rules rules/rules.json --results .git/asset-check/results-3d-review-cli.json
   ```

What they hold:
- **A `file.no_profile` finding** for the unmatched name. Its `profile_label` and `stage` are `null`.
- **A run-level `tool.too_old` finding** from `3d-review-cli` 0.8.0 (the fixture requires 0.9.0).
- **A colour-profile warning:** Launcher's own `assets/application-logo.png` is Display P3.
- **Two info findings** on the model.

## Platform

- **This work is on Windows** (Fork is installed on the development machine). All of it is
  cross-platform, so it can also be built and tested on the Mac.
- **The hook waits for the window and gets its exit code.** Checked on Windows on 5 November 2026,
  three times:
  - first with a stand-in GUI-subsystem program;
  - then with Launcher's own release build and the stand-in tool: **Don't commit** and closing the
    window stopped the commit, and **Commit anyway** let it through;
  - then committing from Fork: the window opened with its findings, and closing it stopped the
    commit. Fork then shows its "Git Error" dialog with the hook's output (Asset Check's AC4 and S1).

  `docs/LESSONS.md` L2.4 now says that Git's `sh` waits, unlike PowerShell and `cmd`.
- **Still open, Asset Check's spike S1, with Fork:**
  - whether the window comes to the front over Fork by itself. Windows may refuse a process started
    in the background; if it does, flash the taskbar button, which Launcher already does for
    finished runs;
  - what Fork shows while the hook waits;
  - whether Fork gives up on a hook that waits for minutes.

  The scratch repository is `D:\Dev\hook-try\repo`. Its `pre-commit` hook opens the stand-in tool
  (copied to `D:\Dev\hook-try\tool`) with this checkout's release build.

## References

- `asset-check-plan.md` in psmyles/fire-decode: AC4 (the hook flow) and AC14 (these features). On the
  Windows development machine it's at `D:\Dev\fire-decode\asset-check-plan.md`; the results format is in
  §3 and the window's layout in §1.
- [`docs/LESSONS.md`](LESSONS.md): platform pitfalls.
- `docs/PLAN.md` in psmyles/asset-rules, psmyles/fire, psmyles/3d-review and psmyles/fire-decode.
