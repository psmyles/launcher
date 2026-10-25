# Third-party software in Launcher

The launcher binary (`launcher.exe` on Windows, `Launcher.app/Contents/MacOS/launcher` on macOS)
statically links the following. Each is used under its license; the full license texts are in
each project's repository and in its published crate source. The installers put this file, the
launcher's own `LICENSE.txt` and the licenses that must travel with a binary (`licenses/`: the
fonts' and subtle's) beside the binary.

| Component | License | Use |
|---|---|---|
| egui, epaint, emath, ecolor, egui-winit 0.36 | MIT OR Apache-2.0 | The UI |
| egui's default fonts (epaint_default_fonts 0.36): Hack, Ubuntu Light, Noto Emoji, emoji-icon-font | MIT, Ubuntu Font Licence 1.0, OFL-1.1 | Fallback glyphs |
| Inter (rsms/inter) | OFL-1.1 (`licenses/Inter-OFL.txt`) | Text font |
| JetBrains Mono NL | OFL-1.1 (`licenses/JetBrainsMono-OFL.txt`) | Monospace and console font |
| skrifa, read-fonts, font-types (fontations); harfrust; vello_cpu, vello_common, kurbo, peniko, color, fearless_simd (Linebender) | MIT OR Apache-2.0 (harfrust: MIT) | Text shaping and rasterizing |
| sokol_gfx (floooh/sokol, via sokol-rust @ b22a545) | zlib | Drawing (D3D11, Metal) |
| winit 0.30 | Apache-2.0 | Windows and events |
| arboard 3.6 | MIT OR Apache-2.0 | Clipboard |
| clipboard-win 5.4, error-code 3.4 | BSL-1.0 | Clipboard (Windows) |
| alacritty_terminal 0.26, vte | Apache-2.0 (vte: Apache-2.0 OR MIT) | The console's terminal model |
| Luau (luau0-src), mlua 0.12 | MIT | The scripting language |
| full_moon 2.2, selene-lib 0.31 | MPL-2.0 (source: github.com/Kampfkarren/full-moon, github.com/Kampfkarren/selene) | Luau parsing and linting in the editor |
| kdl 6.7, miette 7.6 | Apache-2.0 | The `.launcher` file format |
| serde, serde_json 1 | MIT OR Apache-2.0 | JSON |
| regex 1 | MIT OR Apache-2.0 | Patterns |
| notify 8.2 | CC0-1.0 | Watching files |
| ureq 3.4, rustls 0.23 | MIT OR Apache-2.0 (rustls: also ISC) | HTTP(S) |
| ring 0.17 | Apache-2.0 AND ISC | Cryptography for TLS |
| rustls-webpki 0.103, untrusted 0.9 | ISC | Certificate checking for TLS |
| subtle 2.6 | BSD-3-Clause (`licenses/subtle-BSD-3-Clause.txt`) | Constant-time comparisons for TLS |
| webpki-roots 1.0 | CDLA-Permissive-2.0 | TLS root certificates |
| rfd 0.17 | MIT | File and folder pickers, message boxes |
| windows 0.61 (windows-rs) | MIT OR Apache-2.0 | Win32 APIs (Windows) |
| objc2, objc2-foundation, objc2-metal, objc2-quartz-core, raw-window-metal | MIT (some also Apache-2.0 or zlib) | AppKit and Metal (macOS) |
| unicode-ident | (MIT OR Apache-2.0) AND Unicode-3.0 | Unicode tables |
| ICU4X (icu_normalizer, icu_properties, icu_collections, icu_locale_core, icu_provider, zerovec, zerotrie, zerofrom, yoke, tinystr, litemap, writeable, potential_utf), via url and idna | Unicode-3.0 | Unicode normalization of host names |
| foldhash 0.2 | Zlib | Hashing |
| indexmap, hashbrown, smallvec, memchr, base64, libc and the other small crates in `cargo tree -p launcher` | MIT and/or Apache-2.0 (some also offer Zlib, BSL-1.0 or Unlicense) | Supporting code |

On Windows, Segoe UI is read from the Windows fonts folder at run time; on macOS, the system font
is. Neither is shipped.
