# Launcher

Launcher turns a script, or a procedure that lives in one person's head, into a form anyone on
the team can run. A tool is one short `.launcher` text file: a window with a few fields and a
button, and the steps the button runs. Double-click the file, fill in the form, press the button,
and watch every step run in a live console. It runs on Windows and macOS.

## Download

Get the latest version from [Releases](https://github.com/psmyles/launcher/releases/latest):

- **Windows:** `Launcher-<version>-setup.exe`. It installs for your user only, with no admin
  rights needed, and opens `.launcher` files when you double-click them.
- **macOS (Apple silicon):** `Launcher-<version>.dmg`, signed and notarized.

## What a tool looks like

This one checks a folder of FBX models against an audit preset from
[3D Review](https://github.com/psmyles/3d-review), then makes LODs for the models that failed,
in place, so the game engine picks them up:

```kdl
tool "Audit and Optimize Models" format=2 requires="0.2"

window {
    file "profile" label="Audit preset" extensions="json" required=#true
    file "preset" label="Opt preset" extensions="json" required=#true
    folder "models" label="Models" required=#true
    button "start" label="Start"
}

steps "start" {
    review3d.audit "audit" profile="{profile}" models="{models}"
    review3d.run-preset "run-preset" preset="{preset}" models="{audit.to-fix}" save-to="{models}" replace-models=#true
}
```

`window` is the form. `steps` is what the button does, and each step can use what an earlier
one found: here, `{audit.to-fix}` is the list of models the audit flagged. Commit the file next
to the work it's for, and everyone who pulls gets the new version.

## What it does

- **Forms:** files, folders, text, numbers, drop-downs and tick boxes. Fields can show, hide or
  turn off based on other fields. Each field remembers its last value. Until something required
  is filled in, the button stays off and says why.
- **Steps from add-ons:**
  - Run any program and read its output.
  - Files and folders.
  - Git and .NET builds.
  - Web requests.
  - Questions and messages for the person running it.
  - [3D Review](https://github.com/psmyles/3d-review) and [Bite](https://github.com/psmyles/bite).
- **Add-ons are text files too.** Each one is a `.kdl` file. Put one in the Add-ons folder (on
  the home screen, under More) and its steps show up in every tool, with no new build.
- **A live console:** every program runs in a real terminal, so progress bars and colours show
  up, and you can type into it when a program asks.
- **A visual editor:** the Edit button in a tool's window opens it (so does right-click > Edit
  on Windows). It changes only the lines you touch, so comments and layout stay as you wrote
  them.
- **Without the window:** `launcher run FILE --set models=./Mesh` runs a tool from a script or a
  build server. Add `--preview` to see what each step would do without running anything.
- **Safe by default:** a tool's Lua code can't touch files or start programs by itself.
  Everything goes through steps you can see in the console.

## Writing your own

- Choose **Make a new tool** on Launcher's home screen and start from an example, or copy one of
  the [starters](examples/starters).
- [Step reference](docs/step-reference.md): every step and setting, by add-on.
- [Lua API](docs/lua-api.md): for values and conditions that need more than text.
- Full examples: [git-build](examples/git-build) and [unity-build](examples/unity-build).

## Build from source

Requirements: Rust (the pinned toolchain in `rust-toolchain.toml` installs itself), and on Windows
Visual Studio 2022 Build Tools with "Desktop development with C++" (sokol and Luau are C/C++).

```sh
cargo build --release -p launcher     # target/release/launcher(.exe)
cargo xtask check                     # the gate: fmt, clippy, tests, macOS type-check, release build
cargo xtask ttfp                      # time to first frame (budget: 150 ms median)
cargo xtask idle                      # an idle window must use ~no CPU
```

## Packaging

The Windows icon is drawn from `assets/application-logo.png`. After you change it, run
`cargo xtask icons` and commit the redrawn `assets/launcher.ico`. Every xtask release build, and
`cargo xtask check`, also redraws it when it's out of date, and a test fails until it's current.
The macOS icon is `assets/launcher-app.icon` (edit it in Icon Composer). `package-macos` compiles
it, which needs Xcode 26 or later.

**Windows** (needs [Inno Setup](https://jrsoftware.org/isdl.php) 6.5.2 or later):

```sh
cargo xtask package-windows           # target/package/Launcher-<version>-setup.exe
```

A per-user install into `%LOCALAPPDATA%\Programs\Launcher`, no admin rights. Double-clicking a
`.launcher` file opens it in the launcher, and right-click > Edit opens the visual editor. Setup
adds a Start Menu shortcut, and optionally a desktop shortcut and a PATH entry. Uninstalling takes
all of that out again and keeps the user's app data.

**macOS** (on an Apple silicon Mac with the Xcode command-line tools):

```sh
cargo xtask package-macos             # target/package/Launcher-<version>.dmg
cargo xtask package-macos dev         # the same, unsigned and not notarized
```

This builds `Launcher.app`, which `.launcher` files open in. It signs the app with the keychain's
Developer ID Application identity, notarizes and staples it, then does the same for the DMG.
Before the first run you need the certificate in the keychain and a stored notary profile:
`xcrun notarytool store-credentials fire --apple-id <id> --team-id <team> --password <app-specific password>`.
To try a build, pass `--no-notarize` (signed only) or `dev` (the same as `--no-sign`; runs only on
that Mac). `--sign-id` and `--notary-profile` choose other credentials.

## Layout

| Path | What |
|---|---|
| `crates/launcher-core` | Everything without a window: config, Lua, engine, OS primitives (PTY, processes), state, logging |
| `crates/launcher` | The app: winit loop, the sokol_gfx painter for egui, the UI, the console, the CLI |
| `crates/sokol-gfx` | sokol_gfx + sokol_log from `vendor/sokol-rust` |
| `addons` | The built-in add-ons, one `.kdl` file each |
| `examples` | Starters and full example tools |
| `tools/argdump` | A stand-in program for tests: records its exact command line, environment and stdin |
| `xtask` | Automation (`cargo xtask ...`) |
| `docs/` | Plan, decisions, references, renderer benchmark results |

Lessons learned the hard way: [`LESSONS.md`](LESSONS.md).

## Licence

MIT, see [LICENSE](LICENSE). The software Launcher is built on, and its licences, are listed in
[THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).
