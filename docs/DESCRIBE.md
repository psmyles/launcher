# Launcher: make a tool by describing it (study)

> **Status (2026-10-14): a study.** Nothing is built and nothing is decided. It asks whether a
> small model, running on the person's own computer, can turn a description into a tool, and how
> that would fit the redesign ([`REDESIGN.md`](REDESIGN.md), N19–N33). Speeds and sizes here are
> estimates; stage 0 measures them.

## The idea

- In Make a new tool, someone writes what they want in their own words: "get the branches, let me
  pick one, switch to it and get the latest".
- Launcher asks a question or two if it has to, shows two or three versions to choose from, and
  opens the chosen one in the editor, ready to try and save.
- A small model on the computer does the understanding. Nothing leaves the machine.

## Why it's worth trying

- The first try with a colleague showed that the hard part is turning an idea into steps, controls
  and results passed between them. A description skips that part.
- The result lands in the editor, where people see their idea as cards and controls. That teaches
  the editor's ideas better than an empty tool does.
- Requests that nothing can do yet show which add-ons to write next.

## Short answer

Feasible, if the model's job stays narrow: it **chooses and fills in** from the add-ons, and the
code we already have builds the tool, checks it and shows it. A small model writing KDL and Luau
freely would be unreliable. A small model that can only pick from what the add-ons offer is
realistic.

The main cost isn't the model. It's the training data, the evaluation, and keeping both current as
add-ons change.

## Why format 2 makes it workable

- **Tools are tiny.** The starters in `examples/starters/` are 14–24 lines, about 150–250 tokens.
- **The vocabulary is closed, and written in plain words.** Today: 42 steps in 7 built-in add-ons,
  each with `about`, `search-words`, `comes-next`, settings with kinds, labels and help, and
  `needs`. That's the description a model needs, already written for people who aren't developers.
  The relevant lines of every add-on come to about 19 KB (about 5,000 tokens); the steps that
  matter for one request are a few hundred.
- **Every answer can be checked.** `config/format2.rs`, `launcher check`, `--preview` and mock dry
  runs. A tool that fails gets one more try, with the problem in the prompt. The same checks filter
  the training data.
- **The editor owns the file (N25).** The model doesn't have to write KDL. It makes the decisions a
  person makes in the editor, and `config/tool_edit.rs` carries them out: `insert_step`,
  `use_new_control` ("Let people choose"), `set_put` ("Put it into"), `add_control`. Then
  `config::print` writes the file. `use_new_control` works out the control from the setting (its
  kind, label, help, options and starting value), so the model chooses none of that.
- **"Let people choose" is always a safe answer.** A value the model doesn't know becomes a control
  in the window. The tool still works, and the person fills it in when they use it. So questions
  can always be skipped.

## What's hard

**Launcher's way of doing things.** The first try with a colleague is the example. "Get the
branches, let me choose one, switch, get the latest" isn't four steps. It's List branches putting
its result into a Drop Down Button, behind a button of its own, then Switch branch and Pull behind a
second button (`switch-and-update.launcher`). The same goes for checks under "Before starting"
versus steps, and for "Always use" versus "Let people choose". A large model can reason its way
there. A small model learns it only from examples.

**Saying no.** No step zips a folder, submits to Perforce or posts to Slack. A model that can only
pick real steps won't make one up, but it will pick the nearest wrong one, or reach for `core.run`
with made-up flags. It needs a way to say "nothing does this yet", and examples that teach it when.

**Making up values.** Paths, branch names, program arguments: small models fill gaps with plausible
text. The fixed-value rule (below) deals with this in code.

## How it would work

### Where it shows

Make a new tool (`crates/launcher/src/ui/home.rs`) gets a box at the top: "What should the tool
do?". The project folder stays where it is, and stays optional. Then, on the same screen: questions
if there are any, and the suggestions as cards. Then the save dialog and the editor, as today.
Without the model installed, the box works from word matching (stage 1).

### The flow

```
description + project folder (what Make a new tool found there: Git, .NET, …)
 → shortlist: about 10–15 steps and the 2 closest starters, matched by words
   (as the step picker's search does)
 → the model writes a plan; a grammar built from the shortlist allows only real steps and
   their real settings
 → the fixed-value rule; questions for what's unknown
 → 2–3 suggestions made from the plan
 → tool_edit builds the tool, config::print writes it, launcher check checks it
   (one more try with the problem if it fails)
 → the save dialog, then the editor
```

### The plan

The plan holds the editor's decisions, not the file. The colleague's request:

```json
{
  "name": "Switch branch and get the latest",
  "buttons": [
    {
      "label": "Get the list of branches",
      "steps": [
        { "step": "git.fetch" },
        { "step": "git.list-branches", "put": { "branches": "branch" } }
      ]
    },
    {
      "label": "Switch and get the latest",
      "checks": [{ "step": "git.no-unsaved-changes" }],
      "steps": [
        { "step": "git.switch-branch", "settings": { "branch": { "choose": "branch" } } },
        { "step": "git.pull" }
      ]
    }
  ]
}
```

- **A setting is one of:**
  - `{ "value": … }`: Always use;
  - `{ "choose": "<control>" }`: Let people choose (the same name reuses one control);
  - `{ "from": "<step>.<result>" }`: what an earlier step found;
  - `{ "ask": true }`: unknown, so a question.
- **A request for something no step does** adds `"missing": ["zip the build folder"]`. The rest is
  still built, and the person is told what couldn't be.
- **Needs** (N28) come from the steps; the model never writes them.
- **Each plan item is one `tool_edit` operation**, so the plan changes only when the editor gains an
  operation. A test builds every starter from a plan and checks that the result matches it (the
  same window, steps and settings), which keeps the plan and the format in step.
- **JSON**, because models and grammar tools handle it best. A shorter line format is the fallback
  if stage 0 finds JSON too slow.

### The fixed-value rule

Every fixed value must come from the person or the project:
- their description or their answers;
- a setting's options;
- the project's real values (its branches, as "Always use" lists them).

A value that can't be traced to one of these becomes a question, with the model's guess filled in
to confirm. This stops made-up paths and flags, and it's checked in code, not left to the model.

### Questions

- Only for settings the plan marks unknown, and at most three. The rest become "Let people choose".
- Built by the app from the setting's label, help and kind, not written by the model: a folder
  picker for a folder, the options for a choice, the project's real values when the setting has a
  list. Those words already pass the plain-words check.
- Every question offers "Let people choose when they use the tool".

### Suggestions

Asking a small model the same thing three times gives the same plan three times, or three worse
ones. So the suggestions are versions of one plan, along real choices:
- which settings are fixed and which people choose ("Always copy to S:\Builds" or "Let people
  choose the folder");
- one button or two;
- a step from the add-on's "What comes next" (`comes-next`: Copy, then Show in folder), or a check
  (No unsaved changes before Pull).

Each card shows the window and the steps' sentences (`says`), with one line on how it differs.
When a request could mean either of two steps, the model can return two plans, and both become
suggestions.

### What it never does (first version)

- Write `core.script` or any Luau. `core.run` (an advanced step) only for a program the person
  named, with only the arguments they gave (the fixed-value rule).
- Change advanced settings. They keep their defaults, which keeps the safe defaults (N33): Delete
  goes to the Recycle Bin or Trash, Copy asks before replacing.
- Use Only when, If or Repeat for each, until slices 2 and 3 settle them.
- Write add-ons, run the tool, or save without the save dialog.

## The model

### Size and speed

A 1–4B instruct model, quantized to 4 bits: 0.7–2.5 GB on disk, and 1–3 GB of memory while loaded.
One request is a prompt of about 1,000 tokens (instructions, the shortlist, two starters, the
description) and a plan of about 150 tokens. The instructions are the same every time, so the
helper reads them once and keeps them. Estimates, to be measured in stage 0:

| Computer | Model | Reading the prompt (~1,000 tokens) | Writing the plan (~150 tokens) |
|---|---|---|---|
| Mac, Apple Silicon (Metal) | 3B | 1–3 s | 2–5 s |
| Windows laptop, CPU only | 1.5B | 3–8 s | 5–8 s |
| Windows laptop, CPU only | 3B | 6–15 s | 8–15 s |
| PC with a recent NVIDIA card (Vulkan) | 3B | under 1 s | 1–2 s |

- Reading the prompt is the slow part on a CPU. That's why the prompt holds a shortlist, not every
  add-on.
- A failed check costs one more run.
- Making a tool is a one-off, so a few seconds with progress shown is fine; 30 seconds on an office
  laptop isn't.

**Expectation:** fine-tuned and held to the grammar, a ~3B model handles common requests well (1–4
steps, a few controls, one result put into a control). A ~1B model is plausible only because the
plan is so constrained. Neither will match a large model on unusual requests or on saying no.

### Which model

Chosen by the evaluation, at the time. As of mid-2026 the candidates in this class include Qwen3
(1.7B, 4B), Gemma 3 (1B, 4B), Llama 3.2 (1B, 3B), Phi-4-mini and SmolLM3. Prefer weights under
Apache-2.0 or MIT (Qwen3, Phi-4-mini, SmolLM3), so their licence ships the way the fonts' licences
do.

### Running it

- **llama.cpp:** Metal on macOS; CPU and Vulkan on Windows (CUDA's libraries are too big to ship);
  output held to a grammar (GBNF) built for each request from the shortlist, which keeps it small
  and fast; standard GGUF model files; LoRA adapters loaded at run time.
- **In a helper program, not in the app.**
  - It starts when someone describes a tool, and quits after it's been idle for a while.
  - The app's first frame, idle CPU and memory don't change (N8, N15).
  - A crash in the C++ code can't take the editor down with it (`catch_unwind` only catches Rust
    panics).
  - The app talks to it over its stdin and stdout.
- **The helper ships in the installers**, signed and notarized with the app on macOS (N13). The
  weights are an optional download into `<app data>/models/`, checked against a hash. They're data,
  not code, so there's nothing to sign.
- **Updates are adapters.** The base weights rarely change; a retrained LoRA adapter is tens of MB.
- **The other-OS type-check** (`cargo xtask check`): llama.cpp is a large C++ build. Its build
  script skips the C++ step when cross-checking, as sokol's does, or the helper's crate stays out
  of the cross-check.

### Models in the OS

macOS 26 has an on-device model (about 3B) whose output can be held to a given shape, with nothing
to download. But it needs Apple Silicon with Apple Intelligence turned on, its context is small
(about 4,000 tokens), and adapters must be retrained for each version of the OS model. Windows has
nothing like it outside Copilot+ PCs. A possible extra on Macs later, not the plan.

## Training data and evaluation

### Making the data

There are no real descriptions yet, so a large model writes them, and our checks keep only the good
ones.

1. **Seeds:** the starters, plus a hand-written tool or two for each step.
2. **Plans:** the large model puts together plausible tools from the add-ons, for the people who'd
   use them (artists, build engineers, QA) and the projects they'd have (Git and Unity, .NET, plain
   folders).
3. **Descriptions:** for each plan, several ways people would ask for it: short ("pull then build
   release"), chatty, vague ("update my project"), with typos, or with the values in it ("copy
   Builds to the S drive"). Describing a tool is easier for a model than building one, so going in
   this direction gives cleaner pairs.
4. **Hard cases:** vague requests with the right unknowns; impossible requests with the right
   "missing"; half-possible ones with both.
5. **Checks:** each plan is built, and must pass `launcher check` and a mock dry run. Its fixed
   values must follow the fixed-value rule.
6. **Renamed add-ons:** the same examples with add-ons and steps renamed, reordered and invented (a
   "Perforce", a "Blender"), so the model learns to read the shortlist instead of remembering it.
   Teams install their own add-ons, and tools carry theirs (N21, N22).

A few thousand checked examples are enough for a LoRA fine-tune of a 1–4B model: hours on one GPU,
or on a Mac with enough memory (mlx-lm), then converted to GGUF.

### The evaluation set

About 200 requests written and checked by hand and kept out of training, including the usability
tasks in `REDESIGN.md` (task 2 must give `switch-and-update.launcher`). Scored automatically:
- the shortlist holds the right steps;
- the tool builds and passes `launcher check` without the second try;
- the right steps, in the right order;
- the right "Always use" or "Let people choose" for each setting, and the right "Put it into";
- the right unknowns, and the right "missing" (no plan for a request nothing can do);
- the time taken and the peak memory on each kind of computer.

Also 30–50 requests that a colleague judges: "would you keep this tool?". Real descriptions from
the colleague tests (with their permission) go into the set first.

### Keeping it current

- **Add-ons change:** the shortlist and the grammar come from the add-ons loaded at the time, so
  nothing is retrained. The evaluation runs again. It needs the weights, so it's an `xtask` command
  of its own, not part of `cargo xtask check`.
- **The plan changes** (Only when, If, Repeat for each): a new adapter.
- **The starters' plan test** runs in `cargo xtask check`, without a model.

## Fit with Launcher's rules

| Rule | How this fits |
|---|---|
| First frame in 150 ms; an idle window draws nothing (N8, N15) | The model lives in a helper started on use. Until then the app is unchanged. |
| Nothing product-specific in `crates/*/src` (N15) | The prompt is generic: every product word reaches the model through the add-ons. Training data and the evaluation set live outside `src`. |
| Plain words (N19) | Questions are built from setting labels and help. The names the model writes (the tool, its buttons) are checked against `NEVER_WORDS` before they're shown, and people can rename them. |
| The editor owns the file (N25) | The plan goes through `tool_edit` and `config::print`; the model never writes KDL. |
| Trust (N9) | A tool made here has no download mark, so no trust prompt stops it. It opens in the editor instead and never runs by itself; its cards, "Before starting" and Preview show what it does. |
| Safe defaults (N33) | Advanced settings are left out of the plan, so their defaults stay. |
| Signed macOS app (N13) | The helper is signed and notarized with the app; the weights are data. |
| `cargo xtask check` | The C++ build is skipped when cross-checking; the starters' plan test runs; the evaluation doesn't. |

## Risks

| Risk | Mitigation |
|---|---|
| It picks the wrong steps and looks sure of them | It always opens in the editor; each suggestion says how it differs; the evaluation set. |
| It makes up values | The fixed-value rule, in code. |
| People run what it made without reading it | It never runs by itself; the editor shows the steps in plain words; the safe defaults stay on. |
| Too slow on office laptops | Measured in stage 0; a smaller model; a short prompt; progress shown. |
| Add-ons change under it | The shortlist and grammar come from the loaded add-ons; it's trained on renamed ones; the evaluation runs again. |
| The plan falls behind the format | The plan is the editor's operations; the starters' plan test. |
| Made-up requests don't sound like real people | Real descriptions from the colleague tests go into the evaluation first. |
| The download is too big, or not allowed | It's optional; word matching (stage 1) works without it; Make a new tool works as today. |
| A second big system for a small team | Stages with stop points; the model can be swapped; the flow and checks are useful without it. |

## Stages

Each stage ends with a decision to go on or stop.

**0. Measure (a spike, like M0).**
- 50 hand-written requests, the usability tasks among them. The plan format, the shortlist and a
  prompt.
- 2–3 small models as they come, and one large model, on those requests.
- Speed and memory on a Windows laptop (CPU only), a machine with a discrete GPU and a Mac.
- **Done when:** the results are written up like `docs/m0/RESULTS.md`. If the large model can't
  make good plans, stop: the plan or the add-ons need work first, and a small model won't do better.

**1. Describe it, without a model.**
- The box in Make a new tool. The description is matched against the starters' `about` and the
  steps' names and search words. The best starter, or a new tool with the matched steps, opens in
  the editor.
- **Done when:** it has a score on the evaluation set. That's the bar the model has to beat, and it
  stays as the fallback when the weights aren't installed.

**2. The whole flow, with a large model behind it (development builds only).**
- The plan, the fixed-value rule, questions, suggestions, building and checking, the second try. A
  `launcher describe` command for scripted runs of the evaluation.
- The large model (hosted, or a large open model on a workstation) only stands in while the flow is
  built. It's never part of the product.
- **Done when:** it passes the evaluation set, and colleagues finish the usability tasks with it.

**3. The small model.**
- The training data, 2–3 fine-tuned candidates, the helper, the grammar, the download.
- **Done when:** the chosen model is close to the large model on the evaluation, at least as good at
  saying no, and fast enough on the slowest computer, and colleagues finish the usability tasks with
  it.

**4. Later.**
- Only when, If and Repeat for each, once slices 2 and 3 land.
- Describing a change to an open tool ("also copy the build to the share"). The plan is the editor's
  operations, so a change is a plan applied to the tool.
- "Nothing does this yet" answers (with permission) show which add-ons to write next.

## Open questions

- JSON or a shorter line format for the plan (stage 0, by speed).
- Which model, and what size (stage 0, then stage 3).
- On Windows, CPU only or Vulkan as well? It depends on the graphics drivers on the team's
  computers.
- Where the weights download from, and how updates reach people; there's no updater (N13).
- Does the model name the tool and its buttons, or does the app take the names from the steps'
  `says`? This study assumes the model, checked against `NEVER_WORDS`.
- Can "nothing does this yet" answers be collected, and where would they go?
