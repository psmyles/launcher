# Third-party software in Launcher

The launcher binary (`launcher.exe` on Windows, `Launcher.app/Contents/MacOS/launcher` on macOS)
statically links the following. Each is used under its license; the full license texts are in
each project's repository and in its published crate source. The installers put this file, the
launcher's own `LICENSE.txt` and the font licenses (`licenses/`) beside the binary.

| Component | License | Use |
|---|---|---|
| egui, epaint, emath, ecolor, egui-winit 0.36 | MIT OR Apache-2.0 | The UI |
| egui's default fonts (epaint_default_fonts 0.36): Hack, Ubuntu Light, Noto Emoji, emoji-icon-font | MIT, Ubuntu Font Licence 1.0, OFL-1.1 | Fallback glyphs |
| Inter (rsms/inter) | OFL-1.1 (`licenses/Inter-OFL.txt`) | Text font |
| JetBrains Mono NL | OFL-1.1 (`licenses/JetBrainsMono-OFL.txt`) | Monospace and console font |
| ab_glyph | Apache-2.0 | Font rasterizing |
| sokol_gfx (floooh/sokol, via sokol-rust @ b22a545) | zlib | Drawing (D3D11, Metal) |
| winit 0.30 | Apache-2.0 | Windows and events |
| arboard 3.6 | MIT OR Apache-2.0 | Clipboard |
| alacritty_terminal 0.26, vte | Apache-2.0 (vte: Apache-2.0 OR MIT) | The console's terminal model |
| Luau (luau0-src), mlua 0.12 | MIT | The scripting language |
| full_moon 2.2, selene-lib 0.31 | MPL-2.0 (source: github.com/Kampfkarren/full-moon, github.com/Kampfkarren/selene) | Luau parsing and linting in the editor |
| kdl 6.7, miette 7.6 | Apache-2.0 | The `.launcher` file format |
| serde, serde_json 1 | MIT OR Apache-2.0 | JSON |
| regex 1 | MIT OR Apache-2.0 | Patterns |
| notify 8.2 | CC0-1.0 | Watching files |
| ureq 3.4, rustls 0.23 | MIT OR Apache-2.0 (rustls: also ISC) | HTTP(S) |
| ring 0.17 | Apache-2.0 AND ISC | Cryptography for TLS |
| webpki-roots 1.0 | CDLA-Permissive-2.0 | TLS root certificates |
| rfd 0.17 | MIT | File and folder pickers, message boxes |
| windows 0.61 (windows-rs) | MIT OR Apache-2.0 | Win32 APIs (Windows) |
| objc2, objc2-foundation, objc2-metal, objc2-quartz-core, raw-window-metal | MIT (some also Apache-2.0 or zlib) | AppKit and Metal (macOS) |
| unicode-ident | (MIT OR Apache-2.0) AND Unicode-3.0 | Unicode tables |
| indexmap, hashbrown, smallvec, memchr, base64, libc and the other small crates in `cargo tree -p launcher` | MIT and/or Apache-2.0 | Supporting code |

On Windows, Segoe UI is read from the Windows fonts folder at run time; on macOS, the system font
is. Neither is shipped.
