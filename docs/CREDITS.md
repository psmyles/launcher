# Credits

Launcher is built on the projects listed below. For license terms, see
[THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md).

## Window, GPU and interface

- **[winit](https://github.com/rust-windowing/winit)** - the winit contributors, Pierre Krieger,
  Kirill Chibisov and others. Windows, the event loop, input and DPI, with
  **[dpi](https://github.com/rust-windowing/winit)**,
  **[raw-window-handle](https://github.com/rust-windowing/raw-window-handle)**,
  **[raw-window-metal](https://github.com/rust-windowing/raw-window-metal)** and
  **[cursor-icon](https://github.com/rust-windowing/cursor-icon)**.
- **[sokol](https://github.com/floooh/sokol)** and
  **[sokol-rust](https://github.com/floooh/sokol-rust)** - Andre Weissflog (bindings by Erik
  Wilhelm Gren). `sokol_gfx` draws over Direct3D 11 and Metal.
- **[egui](https://github.com/emilk/egui)** - Emil Ernerfeldt and contributors. The interface,
  with **egui-winit**, **epaint**, **emath** and **ecolor**. Text shaping and rendering:
  **[skrifa, read-fonts and font-types](https://github.com/googlefonts/fontations)** (the
  Fontations developers), **[harfrust](https://github.com/harfbuzz/harfrust)** (the HarfBuzz
  developers and Yevhenii Reizner), and **[vello_cpu](https://github.com/linebender/vello)**,
  **[kurbo](https://github.com/linebender/kurbo)**, **[peniko](https://github.com/linebender/peniko)**
  and **[color](https://github.com/linebender/color)** (Raph Levien and the Linebender and Vello
  authors).
- **[arboard](https://github.com/1Password/arboard)** - the Arboard contributors, with
  **[clipboard-win](https://github.com/DoumanAsh/clipboard-win)** by Douman. Clipboard.
- **[webbrowser](https://github.com/amodm/webbrowser-rs)** - Amod Malviya. Opens links in the
  browser.
- **[png](https://github.com/image-rs/image-png)** - the image-rs developers. The interface
  icons.

## Console

- **[alacritty_terminal](https://github.com/alacritty/alacritty)** - Joe Wilm, Christian Duerr
  and the Alacritty contributors. The terminal model behind each step's console, with
  **[vte](https://github.com/alacritty/vte)** for the escape-sequence parser.
- **[egui_term](https://github.com/Harzu/egui_term)** - Harzu. The idea of drawing an Alacritty
  terminal in egui; Launcher's console view is its own.

## Scripting

- **[Luau](https://github.com/luau-lang/luau)** - Roblox Corporation and the Luau contributors.
  The scripting language in `.launcher` files and add-ons.
- **[mlua](https://github.com/mlua-rs/mlua)** - Aleksandr Orlenko and contributors. The sandboxed
  Luau VM, built from **[luau0-src](https://github.com/mlua-rs/luau-src-rs)**.
- **[full_moon](https://github.com/Kampfkarren/full-moon)** - Kampfkarren and contributors.
  Luau parsing: the editor's highlighting and parse errors.
- **[selene](https://github.com/Kampfkarren/selene)** - Kampfkarren and contributors. Luau
  linting in the editor and `launcher check`.

## File format

- **[KDL](https://kdl.dev)** - Kat Marchán and the KDL contributors. The language `.launcher`
  files, add-ons and the theme are written in, read through
  **[kdl-rs](https://github.com/kdl-org/kdl-rs)**.
- **[miette](https://github.com/zkat/miette)** - Kat Marchán. Source spans for diagnostics.
- **[serde](https://github.com/serde-rs/serde)** and
  **[serde_json](https://github.com/serde-rs/json)** - Erick Tryzelaar, David Tolnay and
  contributors. State, recent files, reports from other tools, and the theme mapped onto egui's
  style.
- **[regex](https://github.com/rust-lang/regex)** - Andrew Gallant and the Rust project.
  Patterns in steps and checks.

## Typefaces

- **[Inter](https://github.com/rsms/inter)** - Rasmus Andersson and the Inter Project Authors.
  Interface text.
- **[JetBrains Mono](https://github.com/JetBrains/JetBrainsMono)** - JetBrains, designed by
  Philipp Nurullin and Konstantin Bulenkov. The NL cut, for the console and all monospace text.
- egui's fallback faces:
  **[Ubuntu](https://design.ubuntu.com/font)** (Canonical), **[Hack](https://github.com/source-foundry/Hack)**
  (Source Foundry, from Bitstream Vera and DejaVu), **[Noto Emoji](https://github.com/googlefonts/noto-emoji)**
  (Google) and **[emoji-icon-font](https://github.com/jslegers/emoji-icon-font)** (John Slegers).
- Segoe UI (Microsoft) and the San Francisco system font (Apple) are read from the system at run
  time for scripts Inter lacks. Neither is shipped.

## Network

- **[ureq](https://github.com/algesten/ureq)** - Martin Algesten and contributors. HTTP(S) for
  the Internet add-on.
- **[rustls](https://github.com/rustls/rustls)** - Joseph Birr-Pixton and the rustls
  contributors. TLS, with **[ring](https://github.com/briansmith/ring)** by Brian Smith for the
  cryptography and **[webpki-roots](https://github.com/rustls/webpki-roots)** for the root
  certificates, from Mozilla's CA program via the CCADB.
- **[ICU4X](https://github.com/unicode-org/icu4x)** - the ICU4X Project Developers, Unicode, Inc.
  Unicode normalization, via `url` and `idna`.

## Platform

- **[windows](https://github.com/microsoft/windows-rs)** - Microsoft. Win32 and COM bindings:
  the D3D11 device and DXGI swapchain, ConPTY, Job Objects, the taskbar and notifications.
- **[objc2](https://github.com/madsmtm/objc2)** - Mads Marquart, Steven Sheldon and contributors.
  Objective-C bindings for AppKit, Metal and the `CAMetalLayer`.
- **[libc](https://github.com/rust-lang/libc)** - the Rust project. Pseudo-terminals, process
  groups and signals on macOS.
- **[rfd](https://github.com/PolyMeilex/rfd)** - Bartłomiej Maryńczak. File and folder pickers
  and message boxes.
- **[notify](https://github.com/notify-rs/notify)** - Félix Saparelli, Daniel Faust, Aron Heinecke
  and contributors. Reloads a tool when its file changes.
- **[Inno Setup](https://jrsoftware.org/isinfo.php)** - Jordan Russell and Martijn Laan. The
  Windows installer.

## Infrastructure

- **[syn](https://github.com/dtolnay/syn)**, **[quote](https://github.com/dtolnay/quote)**,
  **[proc-macro2](https://github.com/dtolnay/proc-macro2)** and
  **[thiserror](https://github.com/dtolnay/thiserror)** - David Tolnay.
- **[hashbrown](https://github.com/rust-lang/hashbrown)**,
  **[indexmap](https://github.com/indexmap-rs/indexmap)**,
  **[smallvec](https://github.com/servo/rust-smallvec)**,
  **[parking_lot](https://github.com/Amanieu/parking_lot)**,
  **[memchr](https://github.com/BurntSushi/memchr)**,
  **[zerocopy](https://github.com/google/zerocopy)** and others - 209 crates in all across the
  Windows and macOS builds; `cargo tree -p launcher` lists them.

## Build tooling

Used to build and package Launcher; not linked into the binary.

- **[cc](https://github.com/rust-lang/cc-rs)** - the Rust project. Compiles sokol and Luau.
- **[winresource](https://github.com/BenjaminRi/winresource)** - Benjamin Richner. The Windows
  icon and version resource.
- **[image](https://github.com/image-rs/image)** - the image-rs developers. Draws
  `assets/launcher.ico` from the logo (`cargo xtask icons`).
- **Xcode: Icon Composer, `actool`, `ictool`, `codesign` and `notarytool`** - Apple. The macOS
  app icon, signing and notarization.
- **The Rust project and its contributors** - the language, compiler, standard library and Cargo.

## Development

- **[Claude Code](https://claude.com/claude-code)** - Anthropic. Coding assistant.

## Original work

The following are original to Launcher and MIT licensed with the rest of the project: the
application logo and icons (`assets/`), the theme (`crates/launcher/theme.kdl`), the built-in
add-ons (`addons/`), the examples and starters (`examples/`) and the documentation.

---

_Something missing or miscredited? Open an issue at
<https://github.com/psmyles/launcher/issues/new>._
