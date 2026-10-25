# Launcher: tools made of steps (redesign plan)

> **Status (2026-10-10, branch `tools-redesign`).** Slice 0 is done. Slice 1a is done except the
> keyed run events and history (they come with the paths of slice 2) and one Lua environment per
> add-on. Slice 1b is built: the side-by-side editor, the step picker, Make a new tool, the plain
> run view. Checked by hand on macOS from screenshots (columns, selecting, the picker, adding);
> the rest of its hand checks are still to do. The decisions are N19–N33 in
> [`DECISIONS.md`](DECISIONS.md).
>
> **First try with a colleague (2026-10-10):** they wanted "get the branches, let me choose one,
> switch to it, get the latest" and got stuck. See [First try with a colleague](#first-try-with-a-colleague)
> for what went wrong and what changed.

## First try with a colleague

**The task:** a tool that gets the list of branches, lets them choose one, switches to it and gets
the latest changes.

**What went wrong:**
- **Their idea of the tool was four steps.** "Get the list" and "let me choose" aren't steps in
  Launcher: they're a list in the window that fills itself from Git. Nothing said so. They named the
  button "Get list of all branches" and added a second button, looking for a place for the list.
- **Their steps went into "Before starting, make sure".** The first + on screen was inside that box,
  and its cards looked like the main steps, so the steps became checks and the editor offered "Add
  the first step" below them.
- **The Branch setting was a text box** with "Type it, or tick Ask each time"; "Ask each time"
  didn't say it puts a branch list in the window.
- **File text leaked into the editor:** `(add branch="…")` on the card, `{switch-branch.previous}`
  under "This step gives you".
- "1 thing to fix" was red on red; Try it and Preview were greyed out with no visible reason;
  "Steps for:" didn't read as a choice of buttons.
- **The window:** buttons sat to the right (the empty label column), the first one was taller (the
  "main" button, which meant nothing to them), controls kept fixed widths with a side scroll bar,
  and they expected to drag controls to put them in one row or two.

**What changed:**
- A step's setting is a choice between **Always use** (a dropdown of the project's real branches
  when the setting has a list) and **Let people choose when they use the tool**, which adds the
  list to the window, filled from Git. This replaces the "Ask each time" tick box everywhere.
- The picker finds Switch branch for "list branches" or "choose a branch", and its details say the
  choice can be made in the window. Search matches every word, anywhere.
- **"Before starting, make sure"** lists checks as ticked lines, not cards, with a small "Add a
  check" that opens the picker showing only checks. The first + on screen is always for the steps.
- Problems on cards and controls are plain ("Fill in "Branch" for "Switch branch""); the file's
  wording only shows for problems the editor can't point at. Results show by name; their `{…}` form
  is under Advanced.
- "N things to fix" is white on red. Try it and Preview stay on; with something to fix they open
  Things to fix. "Steps for the button:".
- **No main button** in tools: every button looks the same (`main=` is gone from the format).
- **Controls stretch with the window** (format 2 writes nothing; they're `width="fill"` underneath),
  the label column is as wide as the longest label (none without labels), and buttons run the full
  width.
- **Drag to arrange** (from slice 4): drag a control above or below another, or onto its left or
  right quarter to put them side by side in a `row` (up to 3, sharing the width equally). A row left
  with one control goes away.
- **Text, lines and sections** are selected, changed, moved and deleted like controls (the editor
  matches what it draws to the file by where each item is written). Dropping on a section's empty
  area puts the item inside it; deleting a section keeps what it held.
- The steps column's cards fill the column and it scrolls only up and down; the picker's groups and
  steps are full-width buttons.
- **One layout everywhere:** the three columns share one frame and margin, each with its heading
  and a one-line hint at the same height; the picker's panes share one padding, so the title,
  groups, steps and details start the same distance in and their first lines line up. Small buttons
  are gone (they were shorter, and grew on hover); the smallest text is 12 px (was 11).
- **Control names** (asked for by name): Drop Down Button (`choice`), Multi Choice Tick Box
  (`choices`), Tick Box, Text Input (`text`), Number Input (`number`), File, Folder, Button, and
  for arranging: Text Label (`info`), Line, Section. "Input" left the never-used words for this.
- **Passing data (second try, 2026-10-10).** They built get list → select branch → switch → get
  latest as four controls (three buttons and a Drop Down Button) and couldn't connect them. Now:
  - every setting has a **Use…** menu: type a value, what people chose in any control (or a new
    one), or what an earlier step found; the setting shows its source ("What people chose in
    “Select branch”");
  - each result in "This step gives you" has **Put it into** a control (`put "branches"
    into="select-branch"`): a list fills a Drop Down Button's choices, anything else sets its
    value; the card says "Fills: Select branch", the control says what fills it;
  - Git's new **List branches** step gives the branches as a list;
  - Drop Down Buttons only get choices from a step (decided): the editor no longer adds
    `list-from`, and "Switch branch and get the latest" became two buttons, "Get the list of
    branches" and "Switch and get the latest".
- **Numbers for controls.** In the editor each control has a faint number before it, in window
  order; the same number shows wherever a control is named (Use…, Put it into, "filled by", the
  button tabs, card badges, the settings heading).
- **The tool window keeps its form during a run.** The run shows beside it on the right (the
  window grows; Close puts it back), its header on lines of their own; the window's buttons work
  again as soon as a run ends, so one button's results can feed the next. "What this does" is
  gone. A run is called by its button's label. An add-on's explanations now also match what the
  step printed (git's "would be overwritten" → "You have changes that aren't committed…").
- **The run beside the window is a log, in one console.** Each button press writes into it:
  the run's title, a heading for each step, what the step printed, a line for a step that went
  wrong or was skipped, and "✓ Done in …" at the end. Earlier runs stay above (Try again adds
  "…: trying again"; Close clears the log). A bar above the console has the newest run's status
  and buttons. The window's margin shows on both sides of the line between form and log, and a
  field that stretches never grows past the room it has.
- **Logs stay in the window unless the tool saves them.** A run's output only goes to the log
  beside the window; `save-logs=#true` on the tool's first line (the editor's "This tool"
  settings, or More) also keeps it in files, with Log files to open them. A step's output is kept
  in memory as well, so an add-on's explanations work either way. In Finder, `.launcher` files
  show the app's icon.
- **Opening a tool from Finder** no longer shows the home screen first: Finder's documents
  arrive before the window exists, so the first one opens in its place.

## Context

Colleagues liked Launcher, but nobody could build even a one-step workflow. To do so they needed
four things:
- KDL syntax;
- each app's command line;
- the wiring of ids, templates and Lua between the form and the steps;
- the split between `layout` and `action`.

A first redesign fixed the mechanics: a step library, a diagram, and add-ons written in KDL. But it
still spoke like a developer: node, flow, lane, pack, binding, inspector, dry run.

This plan starts from what a non-technical artist sees and says, then describes how it's built.
Nothing is built yet.

## The idea in five sentences

1. A **tool** is a list of **steps** the computer does for you when you press its button.
2. Its window has **controls**, such as a list of branches or a folder picker, so people can
   choose things before it starts.
3. A step can run **only when** something is true, for example when "Run tests" is ticked.
4. Steps can **repeat for each** thing: each ticked platform, each file in a folder.
5. If a step **goes wrong**, you choose what happens: stop, ask what to do, keep going, or do
   other steps.

Everything the app shows must fit these five sentences.

## Words: the same in the app, the files and the code

| The app says | Files and code | Meaning |
|---|---|---|
| tool | `tool` (first line of a `.launcher` file); `Tool` | A window with controls, plus the steps behind each button |
| app | — | Git, Unity, Bite, 3D Review… |
| window | `window` | What people see when they open a tool |
| control | `choice`, `choices`, `tick-box`, `text`, `number`, `file`, `folder`, `button`; `Control` | Anything in the window people use: a list, a tick box, a folder picker, a button |
| text, line, section, pages | `info`, `line`, `section`, `pages` / `page` | Arranging the window |
| button | `button` | A control that starts its steps |
| steps | `steps "<button>" { }` | What the tool does when that button is pressed |
| step | `<add-on>.<step>` in tools; `step` in add-ons; `StepDef`, `StepUse` | One thing it does |
| setting | `setting`; `Setting` | Something a step needs to know |
| Use… | A setting's value: typed, `{control}` or `{step.result}` | Where a setting's value comes from: a value, what people chose in a control, or what an earlier step found |
| Put it into | `put "result" into="control"` on a step | A step's result shown in the window: a list fills a Drop Down Button's choices |
| result | `result`; `Result` | What a step gives back, e.g. "Build folder" |
| Only when… | `only-when { }` | Skip the step unless the rule is true |
| If… / Yes / No | `if { yes { } no { } }` | Two paths |
| Repeat for each… | `repeat { for-each … }` | Do some steps once per item |
| If it goes wrong | `if-wrong="stop\|ask\|keep-going"` or `if-wrong { }` | What happens when a step fails |
| Before starting, make sure… | `before-starting { }` | Checks that run first |
| Needs Git | `needs { app "git" }` | What a step needs on the computer |
| list | `list "<name>" { }` | Where a control's choices come from (e.g. Git branches) |
| add-on | `add-on "<name>" { }`; `AddOn` | A text file of extra steps, in the Add-ons folder or copied into a tool |
| example | `example { }` in a tool; `examples/` | A ready-made tool to start from |
| Preview | `--preview` (replaces `--dry-run`) | What would happen, without doing it |
| Details | — | A step's full output |
| problem | `{step.problem}`; `Problem` | What went wrong, in plain words |

**Never in the app:**
- node, flow, canvas, lane, pack, provider, binding, output, template, inspector, form,
  config;
- KDL and Lua (the Advanced sections say "Script");
- argument, flag, parameter, exit code, stdout, stderr, process, execute, PATH, registry, regex,
  glob, boolean, string, null.

## Screens

### Home
- **Make a new tool** and **Open a tool…**.
- **Your tools:** recent tools, with "Show in folder".
- A **⋯** menu: Examples folder, Add-ons folder.

### Make a new tool
One screen, then the save dialog.
1. **Project folder (optional).**
   - Drag in a folder or Browse. Leave it empty for a general tool.
   - With a folder, Launcher shows what it found ("Git project · Unity project (Unity
     6000.0.60f1)") and puts the matching examples first.
2. **Start from:**
   - "Nothing (an empty tool)";
   - general examples: "Copy files to a shared folder", "Start my work apps", "Back up a folder";
   - with a project folder, also: "Get the latest changes", "Switch branch and get the latest",
     "Build the game", "Process images with Bite", "Open models in 3D Review".
3. **Where to save it.** Launcher always asks, in the save dialog.
   - It starts in the project folder if there is one, otherwise in the last folder used.
   - The example's name is the starting file name.
   - Then the editor opens.

### The editor: side by side
```
+---------------------------------------------------------------------------+
| Build the game   Undo Redo   2 things to fix                       Save    |
+--------------------+------------------------------+-----------------------+
| WHAT PEOPLE SEE    | WHAT IT DOES                 | SETTINGS              |
|                    | When Build is pressed:       | Switch branch (Git)   |
| Branch [main    v] | Before starting, make sure:  |  Branch               |
| [x] Run tests      |   Git is installed           |  [main          v]    |
|                    |   No unsaved changes         |  (o) Let people choose|
| [ Build ]          | +--------------------------+ |  Only when…  (always) |
|                    | | Git  Switch to the       | |  If it goes wrong     |
| + Add a control    | |      chosen Branch       | |  [Stop          v]    |
|                    | +--------------------------+ |  This step gives you: |
|                    |             (+)              |   Previous branch     |
|                    | +--------------------------+ |  > Advanced           |
|                    | | Git  Get the latest      | |                       |
|                    | |      changes             | |                       |
|                    | +--------------------------+ |                       |
|                    |             (+)              |                       |
+--------------------+------------------------------+-----------------------+
```

**What people see** (left) is the live window.
- Click a control to change it.
- Drag a control to reorder it, or to sit beside another (up to 3 side by side).
- **+ Add a control** offers the control kinds with pictures (list, tick box, text, number, file,
  folder, button…), plus layout: text, line, section and pages.
- Clicking a button shows its steps in the middle.

**What it does** (middle):
- "When **Build** is pressed:".
- The "Before starting, make sure…" box, already listing what the steps need.
- The steps, as cards joined by arrows.
- A **+** between cards opens the step picker. Dragging moves cards.
- No zoom: it scrolls like a page.

**Settings** (right): the settings of the selected step or control.

**Linked:** selecting a control lights up the steps that use it, and the other way round.

**Things to fix:**
- Red marks on cards and controls.
- The top-bar button walks through them, one sentence each, with a way to fix it: "'Switch
  branch' needs a branch. [Choose one]".

**Save:**
- **Save** (Ctrl+S). The editor had Preview and Try it beside it; they were taken out, as not worth
  it for people who aren't technical: Done goes back to the tool, where its buttons run it, and a
  button's right-click Preview (and `--preview`) still lists what it would do.
- A file that was edited by hand says once: "Saving will tidy this file's layout."
- Saving a tool that uses add-ons from the Add-ons folder, without carrying them, offers once:
  "Include the Perforce add-on so this tool works on other computers? [Include it]".

### The step picker
```
+-----------------------------------------------------------------+
| Add a step                  [ Search steps...               ] x |
+---------------+-------------------------+-----------------------+
| Suggested     | GIT                     | Switch branch         |
| Git         < | Pull                    |                       |
| Unity         |  Get the latest changes | Moves the project to  |
| Bite          | Switch branch         < | another branch.       |
| 3D Review     |  Move to another branch |                       |
| .NET          | Commit                  | On the card:          |
| Files/folders |  Save your changes      |  "Switch to main"     |
| Internet      | Push                    | You choose: Branch    |
| Ask or tell   |  Send your changes      | Needs: Git            |
| Choose/repeat |                         |                       |
| Advanced      |                         |        [ Add step ]   |
+---------------+-------------------------+-----------------------+
```

**How it opens.** A large panel opens from any **+**, beside it and over the window column. Esc
or a click outside closes it.

**Groups**, by app:
1. **Suggested**:
   - *Suggested for this project:* steps for the apps found in the project folder.
   - *What comes next:* when opened after a step, what usually follows it. After "Build the
     game": "Copy the build", "Open the build folder". Add-ons say this for each step.
2. The apps: Git, Unity, Bite, 3D Review, .NET.
3. Then: Files and folders, Internet, Ask or tell, Choose and repeat (If…, Repeat for each…),
   Advanced (Run a command, Script, Set a value…).

**Entries:** the app's icon, the step's name, and one plain line. Selecting one shows the details
pane:
- what it does;
- its sentence as it will appear on the card;
- what you choose;
- what it needs.

**Missing apps:** a step whose app isn't on this computer is still shown, with "Unity isn't
installed" in grey. It can be added, since the tool may be for someone else.

**Search:**
- It starts focused and searches every group.
- It matches names, descriptions and the everyday words add-ons list for each step, so "update"
  finds Pull.
- Arrow keys move; Enter, double-click or **Add step** adds the step at that **+**.

**After adding,** the new card is selected, and its first empty setting is ready to fill in.

**If and Repeat on a card:** a card's right-click menu also has "Only do this when…" and "Repeat
this for each…".

### Cards
Each card shows:
- the app's icon;
- the step's name, which can be renamed;
- a sentence with its values in bold: "Switch to the branch chosen in **Branch**";
- badges: "Asks: Branch", "Only when Run tests is ticked", "If it goes wrong: ask", "Needs Unity
  6000.0.60f1" (red when it isn't installed).

Only settings that differ from their starting value are shown.

Shapes:
- **If…** splits into **Yes** and **No** columns.
- **Repeat** is a box around its steps: "Repeat for each ticked Platform".
- An "If it goes wrong" path leaves to the right in orange.

### The settings panel

**A step:**
- Its name, and what it does in one sentence.
- Its settings, required first, each either **Always use** a value or **Let people choose when they
  use the tool**.
- Text fields have **Insert**, for control values, results and special values such as today's
  date. Inserted values show as chips.
- **Only when…**: rules that read as sentences, e.g. [Run tests] [is ticked], joined with "and"
  or "or".
- **If it goes wrong**: Stop, Ask what to do, Keep going, or Do other steps…
- **This step gives you…**: its results.
- **Advanced** (collapsed): advanced settings, the step's id, a note, Script values.

**A control:**
- Its label.
- Its kind:
  - Drop Down Button, or shown as buttons;
  - Multi Choice Tick Box;
  - Tick Box;
  - Text Input;
  - Number Input, optionally a slider;
  - File;
  - Folder;
  - Button.
- Its choices, or "Fill the list from…" (e.g. Git branches).
- "Starts as", "Remember the last value", "Only show when…".
- **Advanced**: data for each choice, presets, patterns.
- A button also says which steps it runs.

### Advanced: tucked away, always there
- Every panel has a collapsed **Advanced** section.
- The step picker has an Advanced group.
- The editor's **⋯** menu has "Show the file's text".
- Nothing is switched off.

### The tool window (people running it)

**Before running:**
- The tool's name, its controls, and **What this does ▸**, which lists the steps as sentences.
- A control that still needs a value keeps the button off; hovering says why: "Choose a Branch
  first".
- A missing app shows a banner: "This tool needs Git, which isn't installed. [How to get Git]".

**While running:**
- "Step 3 of 5: Building the player…", with a progress bar when the app reports one.
- The steps, with plain statuses: Waiting, Doing, Done, Skipped because Run tests isn't ticked,
  Didn't work, Stopped.
- **Details** opens a step's output. **List | Diagram** switches the view.

**When a step goes wrong:**
- The add-on's explanation, if it has one; otherwise the app's last error line.
- **Try again**, **Show details**, **Copy details for help**. The last one copies the step, the
  problem, the last output lines and the versions, ready to send to a developer.

**When it's done:** "Done in 2 minutes", and Back. The log files are in the **⋯** menu.

### Messages (examples of the tone)
- **Downloaded tool:** "This tool came from the internet. It uses Git and Unity and changes files
  in D:\Game. Only open tools from people you know. [Open it] [Don't open]"
- **Missing add-on:** "This tool uses steps from the Perforce add-on, which isn't on this
  computer. Ask whoever made the tool to include the add-on in it, or put the add-on in your
  Add-ons folder. [Open the Add-ons folder]"
- **Newer Launcher needed:** "This tool needs Launcher 0.3 or newer. You have 0.2."
- **Old file:** "This tool was made for an older Launcher and can't be opened. Make it again with
  Make a new tool."

### Safer defaults
- **Delete** moves to the Recycle Bin or Trash. "Delete permanently" is under Advanced.
- **Copy** asks before replacing: "If a file is already there: Ask / Replace / Keep the old one".
- **Write to a text file** adds to what earlier steps of the run wrote to the same file ("Add to
  what earlier steps wrote", on), so several steps make one file, with a heading each; the run's
  first write still starts the file afresh, so a run never piles onto the last one ("Keep what
  earlier runs wrote", under Advanced, for a file that grows). A list goes in one item per line,
  counted beside the heading ("Models to fix (54 models)"), and an empty one is written "none".
  Engine: `write-file`'s `heading`, `append` and `keep`, and `item`/`items`, which the
  translator fills in from the result's own `item=` (a catalog parameter marked `filled`, so it
  is never a setting); the runner remembers the files the run
  wrote (and a Retry carries them), `Effects::append_file` adds on a line of its own.
- Folder and file fields take a folder or file dragged in from Explorer or Finder.

## Writing rules
These are for us and for add-on authors, and they go in the add-on guide.
- **Plain words:** words a ten-year-old knows, sentence case, labels and help at most 15 words.
- **Names:** steps and buttons start with a verb that names the result ("Build the game"). App
  actions keep the app's own button word, with a plain line ("Pull: get the latest changes").
- **Messages:** say what happened and what to do next, without blame. Codes and raw output go in
  Details.
- **Everyday terms:** units in words ("10 seconds"); folders by name ("Same folder as this tool",
  not "."); file kinds by name ("3D models (FBX)", not "*.fbx").
- **One step = one thing a person would say out loud.** Add-ons hide the plumbing (finding an
  app, reading its output) inside the step.
- **The test:** `plain_words.rs` scans add-on text (names, `says`, `about`, labels, help,
  `explain`, search words) and the app's interface strings for the "never" words.

## Decisions taken in discussion

| # | Decision |
|---|---|
| 1 | **Steps diagram.** Cards and arrows laid out automatically top to bottom, with **+** between them; Yes/No columns, orange "if it goes wrong" paths, Repeat boxes. No free placement and no wires: the steps form a tree, so files hold no positions. It scrolls, with no zoom, and runs one step at a time. |
| 2 | **Step picker.** A panel with details opens from any **+**: groups by app, steps, and a details pane. Search with everyday words. "Suggested for this project" and "What comes next". Steps whose app is missing are shown, marked. |
| 3 | **Add-ons are KDL text, one `add-on { }` block,** with no images and no other files. Built-in ones are compiled in; others go in `<app data>/add-ons/`, opened from the home screen's ⋯ menu. |
| 4 | **Self-contained tools.** "Include add-ons in this tool" copies each add-on the tool uses into the file. The tool's copy wins, and the editor offers updates. Built-in add-ons are never copied. |
| 5 | **Window:** controls in rows of up to 3; sections and pages hold rows; auto layout (N15). |
| 6 | **Controls, not questions.** Everything in the window is a control, buttons included. There are fewer kinds (above), and "Use… → a new control" adds one. |
| 7 | **Settings get their values four ways:** a fixed value, Let people choose, a result, or text with chips. Rules are point-and-click, with Script under Advanced. |
| 8 | **Repeat for each:** ticked choices, files of a kind in a folder, lines of a result, or a number of times. |
| 9 | **Run view:** a list by default; **Diagram** shows the read-only diagram with statuses. |
| 10 | **v1 files are dropped**, with the "older Launcher" message. |
| 11 | **The editor owns the file.** It prints canonically, and a `//` comment directly above an item is its note. Supersedes N12. |
| 12 | **First add-ons.** Built in: **Core** (run a command, script, set a value, wait, find an app), **Files and folders** (copy, move, delete, make, open, show in Explorer/Finder, read, write), **Ask or tell**, **Internet**. Then **Git**, **.NET**, **Unity**, **Bite**, **3D Review**. |
| 13 | **The Bite and 3D Review CLIs are ours**, and follow one contract (below). |
| 14 | **The thing people make is a "tool".** |
| 15 | **Editor side by side:** what people see, what it does, settings. Click **+** to add; drag to move. |
| 16 | **One vocabulary** across the app, the files and the code. |
| 17 | **Advanced** sits in collapsed sections everywhere and is always available. |
| 18 | **Plain problems** with "Copy details for help"; add-ons explain known problems. Delete goes to the Recycle Bin or Trash; Copy asks before replacing. |
| 19 | **Tools explain themselves:** sentences on cards, and "What this does". |
| 20 | **Make a new tool:** the project folder is optional, and Launcher always asks where to save, starting in the project folder if there is one. |

## How it's built

### Tool file (format 2)

```kdl
tool "Build the game" format=2 requires="0.2" os="windows"
window {
    choice "branch" label="Branch" { list-from "git.branches" folder="." }
    row {
        choice "mode" label="Mode" show-as="buttons" { option "Debug"; option "Release" }
        choices "targets" label="Platforms" { option "Windows"; option "Android" }
    }
    tick-box "tests" label="Run the tests" starts=#true
    button "build" label="Build"
}
steps "build" {
    before-starting { git.no-unsaved-changes }
    git.switch-branch "switch" branch="{branch}"        // Let people choose
    dotnet.build "build" configuration="{mode}" if-wrong="ask"
    if "tests-on" { when "tests" "is-ticked"; yes { dotnet.test "test" configuration="{mode}" } }
    repeat "platform" { for-each "{targets}"; unity.build-player "player" platform="{platform.item}" }
    files.copy "copy" from="bin/{mode}" to="{out}" {
        only-when { when "out" "is-not-empty" }
        if-wrong { ask-tell.message "tell" text="Copying didn't work: {copy.problem}" }
    }
}
add-on "perforce" name="Perforce" version="1.2" { … }   // a copy carried by the tool (Include add-ons)
```

**Model** (`crates/launcher-core/src/config/model.rs`; `Config` is renamed `Tool`):
- `Header { name, format, requires, os, note }`, plus:
  - `uses`: the add-ons from the Add-ons folder, with their versions;
  - `carried`: the add-ons copied into the file.
- **Window items:** `Row` (at most 3), `Section`, `Pages`, `Info`, `Line`, `Control` (buttons
  included).
- **Steps:** `Steps { button, before_starting, body }`. The body holds `StepUse`, `If` and
  `Repeat`.
- **A step:** `StepUse { id, def, settings, only_when, if_wrong, note }`.
- **Rules:** `Rule = All | Any | When | Script`.
- **If it goes wrong:** `IfWrong = Stop | Ask | KeepGoing | Path`.

**Rules for the file:**
- Steps, Ifs and Repeats have ids, which the editor makes from their names. Ids share the
  `{name}` space with controls; results read as `{id.result}`.
- A step from a missing add-on stays as written: it shows as a grey card and prints back
  unchanged.
- A tool with one set of steps and no button gets a **Start** button.
- **Examples:**
  - a tool with an `example { name about suggest-if }` block is offered by Make a new tool;
  - built-in examples are embedded from the repo's `examples/`, and more come from
    `<app data>/examples/`;
  - this replaces `templates/` and `<app data>/templates`.
- **Kept under Advanced**, because tools like a game launcher need them: `value`, `data`,
  `script`, `on-change`, `(lua)` values, presets, data for each choice.
- **Dropped:** `columns`, `spacer`, the string forms of `visible-if` and `enabled-if`, and
  `action=`.

### Add-ons

```kdl
add-on "git" name="Git" version="1.0" requires="0.2" icon="git" {
    suggest-if { exists ".git" }
    list "branches" { setting "folder" type="folder" starts="."; script #"""..."""# }
    step "pull" name="Pull" says="Get the latest changes" {
        about "Gets the latest changes from the shared copy of the project."
        search-words "update" "get latest" "download changes"
        comes-next "git.switch-branch" "unity.build-player"
        setting "folder" type="folder" starts="." label="Project folder" advanced=#true
        setting "ff-only" type="tick-box" starts=#true label="Only update if no merge is needed" advanced=#true
        result "commit" label="Latest change"
        needs { app "git" help="Install Git from https://git-scm.com" }
        explain "would be overwritten" "You have unsaved changes. Commit or stash them first."
        does {
            run "git" "-C" "{folder}" "pull" { when "ff-only" "is-ticked" { arg "--ff-only" } }
            capture "git" "-C" "{folder}" "rev-parse" "HEAD" id="head"
            set "commit" "{head.output}"
        }
        example folder="." { answer "git" output="3f2a9c1" }
    }
}
```

**Text only.**
- An add-on is one `add-on { }` block of plain text.
- Scripts are written inline, and icons are names from Launcher's built-in set.
- No images, no binary files, no other files. That is what lets a tool carry it.

**A step's body** is one of:
- `does { }`: steps in shorthand, with `{setting}` resolved inside the step;
- `script`: Luau that gets `settings` and returns `results`;
- `built-in="…"`: built-in add-ons only.

**Built-in steps.** Today's 27 step kinds become built-in steps of `core`, `files`, `ask-tell`
and `internet`. A new built-in step is one declaration plus one Rust function, not about 7 places
in 5 files.

**Arguments** follow Bite v2: one argument per item, with `when` and `switch` groups.

**Setting kinds:** text, long-text, number, tick-box, choice, choices, file, folder, plus list and
table. Attributes:
- label, help, starts, required, pattern, must-exist, advanced, os;
- a file's `extensions` (`extensions="json, fbx"`): a new control made for it with Use… takes
  the same ones, and it reads only a file control limited to some of them (the window's `file`
  takes `extensions` too: Browse… shows only those, and another kind of file isn't taken);
- a file or folder's `several=#true`: it also takes a list of files an earlier step found; text's
  (or long text's) takes any list, one item per line (`render_lines`; a Luau step gets the
  lines as text). A built-in step's text setting can be made `long-text` and `several` (Write to
  a text file's What to write);
- a `result`'s `item` (`item="model"`, and `items=` when it isn't just an s more): what a list
  holds, for counts like "54 models" (a list of files holds files, any other list items);
- a `result`'s `type` (text, number, file, folder, files, list; text when left out): a setting
  uses only a result it can take (`SettingDef::takes_result`: a folder a folder, or a list of
  files when it takes several; text any one value; a number a number), in Use… and in the checker;
- **Only choices that work.** Wherever the editor offers a choice, it offers only ones that can
  work, and the checker reports the rest (a file written by hand, or something changed since):
  - Use… lists controls and earlier results the setting can take (kind, extensions, and for a
    drop-down with fixed options, a drop-down whose options are among them);
  - Put it into lists controls that can hold the result (a list fills a drop-down's or tick boxes'
    choices; one value goes into a control of its kind, or text);
  - a typed value it can't take (not a number, another kind of file, not one of the options) is
    said under the field; a control's Starts as is picked from its options, or checked;
  - a control whose extensions or options no longer suit a step that uses it says which step;
  - deleting a control, a section or a step clears the settings that were just it, and says so;
  - a step can't be dragged above a step whose results it uses, nor below one that uses its own;
  - "+ A new … for this" reuses a same-named control only when the setting can take it;
  - the step picker can't add a step for another OS than the tool's `os`;
  - the checker also reports a result used before its step runs, or from another button's steps.
- min and max;
- `option` or `list-from`;
- `show-when` and `enable-when`.

**Other parts of a step:**
- `says`: the card sentence;
- `about`: one sentence;
- `search-words`;
- `comes-next`: "What comes next";
- `needs`: app with help, file, os, check, script;
- `explain`: a phrase in the output, then plain advice;
- `example`;
- `since`, `check-step`, `quiet`, `replaced-by`.

**Add-on level:** `suggest-if` (`exists` and `glob`, evaluated in Rust, so no trust is needed)
and `list`.

**The library** (`crates/launcher-core/src/addons/`):
- **Built-in add-ons** come from `addons/*.kdl`, embedded by a build script that lists the
  folder. They can't be in `crates/*/src`, because nothing product-specific goes there (N15), and
  words like "unity" count.
- **Add-ons folder:** `paths::app_data_subdir("add-ons")`. It is reread when a tool opens, and
  by the picker's Reload.
- **A broken add-on** shows its problems in the picker. It never blocks the app or the other
  add-ons.
- **Carried copies** (decision 4):
  - **Include add-ons in this tool** (the editor's ⋯ menu, per tool) copies each add-on from the
    Add-ons folder that the tool uses, word for word, into the tool file.
  - The tool's copy is used even when the add-on is installed, so the tool behaves the same
    everywhere.
  - When the installed copy is newer (same major version), the editor offers "Update the copy in
    this tool".
  - Built-in add-ons are never copied; `requires=` covers them.
  - A carried add-on is part of the tool file, so the tool's own trust check covers it.
  - "Save to my Add-ons folder" copies one out.
- **Name clashes:**
  - A user add-on can't take a built-in add-on's name.
  - Between two installed add-ons, the higher version wins. A tie loads neither, and the message
    names both files.
- **Versions** are MAJOR.MINOR:
  - A different major version is an error; an older minor version is a warning.
  - A breaking change gets a new step name plus `replaced-by=`.
  - "Surface" goldens for the built-in add-ons catch breaking changes that weren't announced.
- **Trust (N9):** an installed add-on with a download mark runs nothing until trusted. Trust is
  stored as path plus hash in `trust.kdl`.

### Engine (`crates/launcher-core/src/engine/`)

**Scopes, not inlining.** Settings go in and results come out. This keeps:
- the step boundary, for rows, history and Try again;
- Luau receiving `settings`;
- an add-on's names from leaking into the tool;
- problems naming the right file.

**New modules:**
- `walk.rs`: the walker.
- `call.rs`: settings, dispatch, scopes, needs.
- `builtin.rs`: today's `execute` arms (`runner.rs:829-1251`), reading typed settings, plus
  `capture`, `find-app` and `trash`.
- `settings.rs`: typed values with their starting values. This removes the defaults duplicated
  in `load.rs`.

**Results:**
- They land in the caller's scope as `fields[id][result]`.
- They are also kept when a step goes wrong, plus `problem`, so an "if it goes wrong" path can
  read `{build.problem}`.

**Luau:**
- `Vm::call(src, origin, settings) -> Dyn`.
- One environment per origin: the tool, and each add-on.
- The cache is keyed by (origin, span, expr), and problems name `git.kdl:42`.

**Walker:**
- An If checks its rule in Rust; only Script goes to Lua.
- When an only-when is false, the step is Skipped, with the reason in plain words.
- **If it goes wrong:**
  - Stop;
  - Keep going;
  - Ask: Try again, Skip this step, or Stop. This replaces `warn`;
  - Path: run the path, then continue.

**Repeat:**
- **Sources:**
  - ticked choices, or a list;
  - files of a kind in a folder: sorted, subfolders optional, patterns under Advanced;
  - lines of a result, skipping blanks;
  - a number of times.
- Each pass has a child scope with `{id.item}`, `{id.number}` and `{id.count}`. Files add `name`,
  `kind` and `folder`; choices add their data.
- At most 1,000 passes.
- If a pass goes wrong: Stop (the default), Keep going and list the problems at the end, or Ask.

**Row keys:**
- **Keys:** the step, If or Repeat id, plus `#n` for passes and `id/step-n` for Lua `step()`.
- **Events:** `Started`, `RowsAdded`, `RowStarted`, `Output`, `Log`, `Attached`, `RowFinished`,
  `Finished`.
- **History:** a keyed `steps.json`, plus `NN-<key>.log`.
- **Try again** resumes at a key, replaying the paths and passes already chosen.

**Needs:**
- The main path's needs are checked before anything runs; then the "before starting" checks run.
- A path checks its own needs when the run reaches it.
- While the window is open, needs are checked live; a missing app turns the button off and shows
  the banner.

**Preview:**
- One row per step.
- A Luau step that can't be previewed uses its `example` results.
- `--answer KEY=…` matches by key.

**Problems:** `Problem { plain, detail, target, fix }`. The app shows `plain` plus a fix button;
`launcher check` prints both.

**Progress:** the console reads `OSC 9;4`, through a pty or pipes, and feeds the row's progress
bar and the taskbar.

**Trash:**
- Windows: `IFileOperation` with recycle-on-delete.
- macOS: `NSFileManager trashItemAtURL`, through objc2.
- Preview and the test effects only record it.

### Editor internals (`crates/launcher/src/editor/`)

**Edits:**
- `config/tool_edit.rs` covers: insert, move, delete, set a value, If, Repeat, paths, rules,
  rename with references, and window edits.
- **Use… → a new control** creates the control (a Drop Down Button starts empty: a step fills it
  with "Put it into"); the menu lists the controls already there first.
- `config/print.rs` prints in a fixed order. `load(print(t)) == t`, and printing is idempotent.
- Undo keeps the printed texts: 200 entries, merged within 1.5 s.

**Columns:**
- **Window column:**
  - an `egui::SidePanel` with `ui/form.rs` in design mode;
  - select, drag with `dnd_*` (side by side, up to 3);
  - show-all for hidden controls;
  - the real window width (`FormUi::need`);
  - overlays that take no space (L2.13, L2.28).
- **Steps column:**
  - a `ScrollArea` holding the diagram (`diagram.rs`, pure layout with geometry goldens);
  - measured bottom up and placed top down;
  - cards 240 wide, a **+** in each gap, right-angled arrows, Yes/No columns;
  - the run view reuses it, read-only.
- **Settings column:**
  - a `SidePanel` (`settings.rs`, `ask.rs`, `insert.rs` for chips, `rules.rs`);
  - Advanced sections reuse `props.rs` and `luaedit.rs`.

**Step picker** (`picker.rs`):
- An `egui::Area` panel beside the **+**, over the window column.
- Three panes: groups, steps, details.
- Search over names, `about` and `search-words`.
- "Suggested for this project" comes from `suggest-if` detection, run on a thread.
- "What comes next" comes from the previous step's `comes-next`.
- Steps whose app is missing are marked, using the same `needs` check as the banner.
- Keyboard: arrows and Enter.

**Things to fix** lists the `Problem`s, each with its fix button.

**Removed:**
- `editor/outline.rs` and `tests/edit_roundtrip.rs`;
- most of `config/edit.rs`. `notes`, `string_repr` and `ident_repr` stay.

### Tool window and run view
- **Window** (`ui/form.rs`):
  - rows use `Place::RowFirst/RowNext`;
  - rules are evaluated in Rust;
  - `Texts` and `Conditions` are keyed by control id, which fixes the `(lua)` preset leak.
- **Lists** (`eval.rs`):
  - a list runs again when its settings' values change, or on Refresh;
  - `(lua)` choices and starting values record which values they read (`lua/bind.rs`) and run
    again when those change.
- **Run view** (`ui/run.rs`):
  - plain statuses and keyed rows;
  - a path's rows are inserted, indented, when the run reaches them;
  - Repeat passes are child rows ("2 of 3: Android");
  - the Ask dialog;
  - **Copy details for help**;
  - **List | Diagram**.
- The same changes go into `headless.rs` and `editor/bottom.rs`.

### Our own apps: Bite and 3D Review

**CLI contract** (in the add-on guide):
- a console binary with `--version` and long flags;
- no prompts, and no stdin unless asked;
- unknown or value-less flags exit with 2;
- exit 0 only when everything worked, otherwise 1;
- `--report <file.json>`, whose fields become results (an app whose own command line holds
  names its users choose may call it something else: Bite's is `--run-report`);
- `OSC 9;4` progress plus plain lines.

**Finding apps** (`core.find-app`), in this order:
1. PATH;
2. Windows App Paths;
3. the Uninstall key's `InstallLocation`;
4. the macOS bundle id (LaunchServices);
5. known folders.

**Bite add-on** (`addons/bite.kdl`; built). One step for now.
- **Run a Bite workflow** (`bite run`): a workflow saved from Bite, a folder of images, save to,
  if the files are already there (do as the workflow says, or replace them: `--overwrite`),
  whether to stop the tool when an image doesn't work, and under Advanced how many at once
  (`--jobs`) and a report file. Results: files saved and images that didn't work (lists), how
  many were done, skipped and didn't work. With Stop off, an image that didn't work is logged
  and listed, not a problem.
- A workflow names its own inputs and outputs (`--input-1`, `--output-image-1`; the CLI names
  set in Bite's Inspector), so the step asks `bite describe` for them first. The images folder
  goes to the one Input an output reads from (a workflow with two is refused); every output
  goes to Save to: an Image Output's files straight in, a Text or Flipbook Output's file under
  the name the workflow gives it (or `<name>.txt` / `<name>.png`). An output without a CLI name
  is refused, since it would write where the workflow says.
- What Bite needed (in Bite's repository, with this add-on): `bite --version`; exit 2 for a
  command line it can't follow (an unknown command, a name the workflow doesn't have, a flag
  without its path, an Input left out, a workflow that isn't there), 1 when an image failed or
  the run stopped, 0 only when every image worked; `--run-report <json>` (counts, outputs,
  failures with file and problem, and `problem` when the run stopped, written however it ends);
  `bite describe`. Before, a run exited 0 with images failed, ignored unknown flags, and a
  mistyped output name wrote beside the source images. A Bite without `describe` gets "This
  Bite is too old".
- Bite is found on PATH, then through the Uninstall key
  `{7F3A9C41-5D82-4E17-9B6C-2A8E4D1F0C93}_is1`'s `InstallLocation`, then by the bundle id
  `com.bite.app`. `bite` is beside the editor: `<install>\bite.exe`,
  `<app>/Contents/MacOS/bite`.
- What a script step's `run()` and `log()` print is now in the step's log (`Runner::lua` lends
  it to the Lua host), as a run step's output is: `explain` reads it ("no decode delegate",
  "improper image header"), and saved logs keep it. Before, it reached only the console, so no
  script step's explanation could match its program's output (3D Review's Git LFS one included).
- Starter: `examples/starters/run-bite-workflow.launcher`.
- Later: **Open in Bite**; settings built from `bite describe` (a folder per Input).

**3D Review add-on** (`addons/3dreview.kdl`; built). Its id is `review3d`, not the file's name: a
name starts with a letter (`check_name`), so neither `3dreview` nor `3d-review` can be one.
- **Run an optimization preset** (`review-cli optimize --preset`): a preset saved from the Opt workspace, models
  (a folder, one file, or a list an earlier step found, like an audit's Models to fix), subfolders, save to, if the files are already there, and under Advanced
  a name suffix and a report file. It passes nothing that would replace what the preset holds (no
  `--lods`, `--packaging`, `--hierarchy`). Results: files saved and models skipped (lists, from
  the report's `outputs` and its `skipped` entries), how many were done and skipped. It
  replaced an Optimize models step whose own LOD levels and packaging overrode a chosen preset's.
- **Audit models** (`review-cli audit --profile`): an audit preset saved from the Aud workspace,
  models, subfolders, when a model needs fixing (errors; warnings or errors; anything), whether to
  stop the tool when one does (off: a next step fixes them), and under Advanced a report file.
  Results: models to fix and models that passed (lists of files, from the report's `flagged`
  entries and the rest), how many of each, how many were checked, errors and warnings found.
  Write to a text file puts the lists in one report, a heading each. It replaced a Check models step that picked Unity / Unreal /
  any engine rules. A setting that is just `{audit.to-fix}` gets the list itself, not its items
  joined into text (a path may hold a comma).
- **Model stats** (`review-cli info`): polygons, triangles, vertices, GPU vertices.
- **Open in 3D Review:** Launcher doesn't wait for it (`cmd /c start`, `open -a`); with no model
  it just starts 3D Review.
- 3D Review is found on PATH, then through the Uninstall key
  `{B6F4B0E9-2E1B-4C3A-9D2F-3D5E7A1C0F42}_is1`'s `InstallLocation`, then by the bundle id
  `com.psmyles.3d-review` (or `.dev`). `review-cli` is beside the viewer:
  `<install>\review-cli.exe`, `<app>/Contents/MacOS/review-cli`.
- Each step reads `--report` with `read_file` + `json.decode` and returns its fields as results,
  and carries its own copy of the finder: both wait on the engine (report-to-results, one Lua
  environment per add-on, `core.find-app`). It passes no `--progress-bar on` until slice 5 reads
  `OSC 9;4`.
- Starters: `examples/starters/run-preset.launcher`, `audit-models.launcher` and
  `audit-and-optimize.launcher` (audit, then run an optimization preset on just the models to fix).

## Slices

Each slice ends with `cargo xtask check` green.

**0. Fixes on today's engine.**
- **Scope:**
  - the `Vm::eval_str` cache key (`lua/mod.rs:294`);
  - `Effects::capture(cx, spec, stop)` watches Cancel and a timeout and kills the process tree
    (`runner.rs:924`, `effects.rs:409`);
  - the `--answer` wording (`effects.rs:743`);
  - Lua `run()`'s `io`.
- **Done when:**
  - unit tests pass;
  - an e2e test passes where Cancel during a check kills argdump.

**1a. Add-ons, tool format 2, scoped engine (no UI).**
- **Scope:**
  - The add-on loader and library: text-only blocks, plus add-ons carried inside tools.
  - Built-in `core`, `files`, `ask-tell` and `internet`.
  - `git`: fetch, switch branch, pull, no unsaved changes, branches.
  - `dotnet`: build, test.
  - Format 2 for steps, rows and `list-from`.
  - A **temporary internal v1 reader** that turns today's files into the same model, so the
    runner tests and `xtask e2e` prove the new engine before anything is ported. Users never
    see it.
  - The printer.
  - Scopes, Luau `call`, origins, keyed events, history and Try again.
  - Lists and needs.
  - `Problem`; `says`, `explain`, `search-words`, `comes-next`; trash; Copy's "if already there".
  - Lua `run()`/`log()` in row logs, and `step()` rows.
  - `launcher check` for add-ons; `--help-tool` (replaces `--help-config`), `--help-steps` and
    `--preview`.
- **New files:**
  - `addons/*.kdl`;
  - `crates/launcher-core/build.rs`;
  - `src/addons/{mod,load,library,detect}.rs`;
  - `src/config/{print,rule,read}.rs`;
  - `src/engine/{walk,call,builtin,settings}.rs`;
  - `src/platform/trash.rs`.
- **Changed files:**
  - `config/{model,load,validate,catalog,header,summary}.rs`;
  - `engine/{runner,history,args,effects}.rs`;
  - `lua/{mod,bind,api,lint}.rs`;
  - `eval.rs`;
  - in the app: `ui/run.rs`, `headless.rs`, `cli.rs`.
- **Done when:**
  - These stay green on v1 files read by the temporary reader:
    - `examples.rs`;
    - the runner tests (re-keyed only);
    - `xtask e2e`.
  - New tests pass:
    - `tests/addons.rs`;
    - problem goldens for tools and add-ons;
    - `print_roundtrip.rs`;
    - runner tests: scopes, results, needs first, Try again by key;
    - Lua isolation;
    - a carried add-on taking precedence over an installed one;
    - trash;
    - `plain_words.rs` over the add-ons.
  - An e2e test passes with an add-on in `LAUNCHER_DATA_DIR/add-ons`.
  - `xtask ttfp` stays at 150 ms or less.

**1b. Editor, picker and home: the first usability-test build.**
- **Scope:**
  - The three columns: a run of steps, plus "before starting".
  - The step picker, complete: groups by app, search, details, Suggested for this project, What
    comes next, missing apps marked.
  - Settings with Always use / Let people choose, and If it goes wrong (Stop, Ask, Keep going).
  - The basic control kinds.
  - Things to fix; Save.
  - Home: Make a new tool (optional folder, examples, the save dialog), Open, Your tools, ⋯.
  - Tool window: plain statuses, Details closed by default, Copy details for help, What this
    does, files dragged in.
  - `plain_words.rs` over the UI strings.
  - Port `git-build`.
- **Files:**
  - `crates/launcher/src/editor/{mod,diagram,picker,settings,ask,bottom,props}.rs`;
  - `crates/launcher/src/ui/{form,home,document,run}.rs`;
  - `crates/launcher/src/app.rs`;
  - `crates/launcher-core/src/config/tool_edit.rs`.
- **Done when:**
  - The `tool_edit` session golden and the diagram goldens pass.
  - A picker test passes: search words, Suggested, What comes next, missing-app marking.
  - Checked by hand on Windows and macOS.
  - `xtask idle` is about 0 with the editor open.
  - **The usability test** is done. Re-order slices 2–6 by what it shows.

**2. Only when, If, If it goes wrong paths, Insert, results.**
- **Scope:**
  - rules as sentences;
  - Yes/No columns;
  - orange paths;
  - Ask's three answers;
  - results kept when a step goes wrong;
  - Try again inside paths;
  - Insert with chips;
  - "This step gives you";
  - needs checked live;
  - the v1 reader maps simple Lua conditions to rules.
- **Done when:** the walker, row-insertion and layout tests pass, and it's checked by hand.

**3. Repeat for each.**
- **Scope:**
  - the four sources;
  - file kinds;
  - pass rows;
  - Try again at the failed pass;
  - the limit;
  - the box in the diagram;
  - Preview lists the passes.
- **Done when:**
  - walker tests pass for each source;
  - Try again at pass 2 of 3 keeps pass 1;
  - checked by hand: every FBX in a folder opens in 3D Review, and two ticked platforms build.

**4. Arranging the window.**
- **Scope:**
  - side by side up to 3, sections, pages, text, line;
  - every control kind and its settings, Advanced included;
  - several buttons, each with its steps;
  - lists that re-run when their settings change.
- **Done when:**
  - the window edit tests pass;
  - the design view's width equals the real window's;
  - a list re-runs only when its settings change;
  - auto-fit is checked by hand on both OSes;
  - the second usability test is done.

**5. Diagram in the run view, and progress.**
- **Scope:** the read-only diagram with statuses; `OSC 9;4` feeding the row and the taskbar.
- **Done when:**
  - the status mapping tests pass;
  - the parser tests pass (BEL and ST endings, split reads);
  - checked by hand with `cargo build`.

**6. More add-ons, carrying add-ons, ports, v1 dropped.**
- **Add-ons:**
  - `unity`: build the player, run a method, find the project's Unity;
  - more `git`: commit, push, current branch;
  - more `dotnet`: restore, publish;
  - `core.find-app`;
  - `bite` and `3d-review`, following the contract.
- **Carrying add-ons:**
  - "Include add-ons in this tool";
  - "Update the copy in this tool";
  - "Save to my Add-ons folder";
  - the tip shown on save.
- **Ports:**
  - port `unity-build`;
  - move e2e and the runner tests to format 2.
- **v1:** delete the v1 reader and add the old-file message.
- **Also:**
  - trust for downloaded add-ons;
  - `requires=` worked out from `since`;
  - examples for .NET, Unity, Bite and 3D Review;
  - docs.
- **Done when:**
  - the examples are in canonical form;
  - every built-in step has an `example` golden;
  - a tool carrying an add-on runs on a machine without that add-on (e2e);
  - a v1 file gives the old-file message;
  - the downloaded-add-on trust message is tested on Windows;
  - the docs are current.

## Companion work in the other repos (to the contract)

**Bite:**
- exit 1 when any image fails;
- `--report`;
- reject unknown or value-less flags;
- `--help` for each command;
- `OSC 9;4` progress;
- later: `bite describe <wf> --json` and `--set <node>.<param>=<value>`.

**3D Review:** a `review-cli` console binary. **Built** (3D Review's `crates/cli`, reference
`docs/cli.md`), to the contract:
- `optimize` (`--preset`, `--lods`, `--reduce`, `--packaging`, `--hierarchy`, `--format`, `--out`,
  `--suffix`, `--if-exists fail|skip|replace`), `audit` (`--profile`, `--fail-on`) and `info`, each
  over files and folders with `--recurse`, `--report` and `--progress-bar auto|on|off`.
- Exit 2 before anything runs, 0 only when everything worked, 1 otherwise; an empty value is the
  option left out and a list is one comma-separated value.
- The report: flat `ok`, `problem`, `models`, `done`, `skipped`, `failed` plus each command's
  totals, and `results` with an entry per model.
- Shipped beside the viewer on both OSes; Setup's opt-in "Add to PATH" task.

## Docs and decisions

**Docs:**
- `docs/step-reference.md`, generated from the built-in add-ons (`examples/gen-docs.rs`,
  `xtask/src/docs.rs`, `reference_docs.rs`).
- `config-reference.md` becomes `tool-reference.md`.
- `docs/author-guide.md` becomes "Making a tool", written for artists, with screenshots.
- New `docs/add-on-guide.md`:
  - the format, text only;
  - settings, lists, needs;
  - steps vs scripts;
  - search words and what comes next;
  - versions and carried copies;
  - the writing rules, trust, and the CLI contract.

**`docs/DECISIONS.md` changes:**
- N3: tools may use installed add-ons, or carry copies of them.
- N4: a window plus steps from add-ons; point-and-click rules; Script under Advanced.
- N9: covers installed add-ons too.
- N10: the `format` gate and `uses`.
- N11: Try again resumes at the step that went wrong, by key.
- N12: superseded.
- N15: two libraries, the tool format's catalog and the add-ons; no product words in the engine.
- N17: `args` becomes `core.args`.
- N18: `settings`, one Lua environment per add-on, results returned from Luau.

**New decisions:**

| # | Decision |
|---|---|
| N19 | One plain vocabulary, enforced by `plain_words.rs` |
| N20 | Steps diagram: a tree, auto layout, no positions, no zoom, one step at a time |
| N21 | Add-ons: text only, one block; the library, where they live, name clashes, versions; built-in steps only in built-in add-ons |
| N22 | Self-contained tools: carried copies win |
| N23 | The four ways a setting gets its value; rules stored as data |
| N24 | Scopes; results kept when a step goes wrong |
| N25 | The editor owns the file |
| N26 | Editor side by side; the step picker panel; Advanced always available but collapsed |
| N27 | The window holds controls (buttons included), in rows of up to 3 |
| N28 | Needs are checked live and first |
| N29 | v1 files are dropped |
| N30 | Repeat for each |
| N31 | Run view: list or diagram |
| N32 | The CLI contract |
| N33 | Safe defaults and plain problems |

Rewrite `docs/PLAN.md`'s milestones and CLAUDE.md's state section as each slice lands.

## Verification

**Every slice:**
- `cargo xtask check`: fmt, clippy, tests, the TODO ban, the macOS type-check, a release build;
- `cargo xtask e2e`;
- `cargo xtask ttfp` at 150 ms or less;
- `cargo xtask idle` about 0, with the editor open too.

**Goldens and tests:**
- problems, for tools and add-ons;
- the print round trip;
- `tool_edit` sessions;
- diagram geometry;
- step examples;
- add-on surfaces;
- `plain_words.rs`;
- the picker test.

**Usability test** (after 1b, and again after 4):
- **Who and how:** three artists who haven't seen Launcher, each on their own machine and
  project. No help; they think aloud.
- **Tasks:**
  1. Make a tool that gets the latest changes for this project.
  2. Make it ask which branch to switch to first.
  3. After slice 2: only run the tests when "Run tests" is ticked.
  4. After slice 3: open every FBX in a folder in 3D Review.
  5. A general tool with no project: copy a folder to a shared drive.
- **Note:**
  - who finished without help, and how long it took;
  - wrong turns;
  - every word they ask about or misread. Each one becomes a wording fix.
- **The bar:** all three finish tasks 1 and 2 without help within 5 minutes.

**By hand:** Windows and macOS, every slice.

## Risks

| Risk | Mitigation |
|---|---|
| Jargon creeps back in | `plain_words.rs`, a wording review in each slice, and the usability tests. |
| Three columns plus the picker feel cramped on a laptop | The editor needs at least 1280×820. The picker covers the window column only while open. The window column can fold to a preview strip. |
| Carried add-on copies go stale | The editor offers "Update the copy in this tool" when a newer one is installed. |
| The engine rewrite breaks existing behaviour | The temporary v1 reader keeps today's tests green until slice 6. |
| The add-on format freezes too early | Add-ons from the Add-ons folder are marked experimental until slice 6. |
| Lists are slow, or start apps while a tool opens | Time limits, spinners, and re-running only when their settings change. |
| Comments in hand-edited files are lost | Comments above items are kept as notes, and the "tidy" message warns first. |
| The design still doesn't land with artists | Keep 1b thin and re-plan after the first test. |
