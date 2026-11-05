# Launcher - Work Plan

**Status, 5 November 2026:** the features below haven't started, and they are Asset Check's next step
(step 3). The steps before it are done: `asset-rules` `v0.1.0`, and both checkers, `fire-cli` and
`3d-review-cli`, write real results files to build against. The features are generic; Asset Check is
their first user.

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

**Release order:** the add-on now finds only `3d-review-cli`, which first ships in the 3D Review release
after 0.8.0. Release Launcher after that 3D Review release; otherwise 3D Review 0.8.0 users get "3D
Review isn't installed… or it is too old".

## Queue

Line numbers are approximate; they were checked against `main` at 3d9b8d8.

1. **A results table control.**
   - **Keep the control generic: it never reads Asset Check's format.** Its rows come from a computed
     list, as a `choice`'s do (`options=(lua)"…"`): a list of tables with any fields, read as
     `{control.field}`.
     - The tool's own Lua reads the files and reshapes them. `read_file` and `json.decode` are allowed
       while a tool loads (`docs/lua-api.md`), within the 5 s limit.
     - AC14 writes the fields as `{row.field}`; the existing `{control.field}` is the same thing and
       needs no engine change.
   - **What Asset Check's tool will feed it** (the `.launcher` file itself is step 4, in the rules repo):
     - **Two files, shown as one table,** one per checker: `.git/asset-check/results-fire-cli.json` and
       `results-3d-review-cli.json`. Each lists only its own tool's files.
     - **One row per finding:** each file's `findings[]` with that file's `path`, `profile_label` and
       `stage` copied onto each row.
     - **Run-level findings:** the top-level `findings` list, for problems with the whole check (today
       `tool.too_old`). These rows get a group of their own, shown first.
     - **The format:** `asset-check.results` v1, sketched in `asset-check-plan.md` §3 and defined by
       asset-rules' `results` module.
   - **Rows are grouped by a field** (`group-by`), in the order the groups first appear. For Asset
     Check: one group per file, with the run-level group first because the tool lists it first.
   - **Each row** has a severity icon (from a field, e.g. `severity` = error / warning / info), a title
     line, and expandable detail text (what / why / how / note).
   - **Buttons on rows and on group headers,** bound to steps.
     - **A row button** sets the control's value to that row, then presses its button, so the step sees
       the clicked row's fields with no engine change.
     - **A group button** does the same with the group's first row. **Show in Explorer** / **Open** are
       per file, so they go on the group header, not on every finding.
     - **A button can be hidden per row or group** by a condition on its fields (e.g. no **Open** on the
       run-level group, which has no path).
   - **The control scrolls**, and **refreshes after a run finishes.**
   - **Where it goes:**
     - a new choice style (e.g. `show-as="table"`) or a new `ControlKind` in
       `crates/launcher-core/src/config/model.rs` (`ChoiceStyle` around line 342, `ControlKind` around
       365);
     - its entry in `config/catalog.rs` (the dropdown is around line 560), which regenerates
       `docs/config-reference.md`;
     - format-1 parsing in `config/load.rs` (options around line 1297);
     - format-2's `show-as` mapping and `ITEMS` table in `config/format2.rs` (`ITEMS` around line 67,
       `show-as` around 791);
     - drawing in `crates/launcher/src/ui/form.rs` `widget()` (around line 856; the existing choice
       styles around 903-951), with an `egui::ScrollArea` and `egui::Grid`.
   - **Row buttons** emit `FormEvent::Set(id, row)` then `FormEvent::Press(button)`. `document.rs`
     handles them in order (`Set` around line 1000, `Press` around 1017), and `start_run` (around line
     540) snapshots the form.
   - **Refresh after a run:** send `Request::Refresh` when a run ends (`document.rs` already sends one
     around line 1010, after a `Set`).
   - **Optional:** let a step fill rows with full data. Today `set_choices` carries plain strings
     (`launcher-core/src/engine/runner.rs` around line 125, core `form.rs` around 446, `headless.rs`
     around 252). Also optional: visual-editor support.

2. **Pre-filling the window from the command line:** `launcher FILE --set ID=VALUE`, repeatable.
   - Today the window mode accepts exactly one file and rejects anything after it (`cli.rs`, around line
     87). Only headless `launcher run` takes `--set` (`headless.rs`, around line 55).
   - Apply the values as starting values in the window. Values set this way win over remembered ones
     (`remember`) for that launch. Document the flag in `--help` and the README.
   - The hook uses it to pass both results files' paths and the launch context (`hook` or `manual`).

3. **Ending the window with a chosen exit code.**
   - **A step that closes the window with a code,** e.g. `core.close code=1`, or a code on
     `when-done=close`.
   - **A code for closing the window without choosing** (the title bar's close button), set by the
     tool, e.g. `window close-code=1`. Asset Check's tool will set 1, so closing the window counts as
     **Don't commit**, the default button (AC4).
   - **Today the window always exits 0.** `main.rs` returns normally after `event_loop.run_app`
     (around line 73). Keep the chosen code in the app state and `std::process::exit` with it after
     the loop ends.
   - **The hook waits for the window and reads the code:** **Don't commit** → 1, **Commit anyway** → 0.

4. **Tests:**
   - unit tests for the control's model and parsing;
   - an e2e test (`cargo xtask e2e`) that opens two sample results files, presses a group button and a
     row button, and checks the right fields reached each step;
   - one that ends the window with a code, by a step and by closing it.

*Done when* Asset Check step 3's gate passes:
- a pair of real results files (one from each checker, one with a top-level finding) renders correctly
  as one table;
- row and group buttons see their row's fields;
- a `pre-commit` hook that opens the window blocks the commit on **Don't commit** or on closing it,
  and lets it through on **Commit anyway**.

## Test data

Use real results files, made with the two checkers. On the Windows machine both are built:
`D:\Dev\fire\target\release\fire-cli.exe` and `D:\Dev\3d-review-rs\target\release\3d-review-cli.exe`.
Rebuild them with `cargo build --release -p fire-cli` / `-p review-cli` if they're missing.

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

What you get:
- **A `file.no_profile` finding** for the unmatched name.
- **A run-level `tool.too_old` finding** from `3d-review-cli`, as long as it still reports 0.8.0
  (the fixture requires 0.9.0).
- **A colour-profile warning** if the PNG has an embedded non-sRGB profile. Launcher's own
  `assets/application-logo.png` is Display P3.

Copy the resulting files into the repository's test data. They're small JSON files.

## Platform

- **This work is on Windows** (Fork is installed on the development machine). All of it is
  cross-platform, so it can also be built and tested on the Mac.
- **The hook waits for the window and gets its exit code.** Checked on Windows on 5 November 2026: a
  GUI-subsystem program started from a `pre-commit` hook through Git's `sh` was waited for. Its exit
  code reached the hook; 1 stopped the commit and 0 let it through.
  - `docs/LESSONS.md` L2.4 ("shells do not wait for a GUI-subsystem exe") holds for PowerShell and
    `cmd`, not for git's `sh`.
- **Still open, Asset Check's spike S1, with Fork itself:**
  - whether the window comes to the front over Fork. Windows may refuse a process started in the
    background; if it does, flash the taskbar button, which Launcher already does for finished runs;
  - what Fork shows while the hook waits;
  - whether Fork gives up on a hook that waits for minutes.

  Try it once item 3 works, with a real `pre-commit` hook in a scratch repository committed from
  Fork.

## References

- `asset-check-plan.md` in psmyles/fire-decode: AC4 (the hook flow) and AC14 (these features). On the
  Windows development machine it's at `D:\Dev\fire-decode\asset-check-plan.md`; the results format is in
  §3 and the window's layout in §1.
- [`docs/LESSONS.md`](LESSONS.md): platform pitfalls.
- `docs/PLAN.md` in psmyles/asset-rules, psmyles/fire, psmyles/3d-review and psmyles/fire-decode.
