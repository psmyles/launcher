# Launcher rewrite: plan

## Context

On `master`, `launcher` is a Windows-only portable exe that reads a fixed `launcher.kdl` beside
itself (Rust, winit 0.30, Dear ImGui, sokol D3D11). It is being rewritten from scratch as a
cross-platform (Windows + macOS) **GUI front end for scripted workflows**. A double-clickable
`.launcher` file defines a form, a list of steps and a live console, so team members can run
complicated build and launch procedures without the command line. Updating a tool means pushing a
new version of its `.launcher` file to version control.

Driving use cases:
1. **Git build tool:** pick a branch, update, build Debug or Release, optionally run tests, watch the console.
2. **Game launcher:** launch a game build with settings picked from lists, as master's launcher does.
3. **Unity build tool:** update a branch, run Unity in batch mode, copy the build to a folder.

The design was settled in a brainstorm; the results are under **Decisions**. Development continues
on a new **Windows** machine, in this repo, on a branch `rewrite` that replaces the old code. This
file must stand on its own there. The old code stays on `master` as reference (`git show
master:<path>`; see **Reference material on master**).

**First task: M0, a renderer spike.** egui is drawn two ways: by wgpu, tuned, and by a custom
sokol_gfx painter. Both are measured by time to first frame (TTFP) to choose the renderer. The
budget is a **150 ms median** on Windows and macOS.

## Decisions

**[`docs/DECISIONS.md`](DECISIONS.md) is now the authoritative list** (N16, the renderer, is only
there). The table below is the original, kept so this plan still reads on its own.

| # | Decision | Why |
|---|---|---|
| N1 | Product "Launcher", installed once per user (`launcher.exe` / `Launcher.app`). Configs are documents with the extension **`.launcher`**; double-click opens them. | Several tools per folder, one runtime to update. `.launch` collides with Eclipse/ROS; `.lnch` reads as "lunch". |
| N2 | Windows and macOS, one codebase. A config may target one OS: `os="windows"` or `os="macos"` in its header. | Separate per-OS files are acceptable and simpler than one file for both. |
| N3 | The config is a single KDL v2 text file: no zip, no sidecar files. Relative paths resolve against the config's folder, never the working directory. | It diffs and reviews in git; a shortcut's "Start in" can't change behaviour. |
| N4 | A declarative skeleton (window, form, layout, `data`, actions made of steps) plus embedded **Luau** (mlua, sandboxed) for logic: `script` blocks, `lua` steps, `(lua)"expr"` property values, and Lua expressions in `if=`, `enabled-if` and `visible-if`. `{name}` and `{env.X}` stay plain substitution. | Loops, parsing and error handling without growing a homemade expression language; the editor and validation keep working on the skeleton. Evaluation still always ends (time budgets). |
| N5 | No embedded shell scripts. `run` starts programs directly (an argument list, no shell; plus a raw `command-line=` on Windows). Repo scripts run through `run`. An `env-from` step captures environment variables from setup scripts (vcvarsall, conda, etc.). | No generated script files (AV-friendly for an unsigned exe), and user input can't inject commands. |
| N6 | Lua gets only the engine's API: no `io`, `os.execute` or `io.popen`. Load-time Lua has a time budget. Lua never runs on the UI thread. Lua errors report `.launcher` file lines. | Every side effect goes through the console, Cancel, logging and dry-run. |
| N7 | Console: every displayed process runs in a pseudo-terminal, which we spawn ourselves: ConPTY via `CreateProcessW` on Windows (raw command lines; Job Object assigned while suspended), openpty on macOS. Output goes through alacritty_terminal and is drawn by an egui view forked from egui_term. Users can type into it. `capture()` uses pipes. Windows GUI-subsystem programs use pipes. | Tools show progress and colours only on a terminal. egui_term spawns its own PTY, and portable-pty can't pass a raw command line. GUI-subsystem exes (Unity.exe) don't attach to ConPTY. |
| N8 | GUI: egui + egui-winit, no web tech. The renderer is chosen by M0: tuned egui-wgpu or a custom sokol painter. Budget: median TTFP ≤ 150 ms on Windows and macOS. There is no exe-size gate (installed runtime); size is recorded. | Immediate mode suits config-built forms and the visual editor. |
| N9 | Trust: a file carrying the Windows "downloaded" mark (MOTW, `Zone.Identifier` ZoneId ≥ 3) or macOS `com.apple.quarantine` needs a trust prompt before any load-time evaluation. Files from git carry neither mark. | A `.launcher` file runs programs. |
| N10 | `requires="x.y"` gates on the runtime version. The last working copy of each config is cached and used, with a banner, when an update breaks it. Authors validate with `launcher check` and the editor; there is no CI. | Updates arrive by pull, and installers can lag behind. |
| N11 | Runs: one at a time per `.launcher` file, via an OS file lock in app data keyed by the file's resolved full path. An optional `lock="<path>"` on an action guards shared resources across files. The config is snapshotted when a run starts, and reloads wait until it ends. Retry resumes from the failed top-level step. | Different files in one folder may run at once. Git branch switches can rewrite the running config. |
| N12 | Visual editor in v1. Luau highlighting via full_moon. Validation via parse errors, selene-lib lint with definitions generated from the Lua API registry, and cross-checks against the config. Edits are text splices at parser spans; the document is never re-printed. | Comments and formatting survive. kdl 6.7's re-serializer drops comments (LESSONS L1.1). |
| N13 | Distribution: an unsigned per-user Windows installer shared internally; a macOS DMG signed with Developer ID, notarized and stapled. Updates mean running a new installer. | Internal tool first. The stable signature keeps macOS privacy approvals across updates. |
| N14 | macOS: the login-shell environment (PATH from `.zshrc`) is captured at startup. A document opened while the app is running starts a new process. | Finder-launched apps lack the shell's PATH. LaunchServices sends a second document to the running process. |
| N15 | Carried over from master (`docs/legacy/DECISIONS.md`): | Proven in the old project. |
|  | ● Every event handler runs under `catch_unwind`, so keep `panic = "unwind"`. An idle window draws nothing. |  |
|  | ● Nothing product-specific in the engine. |  |
|  | ● Auto layout only; the window fits its content unless given a size. |  |
|  | ● `after=stay\|exit\|minimize`. |  |
|  | ● One state file per config, named with FNV-1a; `persist=#false` opts out. |  |
|  | ● Headless CLI runs never write state. |  |
|  | ● Step warnings are modal and pause the run. |  |
|  | ● `{env.X}` syntax, with a lint for `%X%`. |  |
|  | ● A preset value that matches no option is an error. |  |
|  | ● Our own small logging module. |  |
|  | ● One registry (the catalog) feeds the loader, editor, docs and CLI help; generated docs have a test that keeps them current. |  |
|  | ● No `TODO`/`FIXME` comments. Console output never panics. rfd dialogs run on their own thread. |  |

**Out of scope for v1:** secrets (tools such as git and Unity are assumed to be configured already),
admin elevation, CI for configs, zip packages, fetching configs from a URL, Windows code signing.

## Step 0: hand-off (on this Mac, right after approval)

1. Create branch `rewrite` from `master`.
2. Save this plan as `docs/PLAN.md`.
3. Replace `CLAUDE.md` with a short rewrite version. It points to `docs/PLAN.md`, holds the build commands (to be filled in as they appear), and says the old D1–D27 decisions are superseded except where N15 carries them over. Otherwise Claude Code on the Windows machine follows the old "do not re-litigate" rules.
4. Move `docs/DECISIONS.md` to `docs/legacy/DECISIONS.md`. Keep `LESSONS.md`.
5. Root `Cargo.toml` gets `exclude = ["spike"]`. `.gitignore` gets `spike/target/`; the root `/target` rule doesn't cover it. `spike/Cargo.lock` will be committed.
6. Commit on `rewrite`. Push once the owner confirms; the Windows machine pulls it.

## Step 1: Windows machine setup

- **Git**, with `git config --global core.longpaths true`.
- **Rust** via rustup, `x86_64-pc-windows-msvc`. Pin an exact version in `rust-toolchain.toml` that meets the MSRV of the egui release pinned in M0.
- **Visual Studio 2022 Build Tools**, "Desktop development with C++" (MSVC + Windows 11 SDK), for sokol (C) and Luau (C++, built by mlua).
- **For variant A2 only:** DXC, from the Windows SDK `bin\<ver>\x64\` or the DXC GitHub release.
- **PowerShell 7** (LESSONS L4.2), VS Code with rust-analyzer and a KDL extension. **Inno Setup 6** later (M10).
- **Timing hygiene:**
  - plugged in, power mode "Best performance";
  - close VS Code and other heavy apps;
  - run the bench from a plain shell, not a VS Developer prompt, which puts `dxcompiler.dll` on PATH;
  - time binaries **copied to a normal NTFS folder** (e.g. `%LOCALAPPDATA%\Programs\spike\`) with Defender on, not from a Dev Drive or `target\`.
- **Then:** clone, `git switch rewrite`, `cd spike`, `cargo build --release`.

**macOS** (the second half of M0): full Xcode, for `xcrun metal`/`metallib`. Codesigning and
notarization setup waits until M10.

## M0: renderer spike (first; timebox 1 week)

> **Status (2026-09-29):** both halves are measured. See [`m0/RESULTS.md`](m0/RESULTS.md).
> - **macOS:** B measured 136 ms. A1 measured 201 ms, because wgpu-hal skips drawing while the window is occluded, which includes hidden and just-shown windows. With that check patched out, A1 measured 134 ms.
> - **Windows:** B-warp measured 43 ms (master's launcher: 51 ms). A1 measured 276 ms on WARP and 296 ms on hardware; wgpu's adapter enumeration loads every GPU driver.
>   - With enumeration patched (A1-warp-f), A1's frame is submitted at 67 ms but not drawn until ~158 ms: egui-wgpu's draws are ~13× slower than B's on D3D12 WARP.
>   - Streaming, A1 on WARP runs at 9 fps using ~17 cores.
>
> **Decided (N16 in [`DECISIONS.md`](DECISIONS.md)):** the sokol painter, with WARP for the first
> frame and hardware afterwards. Cold-cache runs and a machine with a discrete GPU are still open.
>
> **M1 status (2026-09-29): done on Windows.** `cargo xtask ttfp`: 54.7 ms median (p90 63 ms);
> `cargo xtask idle`: 15.6 ms of CPU in 10 s; the console prototype runs `git clone` with live
> progress and takes typed input; the WARP → hardware switch works. macOS paths type-check only.
>
> **M2 status (2026-09-29): done.** The catalog drives loading, validation, `launcher check`,
> `--help-config` and `docs/config-reference.md`; golden diagnostics; the last-working-copy cache
> (tested); the folder watcher (reload deferral during runs lands with runs, M6). The example
> `git-build` (Windows, macOS) passes `launcher check`. The `args` builder stays declarative
> (N17).
>
> **M3 status (2026-09-29): done.** The Lua API registry (N18) with a bindings test and
> generated `docs/lua-api.md`; sandbox, budget, cancel and file-line error mapping are tested;
> load-time evaluation runs on its own thread; the trust gate is a type-level token, tested with
> a real Mark of the Web.
>
> **M4 status (2026-09-29): done.** `cargo xtask e2e` passes on Windows: exact arguments in a
> pty and through pipes, the raw command line, environment, cwd and stdin; a timeout kills
> grandchildren; the per-file lock; dry runs; env-from; exit codes. Retry, Cancel, on-error,
> checks-first, Lua in runs and HTTP extracts are covered by the runner's unit tests.
>
> **M5 status (2026-09-29): done on Windows.** In a real window (`--console`): `git clone`
> progress lines overwrite themselves; cargo colours (16, 256 and true colour, inverse,
> underline); Unity 6000.0 `-batchmode -logFile -` is detected as a GUI program and read through
> pipes, each line at the left; a `python input()` prompt takes typed text; Ctrl+C copies a
> selection or interrupts (`ping -t`); wheel scrollback with a "more output below" hint; drag
> selection and copy. 10 MB of coloured output: through pipes the program finishes in 52 ms,
> through ConPTY in 4.8 s (ConPTY's own pace); the window answered every probe within 14 ms. The
> emulator parses 10 MB in about 0.1 s (release). Idle after output: 0 ms of CPU in 10 s. One
> console per step section is part of the run view (M6). macOS: type-checked only.
>
> **M6 status (2026-09-29): done on Windows.** By hand and headless: a game launcher (with a
> stand-in game: a role presets the server, the steps that need a session skip, the game gets the
> full command line, the window closes, `after="exit"`) and `git-build-windows` (a scratch .NET
> repo with a local origin: fetch, switch, pull, build, test). `git-build-macos` shows its OS gate on Windows; it
> needs a Mac to run. A demo file covered every control kind, validation, on-change,
> `visible-if`/`enabled-if`, `confirm=`, `ask`/`confirm`/warning dialogs, a failing step, Retry
> (earlier steps keep their results), the dry run (a button's right-click menu) and closing during
> a run. The result buttons are Show log (the run's log folder), Retry and Back to the form. TTFP
> with the game launcher's form: 71.6 ms median; an idle form uses 0 ms of CPU in 10 s.
>
> **M7 status (2026-09-29): done on Windows.** `cargo xtask register` (HKCU, ProgID
> `Launcher.File`, the installer's too) and opening a file through ShellExecute starts the release
> build; the window's property store carries `Launcher.File.<hash>`, the relaunch command, the
> file's name and icon (read back through `SHGetPropertyStoreForWindow`), which is what a pinned
> button uses. Taskbar progress fills during a run and the button flashes when it ends in the
> background; the notification is posted (this machine has toasts switched off, so it wasn't
> seen). A file with a Mark of the Web prompts and lists its programs, scripts and addresses; a
> `requires="9.9"` file says which version it needs. Home: recent files, Open…, New from template
> (written, opened, `launcher check` clean). TTFP: home 64.3 ms, a document 73.9 ms. macOS (open
> documents in new processes, Dock badge, notifications) is type-checked only.
>
> **M8 status (2026-09-29): built; hand checks pending.** Tested: every catalog kind can be added
> (and every layout item, control and step added through the editor's outline and deleted again)
> and loads cleanly; on the examples every node duplicated and removed gives the file back
> byte for byte, and moved down and back keeps every line; a scripted session matches its golden
> file; undo merges one field's typing; save keeps a `.bak`; a change on disk is taken or asked
> about; the lint fixture's issues name the right file lines (`tests/lint.rs`), and the examples
> lint cleanly. Seen in the window: the editor with the outline, the live preview and the
> Problems panel. Not yet done by hand: selecting, Pick, editing properties, the Lua completion
> popup, the dry run and save from the window. The desktop was in use, so the test window
> couldn't take the focus, and posted mouse messages don't reach egui. TTFP after the editor
> (13.2 MB): 66.1 ms.
>
> **M9 status (2026-09-29): done on Windows.** `tests/cli.rs` runs the built binary: version and
> help, usage errors (exit 2), `check` on the examples and on a broken file (exit 1), a dry run
> with `--set`, and `--help-config` into a closed pipe (exit 0, no panic). The release
> (GUI-subsystem) exe writes to the terminal it's started from and survives `| head`. macOS: the
> CLI is `Launcher.app/Contents/MacOS/launcher` (M10's bundle); type-checked only.
>
> **M10 status (2026-09-29): Windows done; macOS written, not run.** On this machine, silently:
> install (files, `.launcher` open/edit verbs, Start Menu shortcut); double-click opens the
> installed exe, whose windows carry the relaunch identity; an upgrade (0.1.0 → 0.1.1) leaves one
> uninstall entry and every app-data file; uninstall removes everything it installed and keeps app
> data; the PATH task adds its entry and the uninstaller leaves PATH exactly as it was. Installer:
> 5.8 MB. Not a clean machine. macOS: `package-macos` is type-checked only, and needs a
> `bundleId` in `product.json` (a domain the publisher owns; it must never change), a Developer
> ID certificate and notary credentials.
>
> **Later (2026-10-05):** the icon is drawn from `assets/application-logo.png` (`cargo xtask
> icons`); both packages ship the licence notices; Setup gains wizard images, a desktop-shortcut
> task, "Open Launcher" on its last page and an OpenWithProgids entry; `package-macos` follows
> 3d-review's tested script (keychain identity, notary profile, the app notarized and stapled
> before the DMG, Gatekeeper check). `bundleId` is `com.psmyles.launcher`.
>
> **M11 status (2026-09-29): done on Windows.** `unity-build` (Windows, macOS) finds the project's
> editor through the Hub (its custom folder too) and, on a fresh Unity 6000.0.60f1 project, built a
> Windows player in 24 s and copied it out (logs through pipes). The author
> guide is `docs/author-guide.md`; `xtask check` runs `launcher check` on every example.

**Question:** does egui + egui-wgpu, tuned, reach a median TTFP ≤ 150 ms on Windows and macOS? If
not, does egui + a custom sokol_gfx painter? Choose whichever meets the budget with the least
custom code.

### Layout

`spike/` is its own Cargo workspace; the old crates stay untouched until M1.

```
spike/
  Cargo.toml     [workspace]; release profile copied from master: opt-level "z", lto "fat",
                 codegen-units 1, strip, panic "unwind"
  common/        timing origin + phase stamps + results JSON; the test scene; CLI flags; manifest
  eframe-app/    A0: stock eframe (defaults: wgpu + accesskit), untuned baseline
  wgpu-app/      A1/A2: egui-winit + egui-wgpu, own winit loop (no eframe)
  sokol-app/     B: egui-winit + custom sokol_gfx painter (D3D11 on Windows, Metal on macOS)
  bench/         launches the variants, collects results, prints tables, records machine info
  RESULTS.md     machines, numbers, phase breakdowns, decision; raw JSONL committed next to it
```

### Fairness rules: every variant shares

- **Versions:** the same egui / egui-winit / winit. At the time of writing, egui 0.36 goes with wgpu 30; verify when pinning.
- **Features:** the same egui and egui-winit features, accesskit off everywhere except A0.
  - `egui-wgpu` with `default-features = false`; its defaults pull in `wgpu/default` and `webgl`.
  - wgpu features: `dx12`, `metal`, `wgsl`, `std` only.
  - Check with `cargo tree -e features -i wgpu`.
- **Scene and fonts:** the same scene, egui's default fonts, and the same `max_texture_side` (8192) passed to `egui_winit::State::new`. It sets the atlas width, so the first upload is the same size.
- **Window:** 560×640 logical initial size, the GUI subsystem on Windows, and **master's `launcher.exe.manifest`** embedded (PerMonitorV2, UTF-8 code page), since R has it.
- **Renderer init runs on the GPU thread** in both A and B, in parallel with window creation:
  - A: `egui_wgpu::Renderer::new(.., Bgra8Unorm, ..)`;
  - B: the sokol shader and pipeline;
  - once the surface exists, check the format and rebuild if it differs.
- **Exact vertex layout:** assert `size_of::<egui::epaint::Vertex>() == 20`. epaint's `unity` feature reorders the fields, so it must stay off.

### What is measured

- **TTFP: the same definition as master**, so its numbers stay comparable. The clock runs from process creation to the moment the first frame has been presented **and** `set_visible(true)` has returned. It does not include the compositor.
  - **Windows:** port `master:crates/launcher/src/timing.rs`: `GetProcessTimes` creation time against the current FILETIME.
  - **macOS:** `proc_pidinfo(getpid(), PROC_PIDTBSDINFO)` → `pbi_start_tvsec/usec`, against `clock_gettime(CLOCK_REALTIME)`.
- **Bench-side stamps:**
  - `bench` records a precise timestamp just before it spawns each child. The kernel creation time may be rounded to the clock tick, so both origins are reported.
  - **Windows only:** an out-of-process "window shown" time via `SetWinEventHook(EVENT_OBJECT_SHOW)` for the child PID, skipping the `Winit Thread Event Target` window (LESSONS L2.11). This gives every variant, A0 and R included, one comparable end point.
- **Phase stamps** (ms since process creation):
  - `main` entered;
  - GPU instance, adapter, device and renderer init (on the GPU thread);
  - event loop built;
  - window created (hidden);
  - GPU thread joined (wait time);
  - surface or swapchain ready;
  - first egui pass (layout, tessellation, font atlas);
  - texture upload;
  - present returned;
  - window shown.
- **Output:** one JSON line per run appended to `--out <file>`. It carries the variant, adapter name, phases, peak working set and, on Windows, the loaded modules from `EnumProcessModules` at the stamp. That list shows whether the vendor GPU driver or `dxcompiler.dll` got loaded.
- **Exit:** immediately after writing, with `TerminateProcess(GetCurrentProcess(), 0)` or `_exit(0)`, which skips the driver teardown in DLL detach.
- **Exe size:** recorded per variant (release, stripped).

### Window policy (identical in every variant)

1. Create the window hidden.
2. Join the GPU thread and create the surface.
3. **Render and present the first frame synchronously inside `resumed`.** A hidden winit window never gets `RedrawRequested` (LESSONS L2.2); master's `shell.rs` does the same.
4. `set_visible(true)`, then stamp.

**macOS specifics:**
- Present **at most once** while the window is hidden; `nextDrawable` can stall for up to 1 s when earlier drawables were never displayed.
- Verify that the hidden frame appears on show with a 60 fps screen recording, using a clear colour that differs from the window background.
- If it doesn't appear, present again after showing, and stamp after that.

**A0 (eframe)** can't be instrumented this way: it stamps at the start of its second `update()`,
and on Windows the out-of-process "shown" time is its comparable number.

### Test scene and modes

- **Scene:**
  - a title;
  - about 8 form rows: combo box, radio buttons, checkbox, slider, text field, path field with a Browse button;
  - a primary button;
  - a console panel with about 40 lines of coloured monospace text.

  The content is identical in every variant.
- **`--stress`:**
  - The console appends 200 coloured lines per second into a 5,000-line scrollback, drawn with `show_rows`.
  - A timer drives the repaint at 60 Hz, for 10 s, at 900×700 logical.
  - It reports process-wide CPU per frame from `QueryProcessCycleTime` (Windows) or `proc_pid_rusage` (macOS). WARP rasterises on its own threads, and `GetProcessTimes` only advances in coarse ticks.
- **`--idle`:** after the first frame, wait 10 s. The CPU time consumed must be about 0 (no redraws while idle).

### Variants

**Windows**

| ID | Stack and tuning | Adapter |
|---|---|---|
| R | master's `launcher.exe` (ImGui + sokol D3D11), its game-launcher config, speaks its `LAUNCHER_TTFP_OUT` protocol. Calibration only; master recorded a 57 ms median. | WARP (as shipped) |
| A0 | eframe defaults | default |
| A1-warp | egui-winit + egui-wgpu: DX12 backend only; instance, adapter, device and renderer on a thread started first thing in `main` (like master's `Gpu::start`), joined after the hidden window exists; `InstanceFlags::empty()` and no `WGPU_*` env vars; default limits, no extra features; `compatible_surface: None`, compatibility checked after the surface exists; `Dx12Compiler::Fxc` set explicitly; non-sRGB BGRA8 surface, FIFO present; `dithering: false` | WARP via `force_fallback_adapter` |
| A1-warp-f | A1-warp with a `[patch]`ed wgpu-hal whose DX12 `enumerate_adapters` keeps only `DXGI_ADAPTER_FLAG_SOFTWARE` adapters. Counts as custom code to carry if chosen. | WARP only |
| A1-hw | A1-warp settings | the adapter B-hw gets (DXGI adapter 0, matched by LUID); `PowerPreference::None` |
| A1-dxc | *optional*: A1-warp with `Dx12Compiler::StaticDxc` | WARP |
| A2 | *conditional*: A1-warp(-f) + precompiled DXIL shaders | WARP |
| B-warp | egui-winit + sokol D3D11 painter; device thread and flip-model swapchain ported from master | WARP first |
| B-hw | same | hardware |

Why A1-warp-f exists: wgpu's DX12 enumeration **creates a device on every adapter** to query
capabilities (`wgpu-hal/src/dx12/adapter.rs`, `Adapter::expose`). So A1-warp still loads the
vendor driver, the very cost WARP-first avoids, and this is the likely cause of the slow wgpu start
seen before. A1-warp's module list confirms it; A1-warp-f measures what removing it gains.

**A2 runs only if** A1(-f) misses the budget, the join wait is still dominated by renderer init, and
the pinned wgpu has passthrough:
- `create_shader_module_passthrough`, behind `Features::PASSTHROUGH_SHADERS` (named `EXPERIMENTAL_PASSTHROUGH_SHADERS` in some versions), taking `dxil`/`metallib` inputs.
- Compile naga's own HLSL output for egui's `egui.wgsl` to DXIL, so the registers match wgpu's root signature.
- Carry a patched egui-wgpu for the shader creation.
- Metal passthrough still has open limitations upstream; check the tracking issue (wgpu #9052) when pinning.

**R setup:**
1. `git worktree add ..\launcher-master master`.
2. `cargo build --release -p launcher`.
3. Copy master's game-launcher `launcher.kdl` beside the exe.

bench runs R like the other variants.

**macOS**

| ID | Stack |
|---|---|
| A0 | eframe defaults |
| A1 | egui-wgpu, Metal backend only, device and renderer on a thread |
| B | sokol painter drawing through Metal into a `CAMetalLayer` on winit's NSView |

A2 (metallib passthrough) only if A1 misses the budget. Also time one run of each as an `.app`
bundle, and one run with a cold Metal shader cache: A1 compiles MSL at runtime, while B loads a
precompiled metallib.

**Flags on every variant:** `--adapter warp|hardware`, `--out <file>`, `--stress`, `--idle`,
`--no-thread` (measures what the GPU thread saves).

### sokol painter (variant B): implementation notes

- **sokol:** the path dependency `master:vendor/sokol-rust` (floooh/sokol-rust @ b22a545; keep that pin, since the resource API is in flux).
  - Patch its `build.rs` to compile only `sokol_log.c` + `sokol_gfx.c`. The stock build also compiles `sokol_app` and audio, which links AudioToolbox on macOS.
  - Backend is AUTO: D3D11 on Windows, Metal on macOS, where it compiles as Objective-C. The macOS path has **never been built**; do it on day one of the macOS half.
- **Device and swapchain:**
  - **Windows:** port `master:crates/launcher/src/gfx/{mod.rs,d3d11.rs}`:
    - `Gpu::start` thread;
    - `Device::create` (WARP first; `LAUNCHER_GPU=hardware` swaps the order; feature levels 11_1/11_0; no debug flag);
    - `fill_environment`;
    - `Swapchain` (`R8G8B8A8_UNORM`, 2 buffers, `FLIP_DISCARD`, render view created on acquire and dropped on resize, `Present(1, 0)`).
  - **macOS:** `MTLCreateSystemDefaultDevice`, and a `CAMetalLayer` attached with `raw-window-metal`. Set its device, BGRA8 format (sokol's Metal swapchain format; D3D11 uses RGBA8), `contentsScale`, `drawableSize` and `opaque`. `nextDrawable` goes into `sg::MetalSwapchain.current_drawable` each frame.
    - Wrap `sg_setup` (on the GPU thread) and each frame in an `autoreleasepool`: `build.rs` compiles without ARC, and sokol's command buffers are autoreleased.
- **Shader, with no shader compiler needed for the spike:**
  - Reuse the precompiled `_simgui_shd_vs/fs` **hlsl4** and **metal_macos** byte arrays from `master:crates/launcher/simgui/sokol_imgui.h` (between `//>#shdgen` and `//<#shdgen`, lines ~654–2055).
  - Build the `sg_shader_desc` as sokol_imgui does (lines ~2534–2590; Metal entry points are `main0`).
  - The uniform is 16 bytes: `disp_size` (pass egui **points**), then **`gamma = 1.0`** (a zero gamma draws everything white), then padding.
  - The fragment is texture × vertex colour. egui's premultiplied sRGB vertex colours, times a non-sRGB RGBA8 texture, into a UNORM target is exactly egui-wgpu's own gamma-framebuffer path.
  - Only the pipeline state differs from ImGui's: premultiplied blending (colour `ONE`/`ONE_MINUS_SRC_ALPHA`, alpha `ONE_MINUS_DST_ALPHA`/`ONE`, as in egui_glow and egui-wgpu), 32-bit indices, no culling, scissor on.
- **Buffers:**
  - one vertex and one index buffer with `usage.write_transient`;
  - each frame, write every mesh with `sg_write_buffer_transient` at computed offsets before the first bind (several writes per frame are allowed until the buffer is bound), growing the buffers when a frame outgrows them;
  - per mesh: bind with offsets, set the scissor from the clip rect (points → pixels, clamped), draw.
- **Textures:**
  - Persistent images use `usage.dynamic_update` + `sg_update_image`, which updates the whole image once per frame per image (sokol_gfx.h:~340; still supported, marked for replacement).
  - Keep a CPU RGBA8 copy per egui `TextureId`, apply each `ImageDelta` (whole, or partial at `pos`) to it, and upload dirty images once per frame, as sokol_imgui's `_simgui_update_texture` does with `sg_make_view`.
  - Recreate an image when its size changes, and free it on `textures_delta.free`.
  - Create one sampler per `TextureOptions` (filter, wrap).
- **Loop:** winit + egui-winit. Map egui's `repaint_delay` to `ControlFlow::WaitUntil` or `Wait`.

### Harness and procedure

- **Main run:** `cargo run -p bench --release -- --variants R,A0,A1-warp,A1-warp-f,A1-hw,B-warp,B-hw --n 30`.
  - Each round runs every variant in a seeded shuffled order.
  - Each launch waits for the child to exit, then idles 1 s.
  - Launches are spawned **without piped stdio** (`SW_SHOWNORMAL`).
  - Round 1 is dropped as warm-up (as master's `ttfp.ps1` does) and reported separately as "first launch".
- **Reported per variant:**
  - the upper median (as `ttfp.ps1`), p90, minimum, and a 95% order-statistic confidence interval;
  - the median phase breakdown;
  - per-round deltas against B-warp.
- **Steady-state runs:** `--stress` and `--idle`, 3 runs per variant.
- **Cold runs:** run `RAMMap64 -Et` (Sysinternals, Windows) or `sudo purge` (macOS) before each cold launch; 5 cold launches per variant.
- **Machine info:** `bench --machine-info` writes it into `RESULTS.md`:
  - CPU, RAM, GPUs with driver version and date;
  - Windows build + UBR, displays and scaling;
  - power plan and AC status, Defender state, timer resolution;
  - rustc version, git commit and crate versions.
- **Machines:**
  - the new Windows machine (with R for calibration);
  - one team machine with a discrete GPU;
  - this Mac.

### Decision rule

1. A1 (the best of warp, warp-f and hw) has a median ≤ 150 ms on Windows **and** macOS → **egui-wgpu**.
   - Prefer the unpatched variants.
   - If only A1-warp-f passes, the wgpu-hal patch is custom code to carry; weigh it against B's painter.
2. Otherwise, A2 ≤ 150 ms → egui-wgpu with passthrough shaders (a patched egui-wgpu).
3. Otherwise, B ≤ 150 ms → **the sokol painter**.
4. Otherwise, the phase breakdown decides.

The stress results set M1's **device policy**: WARP only, hardware only, or start on WARP and switch
to hardware after the first frame. For the switch, sokol already keeps CPU texture copies; egui-wgpu
doesn't, so it would need an `ImageDelta` cache or `ctx.set_fonts`. The stress results change the
renderer choice only if one renderer uses more than 1.5× the other's CPU on the same device.

**Visual check:** with dithering off, text must look pixel-identical across A and B, and correct at
150% Windows scaling and on Retina.

**Deliverable:** `spike/RESULTS.md` (now [`docs/m0/RESULTS.md`](m0/RESULTS.md)) plus the raw JSONL, and the decision recorded in
`docs/DECISIONS.md` as N16. If B wins, its painter moves into `crates/launcher/src/gfx/` in M1.

## Target architecture

```
crates/launcher-core/      no GUI; everything headless-testable
  config/                  KDL v2 load with spans; catalog.rs (THE registry of node kinds and properties);
                           model; validate; diag; edit (text splices); header (requires/os)
  lua/                     Luau VM (mlua), sandbox, api.rs (THE Lua API registry), chunk↔file line
                           mapping, evaluation budgets
  lint/                    full_moon tokens and syntax tree, selene-lib, config cross-checks
  engine/                  runner, steps, run context, effects (real / dry-run / mock), history, locks, retry
  platform/                process (spawn; Job Objects / process groups; GUI-subsystem detection; raw
                           command line), pty (ConPTY spawn / openpty), env (login shell, env-from),
                           open/reveal, keep-awake, file locks, trust marks, ping (ICMP), per-process volume (Windows)
  state.rs paths.rs logging.rs cache.rs replace_file.rs
crates/launcher/           the app: main.rs, cli.rs, shell.rs (winit loop), gfx/ (from M0),
                           ui/ (form, run view, console, home, trust prompt, modals), editor/,
                           os/ (windows.rs: AppUserModelID, taskbar; macos.rs: open-document events, Dock)
tools/argdump/             stand-in program for tests (kept from master)
xtask/                     cargo xtask check | ttfp (the bench, moved from spike/) | e2e | docs | package-*
installer/windows/         Inno Setup script
installer/macos/           Info.plist template, entitlements, DMG layout
examples/                  git-build/ and unity-build/ (a Windows and a macOS file each)
docs/                      PLAN.md, DECISIONS.md, LESSONS.md, generated config-reference.md and lua-api.md
```

**Threads:**
- **UI** (winit + egui): never blocks, never runs Lua.
- **Evaluation:** parses and validates configs and runs load-time Lua under a budget. That includes `enabled-if`/`visible-if` after each value change; the UI uses the last result. Results reach the UI via `EventLoopProxy`.
- **Run worker:** one per window, with a generation token.
- **Output readers:** one per process (PTY or pipes), feeding the terminal model and the log file.
- **File watcher:** watches the config's **folder**, debounced, with one check queued right after start (LESSONS L2.8, L2.12).
- **Startup (macOS):** a thread that captures the login-shell environment.

**App data** (`%LOCALAPPDATA%\Launcher\`, `~/Library/Application Support/Launcher/`):

| Path | Contents |
|---|---|
| `state/<stem>-<hash>.kdl` | Remembered values and window placement. The hash is FNV-1a 64 of the resolved full path, lowercased on Windows. |
| `cache/<hash>.launcher` | The last working copy of the config |
| `locks/<hash>.lock` | Run locks (OS file lock) |
| `runs/<hash>/<timestamp>/` | Run history, for files with `save-logs=#true`: `steps.json` plus per-step logs; the last 20 are kept |
| `trust.kdl` | Trusted files |
| `logs/launcher.log` | The app's own log |

## Config format (target sketch)

```kdl
launcher requires="1.0" os="windows"
window title="Build Project"

script #"""
    function branches()
        local list = {}
        for name in capture("git", "for-each-ref", "--format=%(refname:lstrip=3)", "refs/remotes/origin"):gmatch("[^\r\n]+") do
            if name ~= "HEAD" then table.insert(list, name) end
        end
        return list
    end
    """#

layout {
    dropdown "branch" label="Branch" options=(lua)"branches()" default=(lua)"capture('git', 'branch', '--show-current')"
    radio "mode" label="Mode" { option "Debug"; option "Release" }
    checkbox "tests" label="Run tests" default=#true
    button "build" label="Build" action="build"
}

action "build" label="Build" {
    check "git" "diff" "--quiet" message="You have local changes. Commit or stash them first."
    run "git" "fetch" "--prune" "origin"
    run "git" "switch" "{branch}"
    run "git" "pull" "--ff-only"
    run "dotnet" "build" "-c" "{mode}"
    run "dotnet" "test" "-c" "{mode}" if="tests"
}
```

- **Top level:** `launcher`, `window`, `script`, `value`, `data`, `layout`, `action`.
- **Controls:** dropdown, radio, checklist, checkbox, slider, number, text, file, folder, display, button. Children: `option` (with data), `preset`, `on-change`.
- **Containers:** group, tabs/tab, columns/column, row, separator, spacer, note.
- **Steps:** run, check, set, set-control, reset, env-from, copy, move, delete, mkdir, find-file, read-file, write-file (text / json), http-get (HTTPS) + extract, ping, open, reveal, message, confirm, ask, wait, log, fail, stop, lua, set-process-volume (Windows only).
- **Step and action options:** `if=` on any step; `on-error=`; `after=` and `lock=` on actions.
- **Lua API (initial):**
  - `values`, `run(...)` (PTY; raises on a non-zero exit unless `{check=false}`), `capture(...)` (pipes; returns stdout with the trailing newline trimmed), `step(name, fn)`;
  - `copy`, `move`, `delete`, `mkdir`, `exists`, `glob`, `read_file`, `write_file`, `json.encode`/`decode`, `http_get`;
  - `ask`, `confirm`, `message`, `log`, `set`, `env`, `os`, `config_dir`, `path.*`, `fail`.
- **Final names** are settled in the catalog (M2) and the API registry (M3).

## Milestones after the spike

Each milestone closes with `cargo xtask check` green and its "done when" checks passing on
Windows. macOS parity is verified per milestone from M1 on.

**M1: foundation (+ two early prototypes)**
- **Reset:**
  - remove the old crates (keep `tools/argdump`, `LESSONS.md`);
  - new workspace skeleton;
  - `xtask` (check: fmt, `clippy -D warnings`, tests, TODO/FIXME ban, size report; the bench moves in as `xtask ttfp`; e2e);
  - build mlua+Luau in the skeleton now to flush out toolchain issues on both OSes;
  - delete `spike/` at the end of M1.
- **Shell:**
  - winit loop; the renderer and device policy from M0;
  - hidden until the first frame is presented synchronously; auto-fit layout settles with egui passes that don't present, counted in `xtask ttfp`;
  - an idle window draws nothing; handlers under `catch_unwind`;
  - the manifest; window placement; logging (port master's module); per-OS paths; fatal-error dialog.
- **Fonts:** compiled in from `crates/launcher/assets/fonts` (OFL): Inter for text, with Segoe UI (Windows) or SF (macOS) behind it for scripts Inter lacks; JetBrains Mono NL for all monospace text, the console included; egui's defaults as the last fallback. NL is the build without ligatures, since egui shapes text with harfrust.
- **Look:** egui's stock dark and light styles, changed only where `crates/launcher/theme.kdl` says so (`ui/theme.rs`): any `Style` field by its serde name, plus Launcher's own `extras` (accent and success colours, margins, gaps, step rows) and `fonts { }` (egui's `FontTweak` per face, by file name). Colours come from `ui.visuals()` (`error_fg_color`, `warn_fg_color`, weak text) and `theme::extras(ui)`. Built in for the release app; debug builds and `LAUNCHER_THEME=path` reload the file on save. `docs/theme-reference.kdl` lists every key (generated). `launcher --gallery` (`cargo xtask gallery`) shows every control to tune against. Only content keeps its own colours: the console's ANSI palette and Luau highlighting.
- **Prototype 1: console.**
  - ConPTY spawn via `CreateProcessW` with the pseudo-console attribute, `CREATE_SUSPENDED`, the Job Object assigned, then resume.
  - openpty on macOS.
  - Bytes go to an alacritty_terminal `Term` and are drawn by an egui view forked from egui_term (egui_term tracks an older egui and spawns its own PTY).
- **Prototype 2: macOS open-document events.** The document arrives as an Apple Event before `applicationDidFinishLaunching`, not in argv, so it shapes the startup path. Add `application:openURLs:` to winit's delegate class via objc2, or register a `kAEOpenDocuments` handler before the loop runs.
- **Done when:**
  - an empty themed window on both OSes, `cargo xtask ttfp` within budget, idle CPU ≈ 0;
  - the console prototype runs `git clone` with live progress and accepts typed input;
  - double-clicking a test file on macOS delivers its path.

**M2: config core + `launcher check`**
- **Registry and loading:**
  - kdl v2 parse with spans;
  - port master's `catalog.rs` registry pattern (type, default, docs, OS availability, placement rules);
  - "did you mean" hints; diagnostics (file:line:col, severity, help) with golden-file message tests.
- **Header and values:**
  - the header is read first: `requires` and `os` give friendly messages;
  - `{name}`/`{env.X}` templates;
  - `(lua)` values stored as Lua chunks with file positions;
  - relative paths resolve against the config folder.
- **Resilience:** the last-working-copy cache; watcher reloads (deferred during runs).
- **CLI:** `launcher check <file>…` (exit 1 on errors) and `--help-config`.
- **Generated docs:** `docs/config-reference.md` from the catalog, with a test that it's current.
- **Decide:** whether master's `args` command-line builder (format, replace, dedupe, pairs) stays declarative or becomes a Lua helper.
- **Done when:** parse, validate and diagnostic tests pass; the example configs pass `launcher check`; a broken config falls back to the cached copy.

**M3: Lua + trust gate**
- mlua (`luau`, `vendored`, `send`) in sandbox mode.
- `api.rs` is the single registry: bindings, `docs/lua-api.md`, the selene definitions YAML and autocomplete data all come from it.
- Chunk↔file line and column mapping that accounts for KDL multi-line string dedent.
- An interrupt-based time budget.
- Load-time evaluation on the evaluation thread (option lists, computed values, defaults, conditions).
- **The N9 trust gate** sits in the load pipeline, before any load-time evaluation.
- **Done when:** tests show that the sandbox blocks `io`/`os.execute`, the budget trips, errors map to the right `.launcher` lines, a marked file evaluates nothing before trust, and API calls reach mock effects.

**M4: engine + headless `run`**
- **Runs:**
  - the run worker; run context (form snapshot, variables, environment); config snapshot at run start;
  - effects with real, dry-run (records only) and mock implementations;
  - headless `launcher run <file> [--action id] [--set k=v]… [--dry-run] [--mock]`.
- **`run`:**
  - output: PTY via the M1 console spawn, or pipes for Windows GUI-subsystem exes (read from the PE header) and `io="pipe"`;
  - Windows raw `command-line=` (`raw_arg` for pipes, our own `CreateProcessW` for the PTY; LESSONS L2.7);
  - stdin, timeout, `expect-exit`;
  - killing: the whole process tree (Job Object on Windows; process group plus `killpg` on macOS).
- **Control:**
  - cancel: kills the tree and interrupts Lua;
  - retry from the failed top-level step, keeping run variables;
  - locks: the per-file OS lock plus `lock=<path>`.
- **Environment:**
  - `env-from`: `cmd /d /c "call X && set"`, or `zsh -c 'source X && env -0'`; apply the differences;
  - macOS login-shell capture: `$SHELL -l -i -c`, with markers around the output and a 3 s timeout with fallback.
- **Ported from master:** ping, per-process volume, atomic writes, http-get and extract (now HTTPS: `ureq` + `rustls`).
- **Run lifecycle:**
  - run history and logs; `after=`; keep-awake during runs;
  - modal warnings pause the run;
  - `check` preflight steps; a `fail-if-output` regex.
- **Done when:** `xtask e2e` with `tools/argdump` on both OSes shows exact arguments (and the Windows raw command line), environment, working directory and stdin; Cancel kills grandchildren; the lock blocks a second run of the same file but not a different file; retry resumes at the failed step; dry-run lists the commands.

**M5: console**
- Build out the M1 prototype:
  - one terminal per step section; typing goes to the focused step; resizing the window resizes the PTY;
  - selection and copy; ANSI colours; about 10k lines of scrollback in memory with the full log on disk;
  - repaint at most 60 Hz while output flows, and not at all when idle;
  - pipes mode goes through the same emulator.
- **Done when:** git clone progress, cargo colours, a Unity batch log (pipes on Windows, PTY on macOS) and an interactive stdin prompt all behave, and 10 MB of output stays responsive.

**M6: form UI + run view**
- **Form:**
  - controls and containers from the catalog; `option` data, `preset`, `on-change`; remembered values;
  - `enabled-if`/`visible-if` results from the evaluation thread;
  - asynchronous option lists: spinner, refresh button, error state;
  - input validation (required, regex, path exists) that disables Run and says why;
  - the window fits its content (LESSONS L2.13: fill-width widgets stop an auto-fit window from shrinking; egui has the same trap).
- **Run view:**
  - a step list with status, duration and "last time: 12 min";
  - a console section per step, collapsible;
  - result buttons (open/reveal);
  - Retry;
  - a dry-run view with the current values;
  - confirmation before closing during a run.
- **Done when:** the three example configs render and run end to end, both headless and by hand.

**M7: OS integration + document model**
- **Windows:**
  - `launcher.exe <file>`;
  - a per-config AppUserModelID with relaunch command and display name, so a pinned taskbar entry reopens the right config;
  - taskbar progress (`ITaskbarList3`);
  - a completion toast when the window is unfocused.
- **macOS:**
  - productize the M1 open-document prototype;
  - a new process per extra document;
  - Dock badge progress;
  - notifications.
- **Home screen** when started without a file: recent files, Open…, New from template.
- **The trust prompt UI**, listing the programs the config runs; a trust store.
- **The `requires=` message.**
- **Done when:**
  - double-click opens a file (via the M10 installer, or a dev registration script);
  - a second file on macOS opens in its own process;
  - a downloaded file prompts;
  - a pinned taskbar entry reopens the right config.

**M8: visual editor**
- **Carried over from master's editor structure:**
  - an outline with add, duplicate, delete, and move by drag or Alt+Up/Down;
  - a live preview with a Pick mode;
  - properties generated from the catalog, including the comment lines above a node as its notes, option data tables, a reference picker, and the live value of each template;
  - a bottom panel with Problems, Dry run and the raw text.
- **Editing:**
  - edits go through the ported `config/edit.rs` splices (LESSONS L1.1–L1.4);
  - undo with up to 200 full texts;
  - saves keep a `.bak` and replace the file atomically;
  - an external change on disk triggers a reload, or a prompt if there are unsaved edits.
- **New: a Lua code editor:**
  - full_moon highlighting through `TextEdit`'s layout function, line numbers, Tab handling;
  - validation re-run after 300 ms without typing: full_moon parse errors; selene-lib with generated definitions (undefined globals, argument counts, unused variables, shadowing); cross-checks (`values.x` exists, functions used in `(lua)` properties exist, conditions compile, step names are unique);
  - autocomplete for API names and for `values.` + control ids.
- **Done when:** every catalog node is editable without touching text; comments and formatting survive (golden splice tests); lint fixtures report the right file lines.

**M9: CLI polish**
- `launcher <file>`, `--edit <file>`, `--version`.
- **Windows:** the GUI-subsystem exe attaches to the parent console first (master's `main.rs`); output macros ignore write errors (LESSONS L2.4).
- **macOS:** the CLI is `Launcher.app/Contents/MacOS/launcher`.
- **Done when:** the CLI tests pass on both OSes, including against a closed stdout pipe.

**M10: packaging**
- **Setup:** Developer ID Application certificate; `xcrun notarytool store-credentials`.
- **Windows:** `cargo xtask package-windows` builds an Inno Setup installer:
  - per user (`PrivilegesRequired=lowest`), installed to `{localappdata}\Programs\Launcher`;
  - HKCU association for `.launcher` (open and edit verbs, icon);
  - a Start Menu shortcut with the AppUserModelID;
  - optional user PATH entry;
  - an uninstaller.
- **macOS:** `cargo xtask package-macos`:
  1. Build.
  2. Assemble `Launcher.app`. Its Info.plist holds the bundle id, `CFBundleDocumentTypes`, a `UTExportedTypeDeclarations` entry for `.launcher` conforming to `public.plain-text` (so Quick Look shows the text), and the icon.
  3. `codesign --options runtime --timestamp`.
  4. `hdiutil` DMG with an Applications link; sign it.
  5. `xcrun notarytool submit --wait`.
  6. `xcrun stapler staple`.
- **Done when:** install and uninstall work on clean machines, double-click works, and an upgrade install keeps state.

**M11: examples + docs**
- **git-build:** Windows and macOS files.
- **unity-build:** Windows and macOS files; the editor path comes from `ProjectSettings/ProjectVersion.txt` plus the Hub install path; runs `-batchmode -quit -projectPath -buildTarget -executeMethod -logFile -`; copies the build out.
- **Docs:** generated references and an author guide (write, validate with `launcher check`, commit). Update `LESSONS.md`.
- **Done when:** `launcher check examples/**` passes inside `xtask check`, and each example runs end to end on its OS.

## Reference material on master (port, don't rewrite)

| Path on master | Use |
|---|---|
| `crates/launcher/src/timing.rs` | Process-creation time origin; `LAUNCHER_TTFP_OUT` protocol (M0) |
| `crates/launcher/src/gfx/{mod.rs,d3d11.rs}` | GPU bring-up thread, WARP-first D3D11 device, DXGI flip swapchain (M0 B) |
| `crates/launcher/simgui/sokol_imgui.h` | Precompiled hlsl4/metal_macos shader blobs, `sg_shader_desc` template, texture updates (M0 B) |
| `vendor/sokol-rust` | sokol bindings @ b22a545 |
| `crates/launcher/src/shell.rs` | Hidden-window synchronous first frame, `catch_unwind` wrapper, idle waits, window placement |
| `scripts/ttfp.ps1`, `scripts/lib/app-window.ps1` | Warm-up and median method; finding the real window by PID |
| `crates/launcher/launcher.exe.manifest` | DPI awareness, UTF-8 code page |
| `crates/launcher-core/src/config/{catalog,load,validate,edit}.rs` | Registry pattern, "did you mean" hints, span splices (M2, M8) |
| `crates/launcher/src/editor/` | Editor structure (M8) |
| `crates/launcher-core/src/{state,logging,replace_file,paths}.rs` | FNV-1a state naming, logging, atomic replace |
| `crates/launcher-core/src/platform/{icmp,wasapi,process}.rs` | Ping, per-process volume, raw command line (M4) |
| `crates/launcher-core/tests/*` | reference-docs, diagnostics goldens, `steps.rs` (local HTTP server, stdin, ping) |
| `tools/argdump` | Stand-in program for headless runs (M4) |
| `LESSONS.md` | L1.1–L1.4 (KDL), L2.2, L2.4, L2.7, L2.8, L2.11–L2.13, L4.x still apply |

## Verification

- **M0:** `bench` on Windows (with R) and macOS; `RESULTS.md` holds the tables, raw JSONL and the decision.
- **From M1:**
  - `cargo xtask check` on both OSes before each milestone closes;
  - `cargo xtask e2e` (headless runs with `tools/argdump`) from M4;
  - `cargo xtask ttfp` as a regression check against 150 ms.
- **Manual checklist per milestone:** double-click, typing in the console, resize, DPI, editor round trip, hot reload.

## Risks (not already handled inside a milestone)

| Risk | Mitigation |
|---|---|
| ConPTY quirks (older Windows 10 builds, resize, escape-sequence passthrough) | Test git, cargo, Unity and an interactive prompt in M1/M5; `io="pipe"` per step as a fallback |
| egui API churn; egui_term lagging egui | Pin egui, egui-winit and winit together; own the forked terminal view |
| sokol resource API in flux | Keep the vendored pin; upgrade deliberately |
| Luau C++ build; selene-lib size and compile time | Build Luau in M1; measure lint cost in M8 |
| AV/EDR flags an unsigned exe that starts processes | No generated scripts, no hidden script hosts; IT allowlists the install path if needed |
| A git branch switch rewrites the running config | Config snapshot at run start; reload after the run |

## Open decisions (default in brackets; settle when reached)

- Visual editor as a mode of the main window [default] or a separate window, as on master (M8).
- macOS build: arm64 only [if every team Mac is Apple Silicon] or universal (M10).
- ~~master's `args` builder: declarative [default, editor-friendly] or a Lua helper (M2).~~ Decided: declarative (N17).
- Notifications: `notify-rust` or native APIs (M7).
