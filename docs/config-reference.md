# `.launcher` file reference

Generated from `crates/launcher-core/src/config/catalog.rs` (`cargo xtask docs`); do not edit by hand.

A `.launcher` file is KDL v2. Values are plain text, templates with `{name}`, `{name.field}` and `{env.NAME}` placeholders, or Luau written `(lua)"..."`. Conditions (`if=`, `enabled-if=`, `visible-if=`) are Luau expressions. Relative paths resolve against the file's folder.

## Top level

### `launcher`

The header, first in the file. The launcher reads it before anything else.

- `requires` (version): The oldest Launcher version this file works with (e.g. "1.2").
- `os` (word: windows | macos): The one OS this file is for. Leave it out for a file that works on both.
- `save-logs` (#true|#false, default `#false`): Save what each run printed to files on this computer ("Log files" opens them). Without it, a run's output only shows in the window.

### `window`

The window.

- `title` (template, default `Launcher`): The window title.
- `width` (number): Fixed width (logical px). Without it the window fits its content.
- `height` (number): Fixed height (logical px). Without it the window fits its content.
- `resizable` (#true|#false, default `#true`): Let the user resize the window.
- `theme` (word: system | dark | light, default `system`): Dark or light colours, or follow the system.
- `label-width` (number, default `140`): Width of the label column (logical px).
- `field-width` (number, default `260`): Width of the field column while the window fits its content (logical px).
- `position` (word: remember | center, default `remember`): Open where it was last closed, or centred.

### `script`

Luau code loaded before anything is evaluated. Several `script` blocks run in order.

- argument `code` (Lua code, required): Luau code: functions and values the rest of the file can use.

### `value`

A value worked out when the file loads: a template, a Lua expression, or a find-file search.

- argument `id` (name, required): The name other parts of the file refer to.
- argument `value` (template): What the value is: a template, or `(lua)"..."`. Leave out to use a find-file block.
- block: `find-file`

### `data`

A named list of key/value pairs (for example, arguments sent on every launch).

- argument `id` (name, required): The name other parts of the file refer to.
- block: `Key "value"` lines

### `layout`

What the window shows, top to bottom.

- block: layout items and controls

### `action`

Steps a button (or `launcher run --action`) carries out, in order.

- argument `id` (name, required): The name other parts of the file refer to.
- `label` (text): The name shown for the action.
- `after` (word: stay | exit | minimize, default `stay`): After a successful run: keep the window, close it, or minimize it.
- `lock` (template): A file path guarding a shared resource: two runs holding the same lock never overlap, even from different files.
- block: steps

## Layout

### `group`

A titled box around related controls.

- argument `title` (template): The group's title.
- `visible-if` (Lua expression): Only show the group when this is true.
- block: layout items and controls

### `tabs`

Tab pages; each `tab` is one page.

- block: `tab`

### `columns`

Side-by-side columns; each `column` is one.

- block: `column`

### `row`

Several controls on one line. The first one's label sits in the label column.

- `visible-if` (Lua expression): Only show the row when this is true.
- block: layout items and controls

### `separator`

A horizontal line, optionally with a title.

- argument `title` (template): Text on the line.

### `spacer`

Empty vertical space.

- `height` (number, default `8`): Height in logical px.

### `note`

A line of text.

- argument `text` (template, required): The text.
- `style` (word: normal | muted | warning | error, default `normal`): How the text looks.
- `visible-if` (Lua expression): Only show the note when this is true.

## Controls

### `dropdown`

Pick one option from a list. Each option can carry key/value data.

- argument `id` (name, required): The name other parts of the file refer to.
- `options` ((lua) expression): `(lua)"..."`: a list worked out when the file loads (strings, or tables with name and data fields), instead of `option` lines. The list has a refresh button.
- `option-label` (template): How each option is shown: {name} and its data fields.
- `label` (text): Text in the label column.
- `tooltip` (template): Shown when hovering the control.
- `default` (value): The value before the user changes it. `(lua)"..."` works it out when the file loads.
- `persist` (#true|#false, default `#true`): Remember the value between launches.
- `visible-if` (Lua expression): Only show the control when this is true.
- `enabled-if` (Lua expression): Grey the control out unless this is true.
- `disabled-tooltip` (template): Shown when hovering the control while it is greyed out.
- `required` (#true|#false, default `#false`): Actions can't run while the control is empty; the Run button says why.
- `width` (width): Field width: "fill" (to the form's right edge), "auto", or a number of logical pixels.
- block: `option`, `preset`, `on-change`, `list-from`

### `radio`

Pick one option with radio buttons.

- argument `id` (name, required): The name other parts of the file refer to.
- `horizontal` (#true|#false, default `#true`): Lay the buttons out on one line.
- `options` ((lua) expression): `(lua)"..."`: a computed list of options instead of `option` lines.
- `option-label` (template): How each option is shown.
- `label` (text): Text in the label column.
- `tooltip` (template): Shown when hovering the control.
- `default` (value): The value before the user changes it. `(lua)"..."` works it out when the file loads.
- `persist` (#true|#false, default `#true`): Remember the value between launches.
- `visible-if` (Lua expression): Only show the control when this is true.
- `enabled-if` (Lua expression): Grey the control out unless this is true.
- `disabled-tooltip` (template): Shown when hovering the control while it is greyed out.
- `required` (#true|#false, default `#false`): Actions can't run while the control is empty; the Run button says why.
- `width` (width): Field width: "fill" (to the form's right edge), "auto", or a number of logical pixels.
- block: `option`, `preset`, `on-change`, `list-from`

### `checklist`

Tick any number of options. The value is the list of ticked option values.

- argument `id` (name, required): The name other parts of the file refer to.
- `options` ((lua) expression): `(lua)"..."`: a computed list of options instead of `option` lines.
- `option-label` (template): How each option is shown.
- `label` (text): Text in the label column.
- `tooltip` (template): Shown when hovering the control.
- `default` (value): The value before the user changes it. `(lua)"..."` works it out when the file loads.
- `persist` (#true|#false, default `#true`): Remember the value between launches.
- `visible-if` (Lua expression): Only show the control when this is true.
- `enabled-if` (Lua expression): Grey the control out unless this is true.
- `disabled-tooltip` (template): Shown when hovering the control while it is greyed out.
- `required` (#true|#false, default `#false`): Actions can't run while the control is empty; the Run button says why.
- `width` (width): Field width: "fill" (to the form's right edge), "auto", or a number of logical pixels.
- block: `option`, `on-change`, `list-from`

### `checkbox`

On or off. The label is shown beside the box.

- argument `id` (name, required): The name other parts of the file refer to.
- `label` (text): Text in the label column.
- `tooltip` (template): Shown when hovering the control.
- `default` (value): The value before the user changes it. `(lua)"..."` works it out when the file loads.
- `persist` (#true|#false, default `#true`): Remember the value between launches.
- `visible-if` (Lua expression): Only show the control when this is true.
- `enabled-if` (Lua expression): Grey the control out unless this is true.
- `disabled-tooltip` (template): Shown when hovering the control while it is greyed out.
- `required` (#true|#false, default `#false`): Actions can't run while the control is empty; the Run button says why.
- `width` (width): Field width: "fill" (to the form's right edge), "auto", or a number of logical pixels.
- block: `on-change`

### `slider`

A number picked by dragging.

- argument `id` (name, required): The name other parts of the file refer to.
- `min` (number, default `0`): Lowest value.
- `max` (number, default `100`): Highest value.
- `step` (number, default `1`): Values snap to multiples of this.
- `suffix` (text): Shown after the number, e.g. "%".
- `label` (text): Text in the label column.
- `tooltip` (template): Shown when hovering the control.
- `default` (value): The value before the user changes it. `(lua)"..."` works it out when the file loads.
- `persist` (#true|#false, default `#true`): Remember the value between launches.
- `visible-if` (Lua expression): Only show the control when this is true.
- `enabled-if` (Lua expression): Grey the control out unless this is true.
- `disabled-tooltip` (template): Shown when hovering the control while it is greyed out.
- `required` (#true|#false, default `#false`): Actions can't run while the control is empty; the Run button says why.
- `width` (width): Field width: "fill" (to the form's right edge), "auto", or a number of logical pixels.
- block: `on-change`

### `number`

A number typed in, with optional limits.

- argument `id` (name, required): The name other parts of the file refer to.
- `min` (number): Lowest value.
- `max` (number): Highest value.
- `step` (number, default `1`): Dragging or the arrow keys change the value by this.
- `decimals` (integer, default `0`): Digits after the decimal point.
- `label` (text): Text in the label column.
- `tooltip` (template): Shown when hovering the control.
- `default` (value): The value before the user changes it. `(lua)"..."` works it out when the file loads.
- `persist` (#true|#false, default `#true`): Remember the value between launches.
- `visible-if` (Lua expression): Only show the control when this is true.
- `enabled-if` (Lua expression): Grey the control out unless this is true.
- `disabled-tooltip` (template): Shown when hovering the control while it is greyed out.
- `required` (#true|#false, default `#false`): Actions can't run while the control is empty; the Run button says why.
- `width` (width): Field width: "fill" (to the form's right edge), "auto", or a number of logical pixels.
- block: `on-change`

### `text`

Free text.

- argument `id` (name, required): The name other parts of the file refer to.
- `multiline` (#true|#false, default `#false`): Several lines.
- `lines` (integer, default `4`): Height of a multiline field, in lines.
- `password` (#true|#false, default `#false`): Hide what is typed.
- `placeholder` (text): Grey hint shown while empty.
- `pattern` (text): A regular expression the whole value must match before actions can run.
- `pattern-message` (text): What to say when the value doesn't match `pattern`.
- `label` (text): Text in the label column.
- `tooltip` (template): Shown when hovering the control.
- `default` (value): The value before the user changes it. `(lua)"..."` works it out when the file loads.
- `persist` (#true|#false, default `#true`): Remember the value between launches.
- `visible-if` (Lua expression): Only show the control when this is true.
- `enabled-if` (Lua expression): Grey the control out unless this is true.
- `disabled-tooltip` (template): Shown when hovering the control while it is greyed out.
- `required` (#true|#false, default `#false`): Actions can't run while the control is empty; the Run button says why.
- `width` (width): Field width: "fill" (to the form's right edge), "auto", or a number of logical pixels.
- block: `on-change`

### `file`

A file path with a Browse button.

- argument `id` (name, required): The name other parts of the file refer to.
- `filter` (text): File types, e.g. "Programs|*.exe;*.bat".
- `save` (#true|#false, default `#false`): Pick a file to create rather than an existing one.
- `must-exist` (#true|#false, default `#false`): Actions can't run until the file exists.
- `relative` (#true|#false, default `#true`): Store paths inside the file's folder as relative paths.
- `placeholder` (text): Grey hint shown while empty.
- `label` (text): Text in the label column.
- `tooltip` (template): Shown when hovering the control.
- `default` (value): The value before the user changes it. `(lua)"..."` works it out when the file loads.
- `persist` (#true|#false, default `#true`): Remember the value between launches.
- `visible-if` (Lua expression): Only show the control when this is true.
- `enabled-if` (Lua expression): Grey the control out unless this is true.
- `disabled-tooltip` (template): Shown when hovering the control while it is greyed out.
- `required` (#true|#false, default `#false`): Actions can't run while the control is empty; the Run button says why.
- `width` (width): Field width: "fill" (to the form's right edge), "auto", or a number of logical pixels.
- block: `on-change`

### `folder`

A folder path with a Browse button.

- argument `id` (name, required): The name other parts of the file refer to.
- `must-exist` (#true|#false, default `#false`): Actions can't run until the folder exists.
- `relative` (#true|#false, default `#true`): Store paths inside the file's folder as relative paths.
- `placeholder` (text): Grey hint shown while empty.
- `label` (text): Text in the label column.
- `tooltip` (template): Shown when hovering the control.
- `default` (value): The value before the user changes it. `(lua)"..."` works it out when the file loads.
- `persist` (#true|#false, default `#true`): Remember the value between launches.
- `visible-if` (Lua expression): Only show the control when this is true.
- `enabled-if` (Lua expression): Grey the control out unless this is true.
- `disabled-tooltip` (template): Shown when hovering the control while it is greyed out.
- `required` (#true|#false, default `#false`): Actions can't run while the control is empty; the Run button says why.
- `width` (width): Field width: "fill" (to the form's right edge), "auto", or a number of logical pixels.
- block: `on-change`

### `display`

Read-only text worked out from other values.

- argument `id` (name, required): The name other parts of the file refer to.
- `text` (template, required): What to show.
- `copyable` (#true|#false, default `#true`): Let the user select and copy the text.
- `label` (text): Text in the label column.
- `tooltip` (template): Shown when hovering the control.
- `default` (value): The value before the user changes it. `(lua)"..."` works it out when the file loads.
- `persist` (#true|#false, default `#true`): Remember the value between launches.
- `visible-if` (Lua expression): Only show the control when this is true.
- `enabled-if` (Lua expression): Grey the control out unless this is true.
- `disabled-tooltip` (template): Shown when hovering the control while it is greyed out.
- `required` (#true|#false, default `#false`): Actions can't run while the control is empty; the Run button says why.
- `width` (width): Field width: "fill" (to the form's right edge), "auto", or a number of logical pixels.

### `button`

Runs an action.

- argument `id` (name, required): The name other parts of the file refer to.
- `action` (action id, required): The action to run.
- `confirm` (template): Ask this question first; No cancels.
- `primary` (#true|#false, default `#false`): Enter presses this button, and it is drawn in the accent colour.
- `label` (text): Text in the label column.
- `tooltip` (template): Shown when hovering the control.
- `default` (value): The value before the user changes it. `(lua)"..."` works it out when the file loads.
- `persist` (#true|#false, default `#true`): Remember the value between launches.
- `visible-if` (Lua expression): Only show the control when this is true.
- `enabled-if` (Lua expression): Grey the control out unless this is true.
- `disabled-tooltip` (template): Shown when hovering the control while it is greyed out.
- `required` (#true|#false, default `#false`): Actions can't run while the control is empty; the Run button says why.
- `width` (width): Field width: "fill" (to the form's right edge), "auto", or a number of logical pixels.

## Steps

### `run`

Start a program and show its output in the console. Result: {id.exit_code}, {id.pid}.

- argument `program` (template, required): The program: a name found on PATH, or a path (relative to the file's folder).
- further arguments `arguments` (template): The program's arguments, one per value, passed exactly (no shell).
- `command-line` (template, windows only): Windows: the arguments as one raw command line, passed exactly as written (instead of separate arguments).
- `cwd` (template): Working folder (default: the file's folder).
- `stdin` (template): Text fed to the program's input, then the input is closed.
- `wait` (#true|#false, default `#true`): Wait for the program to exit. With #false it is started and left running (a game, an editor).
- `timeout` (integer): Give up (and stop the program) after this many milliseconds.
- `expect-exit` (text, default `0`): Exit codes that mean success, e.g. "0" or "0,1". "any" accepts every code.
- `io` (word: auto | pty | pipe, default `auto`): How output reaches the console: a pseudo-terminal (colours, progress bars), pipes, or auto (pipes only for GUI programs).
- `fail-if-output` (text): A regular expression: the step fails if any output line matches it.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.
- block: `env`

### `check`

A precondition: run a program quietly; any other exit code stops the run with `message`. Checks run before any other step.

- argument `program` (template, required): The program: a name found on PATH, or a path (relative to the file's folder).
- further arguments `arguments` (template): The program's arguments, one per value, passed exactly (no shell).
- `cwd` (template): Working folder (default: the file's folder).
- `expect-exit` (text, default `0`): Exit codes that mean the check passed.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `capture`

Run a program quietly and keep what it printed. Result: {id.output}, {id.exit_code}.

- argument `program` (template, required): The program: a name found on PATH, or a path (relative to the file's folder).
- further arguments `arguments` (template): The program's arguments, one per value, passed exactly (no shell).
- `cwd` (template): Working folder (default: the file's folder).
- `expect-exit` (text, default `0`): Exit codes that count as working; any other fails the step.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `use`

A step from an add-on. Its results are {id.result}.

- argument `step` (text, required): The add-on step, as add-on.step (git.pull).
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.
- block: `with`

### `env-from`

Run a setup script and keep the environment variables it sets, for every later step.

- argument `script` (template, required): The setup script (vcvarsall.bat, a conda activate script, ...).
- further arguments `arguments` (template): Arguments for the script.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `set`

Give a name to a value for later steps: {name}, or values.name in Lua.

- argument `name` (name, required): The variable.
- argument `value` (template, required): Its value: a template or `(lua)"..."`.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.
- allowed in `on-change`

### `set-control`

Change a control's value.

- argument `control` (control id, required): The control.
- argument `value` (template, required): The new value.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.
- allowed in `on-change`

### `reset`

Put a control back to its default.

- argument `control` (control id, required): The control.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.
- allowed in `on-change`

### `put`

Put an earlier step's result into a control: a list becomes a choice's choices, anything else its value.

- argument `control` (control id, required): The control it goes into.
- argument `step` (text, required): The step that found it (its id).
- argument `result` (text, required): Which of the step's results.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `copy`

Copy a file, or a folder with everything in it.

- argument `from` (template, required): The file or folder to copy.
- argument `to` (template, required): Where the copy goes (a file path, or a folder ending in / or \).
- `overwrite` (#true|#false, default `#true`): Replace files that are already there.
- `ask-first` (#true|#false, default `#false`): When the copy is already there, ask before replacing it.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `move`

Move or rename a file or folder.

- argument `from` (template, required): The file or folder to move.
- argument `to` (template, required): Where it goes.
- `overwrite` (#true|#false, default `#false`): Replace a file that is already there.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `delete`

Delete a file, or a folder with everything in it.

- argument `path` (template, required): The file or folder.
- `missing` (word: ok | error, default `ok`): Whether a path that isn't there is fine or an error.
- `to-trash` (#true|#false, default `#false`): Move it to the Recycle Bin or Trash instead of deleting it for good.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `mkdir`

Create a folder (and any missing parents).

- argument `path` (template, required): The folder.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `find-file`

Find the first file matching a pattern. Result: its path, or nothing.

- argument `pattern` (text, required): File name pattern, e.g. "*.exe".
- `in` (template, default `.`): Folder to look in.
- `recursive` (#true|#false, default `#false`): Look in sub-folders too.
- `exclude-self` (#true|#false, default `#true`): Never match the launcher's own executable.
- `require-sibling` (template): Only match when this also exists next to the file ({stem}, {name}, {ext}).
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `read-file`

Read a text file. Result: {id.text}; Lua also gets values.id.json when it is JSON.

- argument `path` (template, required): The file.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `write-file`

Write a file: replace it (atomically), or add to it. A list in the content goes in one item per line.

- argument `path` (template, required): The file.
- `heading` (template): A line written above the content. When it is added to a file, a blank line comes first.
- `content` (template, required): What to write: a template or `(lua)"..."`.
- `format` (word: text | json-pretty | json-compact, default `text`): Write as-is, or re-format JSON.
- `newline` (word: keep | lf | crlf, default `keep`): Line endings.
- `create-dirs` (#true|#false, default `#true`): Create missing folders.
- `append` (#true|#false, default `#false`): Add to the end of the file instead of replacing it, each write on lines of its own. The first write to the file in a run still starts it afresh (unless `keep`), so several steps can make one file together.
- `keep` (#true|#false, default `#false`): With `append`: keep what earlier runs wrote too, so the file grows run after run.
- `item` (text, default `item`): What the content's list holds, for the count beside the heading: `item="model"` makes "Models to fix (54 models)". An empty list is written as "none".
- `items` (text): More than one `item`, when it isn't just `item` with an s.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `http-get`

Fetch a URL. Result: {id.status}, {id.body}. `extract` lines pull fields out of a JSON response.

- argument `url` (template, required): The URL (https:// or http://).
- `timeout` (integer, default `10000`): Milliseconds before giving up.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.
- block: `extract`, `header`

### `ping`

Check that a host answers (ICMP echo). Result: {id.ms}.

- argument `host` (template, required): Host name or IP address.
- `timeout` (integer, default `1500`): Milliseconds to wait for the reply.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `open`

Open a file, folder or URL with its default program.

- argument `target` (template, required): A file, folder or URL.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `reveal`

Show a file or folder in Explorer or Finder.

- argument `path` (template, required): The file or folder.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `message`

Show a message and wait for OK.

- argument `text` (template, required): The message.
- `level` (word: info | warning | error, default `info`): Icon and colour.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `confirm`

Ask a yes/no question; No ends the run quietly.

- argument `text` (template, required): The question.
- `yes` (text, default `Yes`): Yes button text.
- `no` (text, default `No`): No button text.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `ask`

Ask for a line of text; Cancel ends the run quietly.

- argument `name` (name, required): The variable the answer goes into.
- argument `prompt` (template, required): The question.
- `default` (template): The answer already filled in.
- `password` (#true|#false, default `#false`): Hide what is typed.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `wait`

Pause for a number of milliseconds.

- argument `ms` (integer, required): Milliseconds.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `log`

Write a line to the console and the log.

- argument `text` (template, required): The line.
- `level` (word: debug | info | warn | error, default `info`): Log level.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.
- allowed in `on-change`

### `fail`

End the run here with an error.

- argument `text` (template, required): The error message.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `stop`

End the run here, successfully.

- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

### `lua`

Run Luau code as a step.

- argument `code` (Lua code, required): Luau code. It can call run(), capture(), copy() and the rest of the Lua API.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.
- allowed in `on-change`

### `args`

Build a command line from pieces. Result: the command line as text.

- argument `name` (name, required): The name the command line is stored under.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.
- block: `format`, `replace`, `dedupe`, `raw`, `pair`, `pairs`

### `set-process-volume`

Set the audio volume of a program an earlier run step started (waits for it to open its audio). (windows only)

- argument `run` (run step id, required): The id of the run step.
- `volume` (template, required): 0 to 100.
- `timeout` (integer, default `10000`): Milliseconds to wait for the program's audio.
- `id` (name): Names the step's result for later steps: {id.field} or values.id in Lua.
- `label` (template): What the step list shows for this step.
- `if` (Lua expression): Skip the step unless this is true.
- `on-error` (word: abort | warn | continue, default `abort`): What a failure does: stop the run, warn and continue, or continue silently.
- `message` (template): The text shown when the step fails. {error} is the underlying error.
- `first` (#true|#false, default `#false`): Run before every other step ("Before starting, make sure…"), like a check.

## Parts (inside a specific block)

### `tab`

One tab page.

- argument `title` (template, required): The tab's title.
- `visible-if` (Lua expression): Only show the tab when this is true.
- block: layout items and controls

### `column`

One column.

- block: layout items and controls

### `option`

One choice. The lines in its block are its data: `Key "value"`.

- argument `name` (text, required): What the list shows (and what is remembered).
- `value` (text): The value when chosen (default: the name).
- `label` (template): Display text overriding the name and option-label.
- block: `Key "value"` lines

### `preset`

Selects the option this names whenever it is not empty (a role that fixes the server, say). While a preset applies, the user's choice is not remembered.

- argument `value` (template, required): The value to select, usually a field of another control.
- `match` (text): Match against this data field instead of the option's value.

### `on-change`

Steps run the moment the control changes. Only instant steps: set, set-control, reset, log, lua.

- block: steps

### `env`

An environment variable for the program.

- argument `name` (text, required): Variable name.
- argument `value` (template, required): Its value.

### `list-from`

Fill the choices from an add-on's list (Git branches, installed versions...).

- argument `list` (text, required): The add-on list, as add-on.list (git.branches).
- block: `with`

### `with`

A setting of an add-on step.

- argument `setting` (text, required): The setting's name.
- argument `value` (value, required): Its value: text with {names}, a number, #true or #false.

### `extract`

Name a field of the JSON response: `extract "session_id" "data[0].id"`.

- argument `name` (name, required): The name.
- argument `path` (text, required): Where the field is: a.b[0].c
- `required` (#true|#false, default `#false`): Fail the step when the field is missing.

### `header`

A request header.

- argument `name` (text, required): Header name.
- argument `value` (template, required): Header value.

### `format`

How each key/value pair is written. Default: {key}={value}

- argument `template` (template, required): The pattern, with {key} and {value}.

### `replace`

Replace text inside every pair value before it is written.

- argument `from` (text, required): Text to find.
- argument `to` (text, required): Replacement.

### `dedupe`

When two pairs have the same key, keep the first, the last, or both.

- argument `keep` (word: first | last | none, required): Which to keep.
- `ignore-case` (#true|#false, default `#true`): Treat keys that differ only in case as the same.

### `raw`

Text added to the command line as-is (skipped when empty).

- argument `text` (template, required): The text.
- `if` (Lua expression): Only when this is true.

### `pair`

One key/value pair.

- argument `key` (text, required): The key.
- argument `value` (template, required): The value.
- `if` (Lua expression): Only when this is true.

### `pairs`

Every key/value of a data block, or of the option selected in a control.

- argument `source` (data or control id, required): A data id or a control id.
- `exclude` (text): Keys to leave out, separated by commas.
- `if` (Lua expression): Only when this is true.
