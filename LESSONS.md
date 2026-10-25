# Lessons

Things learned the hard way while building this, each with where it bit. Nothing here is folklore:
most have a test or a comment at the place they apply.

## KDL (kdl 6.7.1)

- **L1.1** Re-serializing a parsed document drops a `// comment` that follows a `}` on the same
  line (and the newline after it). `KdlDocument::to_string()` is therefore not a safe save path. The
  editor splices text at spans instead (D26, `config/edit.rs`).
- **L1.2** `KdlEntry::set_value` leaves `format.value_repr` stale, and `Display` prints the repr,
  so the old text is saved. Another reason not to mutate-and-print.
- **L1.3** Entry spans cover `name=value`; the value starts at `span.end - value_repr.len()`.
  Offsets inside a string map exactly only for raw strings and quoted strings without escapes.
- **L1.4** KDL v2 pitfalls people hit: `} else {` on one line is a syntax error (`else` is a new
  node); bare `true` is invalid (`#true`); inside `"..."` a backslash starts an escape, so
  `"C:\Windows"` fails and `".\results.json"` silently contains a carriage return. Raw strings
  `#"..."#` are the answer for paths and JSON. The loader's hints say exactly this.

## Windows

- **L2.1** Creating a D3D11 hardware device costs ~130 ms (the vendor's user-mode driver loads);
  WARP costs ~30 ms. For a small UI drawn on input, WARP wins the startup budget (D24).
- **L2.2** A hidden window gets no `WM_PAINT`, so `request_redraw` never produces
  `RedrawRequested` before it is shown. Draw the settle frames synchronously while hidden
  (`shell.rs`), resizing the swapchain to the current client size first.
- **L2.3** An `ALWAYS_AUTO_RESIZE` ImGui window cannot size the OS window: ImGui clamps auto-fit to
  the display size, which is the OS window. Measure `DC.CursorMaxPos` instead; ImGui 1.92 does not
  maintain `DC.IdealMaxPos` for a window that is not auto-fitting (`ui/measure.rs`).
- **L2.4** Shells do not wait for a GUI-subsystem exe, and a pipe they close makes `println!`
  panic. All CLI output goes through non-panicking writes; scripts use `Start-Process -Wait`.
- **L2.5** `IcmpSendEcho2` only exists with the `Win32_System_IO` feature (its APC parameter type);
  `IMMDevice::Activate` needs `Win32_System_Com_StructuredStorage` + `Win32_System_Variant`.
- **L2.6** A WASAPI session enumerator is a snapshot: enumerate again on every poll.
- **L2.7** `Command::arg` would quote `Key="value"` again; command lines from the config go through
  `raw_arg`.
- **L2.8** Watch the config's *folder*, not the file: editors save by renaming a temp file over
  it. Set the debounce deadline on the first matching event and check it after every wake-up, or a
  stream of unrelated events in the same folder (the log file) can starve it.
- **L2.9** A desktop-wide UI Automation search blocks on any hung window (a suspended Settings app
  did it). Find windows with `EnumWindows` filtered by process id first.
- **L2.10** A .NET Framework WinForms button draws itself and ignores `BM_CLICK` and synthetic mouse
  messages; `WM_COMMAND`/`BN_CLICKED` to its parent works (WinForms reflects it to `OnClick`). Its
  automation ids are window handles, and buttons show up as panes.
- **L2.11** Never use `Process.MainWindowHandle` on a winit app: winit's helper window ("Winit
  Thread Event Target") is unowned and reports visible, so while the real window is still hidden
  (settle frames) `MainWindowHandle` returns the helper. Messages posted to it vanish, and a
  `WM_CLOSE` destroys it. Scripts use `scripts/lib/app-window.ps1`, which skips that class.
- **L2.12** The config is read before the watcher exists; a save in that gap is never reported.
  After starting the watcher, queue one "changed" check (the content hash makes it a no-op).
- **L2.13** Auto-fit and stretching widgets feed each other: a field sized to the available width
  makes the measured content as wide as the window, so the window never shrinks (it kept the
  editor's width). Stretching fields end at the form's own right edge and report only their
  minimum to the natural-width measure (`ui/form_view.rs`).
- **L2.14** ImGui's `SameLine(offset)` counts from the current group's indent; inside a group it
  shifts a "field column" x. Continue the line with `SameLine()` then `SetCursorPos` instead.
- **L2.15** ImGui applies queued input a step per frame (a press and its release land in different
  frames). If the loop stops drawing when the events stop, a quick click's release waits for the
  next event. Keep drawing while `InputEventsQueue` is not empty.

- **L2.16** A window accepts a new flip-model swapchain only once the old one is really gone.
  Dropping it is not enough: D3D11 destroys lazily, so release everything sokol made, drop the
  swapchain, then `ClearState()` + `Flush()` the old immediate context. Otherwise
  `CreateSwapChainForHwnd` fails with "Access is denied" (the WARP → hardware switch,
  `gfx/mod.rs`).
- **L2.17** On WARP, "present returned" does not mean "drawn": WARP rasterizes on its own threads
  after the submit. An event query after the present tells when a frame is really done (M0).
- **L2.18** egui 0.36 panics when a `TexturesDelta` is dropped without `clear()`: clear it once the
  painter has applied it, early returns included (`shell.rs`).
- **L2.19** egui-winit answers `RedrawRequested` itself with `repaint: true`. A shell that requests
  a redraw for it redraws forever (M0).
- **L2.20** On macOS a panic inside a winit handler aborts the process: the callback comes from
  Objective-C and cannot unwind. The firewall (N15) is needed there too.
- **L2.21** ConPTY keeps the output pipe open until `ClosePseudoConsole`, so a reader never sees
  the end by itself: close the console after the process exits, while the reader keeps draining
  (older builds block in the close until it's drained). And give the child
  `STARTF_USESTDHANDLES` with invalid handles, or a launcher that attached its parent's console
  hands that console to the child instead of the pty (`platform/pty.rs`).
- **L2.22** egui's `show_rows` needs truly fixed-height rows; wrapped rows make the content height
  oscillate and the scroll bar animate forever (M0). One grid row per console line.
- **L2.23** alacritty_terminal 0.26 ignores line-feed mode (`ESC[20h`) for a plain `\n`; only NEL
  honours it. Pipe output (no pty to add the `\r`) needs the `\r` added before parsing
  (`console/mod.rs`).
- **L2.24** In a ConPTY, cargo and other Rust tools print no colour (and skip their erase-line
  sequences, leaving progress-bar remnants) unless `TERM` is set. Give pty children
  `TERM=xterm-256color` and `COLORTERM=truecolor` unless the run sets them (`platform/child.rs`).
- **L2.25** egui's `drag_started` fires once the pointer has moved past a threshold, and
  `interact_pointer_pos` is then the current position. Start a selection at
  `pointer.press_origin()` (`console/view.rs`).
- **L2.26** Every ConPTY session starts with `ESC[2J ESC[H`. alacritty keeps the cleared lines in
  the scrollback, just above the screen, so a step's earlier output (a `log` line, a previous
  program) vanished from view. Following output shows as much scrollback as fits above the cursor
  (`TermModel::newest_offset`). A session that printed nothing still sent escape sequences: a
  console counts as having output only once it shows text.
- **L2.28** egui's `ui.put` still takes space in the layout it's called in. Something drawn over the
  layout (the form's Edit corner button) belongs in an `Area`, or it shifts the form and the
  window's fit.
- **L2.29** A flip-model swapchain in a window hidden behind others doesn't present, and
  `PrintWindow` then returns the last frame that did. A screenshot of a background window can be
  stale.
- **L2.27** A program started detached inherits the console the launcher borrowed from its parent
  (`AttachConsole`): a console program then waits on that console's input or prints into the
  launching terminal. Start it with `CREATE_NEW_CONSOLE` (`platform/child.rs`).

## Lua (Luau, mlua 0.12)

- **L5.1** In a sandboxed Luau VM, `values.x` compiles to an import that is resolved once, when
  the chunk loads. A condition compiled once then returns its first answer forever. Declare the
  views that change mutable: `Compiler::set_mutable_globals(["values", "fields", "env"])`
  (`lua/mod.rs`; the test `a_compiled_condition_reads_the_current_values`).

## Newtonsoft.Json compatibility (D20)

- **L3.1** `JsonConvert.DeserializeObject<JObject>` parses ISO-looking strings (19–40 chars,
  `T` at index 10) as `DateTime`: fractions lose trailing zeros, `z` becomes `Z`, `24:00:00` rolls
  over, more than 7 fraction digits is not a date, and an offset is **converted to the machine's
  local time** and written with the local offset. The output depends on the time zone.
- **L3.2** Doubles are written with .NET Framework's `"R"` (G15 if it round-trips, else G17) plus
  `.0` when they would read as integers; `1e15` becomes `1E+15`, `0.00001` becomes `1E-05`,
  `-0.0` becomes `0.0`. Integers keep their digits (`-0` → `0`).
- **L3.3** `\u` escapes are lowercase hex; U+0085/U+2028/U+2029 are escaped; nothing else beyond
  control characters, `"` and `\`.

## Process

- **L4.1** Python heredocs on Windows silently turn `'\n'`, `'\\0'` and `"\W"` into control
  characters or eat backslashes, and Python text mode writes CRLF. Patch files with the editor, or
  with scripts written to a file using raw strings and `newline=''`.
- **L4.2** PowerShell: `$Args` is automatic (never a parameter name); a `[string]` parameter
  re-joins an array assigned to it; Windows PowerShell 5.1 turns a native tool's stderr into
  errors under `$ErrorActionPreference = "Stop"` (run such tools through `cmd /c`).
- **L4.3** `powershell -File script.ps1 -Param a,b` does not parse arrays: the script gets one
  string. Pass one argument-line string instead.
- **L4.5** Git Bash can't create NTFS alternate data streams: `printf ... > "f:Zone.Identifier"`
  makes a file whose name holds a private-use character instead. Write streams with PowerShell's
  `Set-Content -Stream` (or Rust's `std::fs::write("f:Zone.Identifier")`, which works).
- **L4.4** Type-checking macOS from Windows: build scripts that compile C/C++ (Luau) need the
  target's compiler even under `cargo check`. `check` never links, so stand-in `cc`/`ar` that
  write empty outputs are enough (`xtask/src/fakecc.rs`).
- **L4.8** While someone uses the desktop, Windows won't bring another app's window to the
  front, and synthesized input (SendInput, SendKeys) goes to whatever window is under the pointer.
  UI automation must check the foreground window first and stop, never click blind. Posted mouse
  messages don't reach egui through winit.
- **L4.7** Toasts can be switched off per user (`HKCU\...\PushNotifications\ToastEnabled=0`); then a
  posted notification succeeds and shows nothing. Check that before debugging a missing toast.
- **L4.6** The agent's shell sets `NO_COLOR=1`, and everything started from it inherits it:
  clear it before judging whether a program prints colour in the console.
