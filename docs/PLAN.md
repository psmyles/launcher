# Launcher - Work Plan

**Status, November 2026:** the 3D Review add-on now runs `3d-review-cli`, 3D Review's console tool,
which was renamed from `review-cli`. The features below haven't started. They're generic, and Asset
Check needs them.

## Context

**Asset Check** checks art assets committed to a Unity project against a technical artist's rules. When
something is wrong, the artist sees it in a **Launcher window**:
- one group per file, one row per problem, with what / why / how;
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

Line numbers are approximate. They were read from a working tree that had other edits in
`crates/launcher/src/ui/form.rs` and `ui/theme.rs`.

1. **A results table control.**
   - **Rows** come from data known only at run time: a JSON file whose path is a form value. Reuse what
     computed choices already do: rows are option tables with any fields, read as `{control.field}`.
   - **Rows can be grouped by a field.** For Asset Check: one group per file, one row per finding.
   - **Each row** has a severity icon, a title line, and expandable detail text.
   - **Rows can carry buttons** bound to steps. A row button sets the control's value to that row and
     then presses its button, so the step sees the clicked row's fields with no engine change.
   - **The control scrolls**, and **refreshes after a run finishes.**
   - **Where it goes:**
     - a new choice style (e.g. `show-as="table"`) or a new `ControlKind` in
       `crates/launcher-core/src/config/model.rs` (`ControlKind` / `ChoiceStyle`, around lines 341-418);
     - its entry in `config/catalog.rs` (around line 560), which regenerates `docs/config-reference.md`;
     - format-1 parsing in `config/load.rs` (around line 1265);
     - format-2's `show-as` mapping and `ITEMS` table in `config/format2.rs` (around lines 788-813);
     - drawing in `crates/launcher/src/ui/form.rs` `widget()` (around lines 856-1001), with an
       `egui::ScrollArea` and `egui::Grid`.
   - **Row buttons** emit `FormEvent::Set(id, row)` then `FormEvent::Press(button)`. `document.rs`
     applies events in order (around lines 920-922, 1000-1006), and `start_run` snapshots the form
     (around line 553).
   - **Refresh after a run:** send `Request::Refresh` when a run ends (`document.rs` around lines
     949-983; `eval.rs` around lines 739 and 836-839).
   - **Optional:** let a step fill rows with full data. Today `RunUi::set_choices` carries plain strings
     (`runner.rs` around lines 125 and 1266, `ui/run.rs` around 392, core `form.rs` around 446). Also
     optional: visual-editor support.

2. **Pre-filling the window from the command line:** `launcher FILE --set ID=VALUE`.
   - Today the window mode accepts exactly one file and rejects anything after it (`cli.rs`, around lines
     85-90). Only headless `launcher run` takes `--set` (`headless.rs`, around lines 43-86 and 351-376).
   - Apply the values as starting values in the window, and document the flag in `--help` and the
     README.
   - The hook uses it to pass the results file's path and the launch context (`hook` or `manual`).

3. **A step that ends the window with a chosen exit code.**
   - For example `core.close code=1`, or an exit code on `when-done=close`.
   - Today the window always exits 0 (`main.rs`, around lines 72-80).
   - The hook waits for the window and reads the code: **Don't commit** → 1, **Commit anyway** → 0.

4. **Tests:** unit tests for the control's model and parsing; an e2e test (`cargo xtask e2e`) that opens
   a sample results file, presses a row button and checks the row's fields reached the step; and one that
   ends the window with a code.

*Done when* Asset Check step 3's gate passes: a sample results file renders correctly, row buttons see
their row's fields, and the hook reads the chosen exit code.

## Working on the Mac

- All of this is cross-platform and can be built and tested on the Mac.
- On Windows, still to confirm: whether the git hook (run by Fork through Git's `sh`) waits for
  Launcher's GUI-subsystem window, and whether that window can come to the front. That's Asset Check's
  spike S1, which needs Fork on Windows; `docs/LESSONS.md` L2.4 is the known pitfall.

## References

- `asset-check-plan.md` in psmyles/fire-decode: AC4 (the hook flow) and AC14 (these features).
- [`docs/LESSONS.md`](LESSONS.md): platform pitfalls.
- `docs/PLAN.md` in psmyles/asset-rules, psmyles/fire, psmyles/3d-review and psmyles/fire-decode.
