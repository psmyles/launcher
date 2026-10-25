//! `cargo xtask <command>`: the project's automation (docs/PLAN.md, M1).
//!
//! * `check [--fast] [--no-macos]`: the gate every milestone closes on.
//! * `ttfp [--n 30] [--file F] [--budget 150]`: time to first frame of the release build.
//! * `idle [--file F]`: CPU used by an idle window (must be about zero).
//! * `docs`: regenerate the docs generated from the registries.
//! * `e2e`: headless runs checked against what `tools/argdump` recorded.
//! * `gallery`: the theme gallery, reloading `crates/launcher/theme.kdl` as it's saved.
//! * `icons`: redraw `assets/launcher.ico` from `assets/application-logo.png` (macOS uses
//!   `assets/launcher-app.icon`).
//! * `package-windows` / `package-macos`: the installer and the signed, notarized DMG (M10).
//! * `register` / `unregister`: open `.launcher` files with this checkout's release build.

mod check;
mod docs;
mod e2e;
mod fakecc;
mod icons;
mod package;
mod register;
mod ttfp;
mod util;

fn main() {
    if let Some(code) = fakecc::dispatch() {
        std::process::exit(code);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some((cmd, rest)) = args.split_first() else {
        usage();
        std::process::exit(2);
    };
    let result = match cmd.as_str() {
        "check" => check::run_check(rest),
        "ttfp" => ttfp::run_ttfp(rest),
        "idle" => ttfp::run_idle(rest),
        "docs" => docs::run_docs(rest),
        "e2e" => e2e::run_e2e(rest),
        "gallery" => run_gallery(),
        "icons" => icons::run_icons(rest),
        "package-windows" => package::run_windows(rest),
        "package-macos" => package::run_macos(rest),
        "register" => register::run_register(rest),
        "unregister" => register::run_unregister(rest),
        "help" | "--help" | "-h" => {
            usage();
            Ok(())
        }
        other => Err(format!("unknown command `{other}`")),
    };
    if let Err(e) = result {
        eprintln!("xtask: {e}");
        std::process::exit(1);
    }
}

/// The gallery runs as a debug build: those read the theme file from the source tree and reload
/// it on save (`ui/theme.rs`).
fn run_gallery() -> Result<(), String> {
    util::run(util::cargo().args(["run", "-p", "launcher", "--", "--gallery"]))
}

fn usage() {
    eprintln!(
        "usage: cargo xtask <command>

  check [--fast] [--no-macos]   fmt, clippy (-D warnings), tests, TODO/FIXME ban, macOS type-check,
                                release build and size report (--fast skips the release build)
  ttfp [--n N] [--file F] [--budget MS] [--no-build]
                                time to first frame of the release build (median of N launches)
  idle [--file F] [--seconds S] [--no-build]
                                CPU used by the idle window after it settles
  docs                          regenerate docs/config-reference.md (and the other generated docs,
                                docs/theme-reference.kdl among them)
  e2e                           headless end-to-end runs against tools/argdump
  gallery                       every control in the current theme; a debug build, so saving
                                crates/launcher/theme.kdl restyles it live
  icons [--force] [--preview DIR]
                                redraw assets/launcher.ico from assets/application-logo.png if it
                                changed; --preview also writes Setup's wizard images and (on a
                                Mac) the iconset drawn from assets/launcher-app.icon to DIR
  package-windows [--no-build] [--version V]
                                the per-user installer (Inno Setup) in target/package
  package-macos [--no-build] [--no-sign] [--sign-id ID] [--no-notarize] [--notary-profile P]
                [--no-dmg]
                                Launcher.app and a signed, notarized DMG in target/package
  package-macos dev [--no-build] [--no-dmg]
                                the same, unsigned and not notarized (as --no-sign); runs on
                                this Mac
  register [--no-build]         open .launcher files with this checkout's release build (Windows,
                                this user only)
  unregister                    undo register"
    );
}
