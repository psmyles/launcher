//! `cargo xtask package-windows` and `package-macos` (N13, M10).
//!
//! * **Windows:** the icon redrawn if the artwork changed (`icons`), the release build, then Inno
//!   Setup (`packaging/windows/launcher.iss`): a per-user installer in `target/package/` that makes
//!   `.launcher` files open (and "Edit") in the launcher and installs the licence notices.
//! * **macOS:** the release build for Apple silicon, then `Launcher.app`: the icon from
//!   `assets/launcher-app.icon` (`Assets.car` and an icns, made by `icons`), an Info.plist that declares the `.launcher` document type (exported as plain text so
//!   Quick Look shows it), the licence notices. It's signed with the hardened runtime, notarized
//!   and stapled, then put in a DMG with an Applications link, which is signed, notarized and
//!   stapled too. Both credentials come from the keychain: the first "Developer ID Application"
//!   identity (or `--sign-id` / `LAUNCHER_SIGN_IDENTITY`), and the `notarytool` profile
//!   [`NOTARY_PROFILE`] (or `--notary-profile` / `LAUNCHER_NOTARY_PROFILE`). `package-macos dev`
//!   (the same as `--no-sign`) skips signing, notarizing and stapling.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use crate::{icons, util};

/// The `xcrun notarytool store-credentials` profile used when none is named. What it holds (an
/// Apple ID, a team, an app-specific password) belongs to the developer account, not to this app:
/// one profile notarizes everything the publisher signs, and this is its name.
const NOTARY_PROFILE: &str = "fire";

/// The licence files installed beside the binary: (source, relative to the workspace root; name
/// installed). The binary links third-party code whose licences require them to travel with it.
const NOTICES: [(&str, &str); 4] = [
    ("LICENSE", "LICENSE.txt"),
    ("THIRD-PARTY-NOTICES.md", "THIRD-PARTY-NOTICES.md"),
    (
        "crates/launcher/assets/fonts/Inter-OFL.txt",
        "licenses/Inter-OFL.txt",
    ),
    (
        "crates/launcher/assets/fonts/JetBrainsMono-OFL.txt",
        "licenses/JetBrainsMono-OFL.txt",
    ),
];

/// A string field of product.json.
fn product(key: &str) -> Result<String, String> {
    let path = util::root().join("product.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let v: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string())
}

fn out_dir() -> PathBuf {
    util::root().join("target").join("package")
}

fn fresh_dir(dir: &Path) -> Result<(), String> {
    let _ = std::fs::remove_dir_all(dir);
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))
}

/// Copy [`NOTICES`] into `dir`, under their installed names.
fn copy_notices(dir: &Path) -> Result<(), String> {
    let root = util::root();
    for (from, to) in NOTICES {
        let (from, to) = (root.join(from), dir.join(to));
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(&from, &to).map_err(|e| format!("{}: {e}", from.display()))?;
    }
    Ok(())
}

/// Inno Setup's compiler: on PATH, or where its installer puts it.
fn iscc() -> Option<PathBuf> {
    let candidates = [
        std::env::var_os("ProgramFiles(x86)")
            .map(|p| PathBuf::from(p).join("Inno Setup 6/ISCC.exe")),
        std::env::var_os("ProgramFiles").map(|p| PathBuf::from(p).join("Inno Setup 6/ISCC.exe")),
        std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Programs/Inno Setup 6/ISCC.exe")),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|p| p.is_file())
        .or_else(|| {
            Command::new("ISCC")
                .arg("/?")
                .output()
                .ok()
                .map(|_| PathBuf::from("ISCC"))
        })
}

pub fn run_windows(args: &[String]) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("package-windows runs on Windows (Inno Setup)".into());
    }
    let iscc = iscc().ok_or(
        "Inno Setup 6.5.2 or later was not found (ISCC.exe); install it from jrsoftware.org",
    )?;
    if !util::flag(args, "--no-build") {
        util::build_release_launcher()?;
    }
    let version = util::value(args, "--version")
        .map(str::to_string)
        .unwrap_or(product("version")?);
    let publisher = product("publisher")?;
    let out = out_dir();
    util::step("wizard images and notices");
    let wizard = out.join("wizard");
    fresh_dir(&wizard)?;
    let (large, small) = icons::write_wizard_images(&icons::Artwork::load()?, &wizard)?;
    let notices = out.join("notices");
    fresh_dir(&notices)?;
    copy_notices(&notices)?;
    util::step("installer");
    let root = util::root();
    util::run(
        Command::new(&iscc)
            .arg("/Qp")
            .arg(format!("/DAppVersion={version}"))
            .arg(format!("/DAppPublisher={publisher}"))
            .arg(format!("/DSourceExe={}", util::launcher_exe().display()))
            .arg(format!("/DSourceIcon={}", root.join(icons::ICO).display()))
            .arg(format!("/DNoticesDir={}", notices.display()))
            .arg(format!("/DWizardImages={large}"))
            .arg(format!("/DWizardSmallImages={small}"))
            .arg(format!("/DOutputDir={}", out.display()))
            .arg(root.join("packaging/windows/launcher.iss")),
    )?;
    let setup = out.join(format!("Launcher-{version}-setup.exe"));
    let size = std::fs::metadata(&setup).map(|m| m.len()).unwrap_or(0);
    println!(
        "{} ({:.2} MB)",
        setup.display(),
        size as f64 / 1024.0 / 1024.0
    );
    Ok(())
}

/// XML text: `&`, `<` and `>` escaped.
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn plist(bundle_id: &str, version: &str, copyright: &str) -> String {
    let (bundle_id, version, copyright) = (xml(bundle_id), xml(version), xml(copyright));
    // The document type is what makes a double-clicked .launcher file reach the app; it arrives
    // as an Apple Event, not in argv (`os/macos.rs` handles `application:openURLs:`). The app owns
    // the extension, so it takes the Owner rank. Files show the app's icon (without one, Finder
    // uses plain text's), and stay plain text underneath, so Quick Look shows what they say.
    // macOS 26+ draws the app's icon from `Assets.car` (`CFBundleIconName`); everything else,
    // documents included, reads the icns (`CFBundleIconFile`).
    let icon_name = icons::MAC_ICON_NAME;
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleIdentifier</key><string>{bundle_id}</string>
    <key>CFBundleName</key><string>Launcher</string>
    <key>CFBundleDisplayName</key><string>Launcher</string>
    <key>CFBundleExecutable</key><string>launcher</string>
    <key>CFBundleIconFile</key><string>launcher</string>
    <key>CFBundleIconName</key><string>{icon_name}</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleShortVersionString</key><string>{version}</string>
    <key>CFBundleVersion</key><string>{version}</string>
    <key>NSHumanReadableCopyright</key><string>{copyright}</string>
    <key>LSApplicationCategoryType</key><string>public.app-category.developer-tools</string>
    <key>LSMinimumSystemVersion</key><string>12.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>CFBundleDocumentTypes</key>
    <array>
        <dict>
            <key>CFBundleTypeName</key><string>Launcher file</string>
            <key>CFBundleTypeRole</key><string>Editor</string>
            <key>LSHandlerRank</key><string>Owner</string>
            <key>LSItemContentTypes</key><array><string>{bundle_id}.file</string></array>
            <key>CFBundleTypeIconFile</key><string>launcher</string>
        </dict>
    </array>
    <key>UTExportedTypeDeclarations</key>
    <array>
        <dict>
            <key>UTTypeIdentifier</key><string>{bundle_id}.file</string>
            <key>UTTypeDescription</key><string>Launcher file</string>
            <key>UTTypeConformsTo</key><array><string>public.plain-text</string></array>
            <key>UTTypeIconFile</key><string>launcher</string>
            <key>UTTypeTagSpecification</key>
            <dict><key>public.filename-extension</key><array><string>launcher</string></array></dict>
        </dict>
    </array>
</dict>
</plist>
"#
    )
}

/// A non-empty environment variable.
fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// The codesign identity: `--sign-id`, `LAUNCHER_SIGN_IDENTITY`, or the keychain's first
/// "Developer ID Application" one. Only that kind makes something another Mac opens: an "Apple
/// Development" identity signs fine and is refused everywhere else, so it is never picked.
fn sign_identity(args: &[String]) -> Result<String, String> {
    if let Some(id) = util::value(args, "--sign-id")
        .map(str::to_string)
        .or_else(|| env("LAUNCHER_SIGN_IDENTITY"))
    {
        return Ok(id);
    }
    let out = Command::new("security")
        .args(["find-identity", "-v", "-p", "codesigning"])
        .output()
        .map_err(|e| format!("could not start `security`: {e}"))?;
    developer_id(&String::from_utf8_lossy(&out.stdout)).ok_or_else(|| {
        "no \"Developer ID Application\" identity in the keychain: install one from \
         developer.apple.com, pass --sign-id, or pass --no-sign for a build that runs only on \
         this Mac"
            .into()
    })
}

/// The first "Developer ID Application: ..." identity in `security find-identity` output.
fn developer_id(listing: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let start = line.find("\"Developer ID Application: ")? + 1;
        let len = line[start..].find('"')?;
        Some(line[start..start + len].to_string())
    })
}

/// Run `cmd`, echoing its output as it comes; returns its status and everything it printed.
fn run_echoed(cmd: &mut Command) -> Result<(ExitStatus, String), String> {
    cmd.current_dir(util::root());
    let shown = format!("{cmd:?}").replace('"', "");
    eprintln!("  $ {shown}");
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start `{shown}`: {e}"))?;
    let echo = |stream: Box<dyn std::io::Read + Send>| {
        let mut all = String::new();
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            eprintln!("{line}");
            all.push_str(&line);
            all.push('\n');
        }
        all
    };
    let stderr: Box<dyn std::io::Read + Send> = Box::new(child.stderr.take().expect("piped"));
    let errors = std::thread::spawn(move || echo(stderr));
    let mut all = echo(Box::new(child.stdout.take().expect("piped")));
    let status = child.wait().map_err(|e| format!("`{shown}`: {e}"))?;
    all.push_str(&errors.join().unwrap_or_default());
    Ok((status, all))
}

/// The submission id and whether Apple accepted it, from `notarytool submit --wait` output.
fn notary_verdict(output: &str) -> (Option<String>, bool) {
    let last = |name: &str| {
        output
            .lines()
            .filter_map(|l| l.trim().strip_prefix(name))
            .map(|v| v.trim().to_string())
            .next_back()
    };
    (last("id:"), last("status:").as_deref() == Some("Accepted"))
}

/// Notarize `path` (a zip or a DMG) and wait for Apple's verdict, read from the output rather than
/// the exit status alone. A rejection names the command that says why.
fn notarize(path: &Path, profile: &str) -> Result<(), String> {
    let (status, output) = run_echoed(
        Command::new("xcrun")
            .args(["notarytool", "submit"])
            .arg(path)
            .args(["--keychain-profile", profile, "--wait"]),
    )?;
    match notary_verdict(&output) {
        (_, true) if status.success() => Ok(()),
        (Some(id), _) => Err(format!(
            "Apple did not accept {}; its reasons: xcrun notarytool log {id} --keychain-profile {profile}",
            path.display()
        )),
        (None, _) => Err(format!(
            "notarization failed before submitting; is the keychain profile \"{profile}\" set up? \
             xcrun notarytool store-credentials {profile} --apple-id <id> --team-id <team> \
             --password <app-specific password>"
        )),
    }
}

pub fn run_macos(args: &[String]) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err(
            "package-macos runs on macOS (actool, ictool, codesign, iconutil, hdiutil, notarytool)"
                .into(),
        );
    }
    let bundle_id = product("bundleId")?;
    if bundle_id.is_empty() {
        return Err(
            "product.json needs \"bundleId\": the reverse DNS of a domain the publisher owns (com.example.launcher). It must never change once shipped: macOS keeps permissions by it.".into(),
        );
    }
    let version = product("version")?;
    let copyright = product("copyright")?;
    // `dev`: the app and the DMG for this Mac, with no credentials needed.
    let identity = if util::flag(args, "dev") || util::flag(args, "--no-sign") {
        None
    } else {
        Some(sign_identity(args)?)
    };
    // Notarization checks a Developer ID signature, so an unsigned build skips it.
    let notary = (identity.is_some() && !util::flag(args, "--no-notarize")).then(|| {
        util::value(args, "--notary-profile")
            .map(str::to_string)
            .or_else(|| env("LAUNCHER_NOTARY_PROFILE"))
            .unwrap_or_else(|| NOTARY_PROFILE.to_string())
    });
    let root = util::root();
    let target = "aarch64-apple-darwin";
    if !util::flag(args, "--no-build") {
        util::step("release build");
        util::run(util::cargo().args([
            "build",
            "--release",
            "-p",
            "launcher",
            "--target",
            target,
        ]))?;
    }
    let exe = root.join("target").join(target).join("release/launcher");
    if !exe.is_file() {
        return Err(format!("{} is missing; drop --no-build", exe.display()));
    }

    util::step("Launcher.app");
    let out = out_dir();
    // Scratch: the iconset, the zip sent to Apple, the DMG's contents.
    let work = out.join("macos");
    fresh_dir(&work)?;
    let app = out.join("Launcher.app");
    let contents = app.join("Contents");
    let resources = contents.join("Resources");
    fresh_dir(&app)?;
    for d in [contents.join("MacOS"), resources.clone()] {
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    std::fs::copy(&exe, contents.join("MacOS/launcher"))
        .map_err(|e| format!("{}: {e}", exe.display()))?;
    let info = contents.join("Info.plist");
    std::fs::write(&info, plist(&bundle_id, &version, &copyright)).map_err(|e| e.to_string())?;
    util::run(Command::new("plutil").arg("-lint").arg(&info))?;
    std::fs::write(contents.join("PkgInfo"), "APPL????").map_err(|e| e.to_string())?;
    let car = icons::compile_assets(&work.join("assets"))?;
    std::fs::copy(&car, resources.join("Assets.car"))
        .map_err(|e| format!("{}: {e}", car.display()))?;
    let iconset = work.join("launcher.iconset");
    icons::write_iconset(&iconset)?;
    util::run(
        Command::new("iconutil")
            .args(["-c", "icns"])
            .arg(&iconset)
            .arg("-o")
            .arg(resources.join("launcher.icns")),
    )?;
    copy_notices(&resources)?;

    if let Some(id) = &identity {
        util::step(&format!("codesign ({id})"));
        // The hardened runtime and a secure timestamp: notarization requires both, and the
        // timestamp keeps the signature valid after the certificate expires. No entitlements.
        util::run(
            Command::new("codesign")
                .args(["--force", "--options", "runtime", "--timestamp", "--sign"])
                .arg(id)
                .arg(&app),
        )?;
        util::run(
            Command::new("codesign")
                .args(["--verify", "--deep", "--strict", "--verbose=2"])
                .arg(&app),
        )?;
    }
    if let Some(profile) = &notary {
        // The app gets its own ticket as well as the DMG: a stapled app opens on a Mac that
        // can't reach Apple, where an unstapled one reads as "damaged".
        util::step(&format!("notarize Launcher.app (profile \"{profile}\")"));
        let zip = work.join("Launcher.zip");
        // ditto, not zip: it keeps the bundle's extended attributes, which notarytool checks.
        util::run(
            Command::new("ditto")
                .args(["-c", "-k", "--keepParent"])
                .arg(&app)
                .arg(&zip),
        )?;
        notarize(&zip, profile)?;
        util::run(Command::new("xcrun").args(["stapler", "staple"]).arg(&app))?;
        util::run(
            Command::new("xcrun")
                .args(["stapler", "validate"])
                .arg(&app),
        )?;
    }
    if util::flag(args, "--no-dmg") {
        println!("{}", app.display());
        return Ok(());
    }

    util::step("DMG");
    let stage = work.join("dmg");
    fresh_dir(&stage)?;
    // ditto again: `cp -R` can drop the extended attributes and break the signature.
    util::run(
        Command::new("ditto")
            .arg(&app)
            .arg(stage.join("Launcher.app")),
    )?;
    // The drag-to-install target.
    util::run(
        Command::new("ln")
            .args(["-s", "/Applications"])
            .arg(stage.join("Applications")),
    )?;
    let dmg = out.join(format!("Launcher-{version}.dmg"));
    util::run(
        Command::new("hdiutil")
            .args(["create", "-volname", &format!("Launcher {version}")])
            .arg("-srcfolder")
            .arg(&stage)
            .args(["-ov", "-format", "UDZO"])
            .arg(&dmg),
    )?;
    if let Some(id) = &identity {
        util::run(
            Command::new("codesign")
                .args(["--force", "--timestamp", "--sign"])
                .arg(id)
                .arg(&dmg),
        )?;
        util::run(
            Command::new("codesign")
                .args(["--verify", "--verbose=2"])
                .arg(&dmg),
        )?;
    }
    if let Some(profile) = &notary {
        util::step("notarize the DMG");
        notarize(&dmg, profile)?;
        util::run(Command::new("xcrun").args(["stapler", "staple"]).arg(&dmg))?;
        util::run(
            Command::new("xcrun")
                .args(["stapler", "validate"])
                .arg(&dmg),
        )?;
        // What Gatekeeper asks on another Mac: a pass means the download opens with a
        // double-click.
        util::step("Gatekeeper");
        util::run(
            Command::new("spctl")
                .args([
                    "-a",
                    "-t",
                    "open",
                    "--context",
                    "context:primary-signature",
                    "-vv",
                ])
                .arg(&dmg),
        )?;
    }
    println!("{}", dmg.display());
    if notary.is_none() {
        eprintln!(
            "not notarized: another Mac opens it only through System Settings > Privacy & Security > Open Anyway{}",
            if identity.is_none() {
                ", if at all (unsigned)"
            } else {
                ""
            }
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plist_declares_the_document_type() {
        let p = plist("com.example.launcher", "1.2.3", "Copyright (c) A & B");
        assert!(p.contains("<string>com.example.launcher.file</string>"));
        assert!(p.contains("<string>public.plain-text</string>"));
        assert!(p.contains("<key>CFBundleTypeIconFile</key><string>launcher</string>"));
        assert!(p.contains("<key>CFBundleIconName</key><string>launcher-app</string>"));
        assert!(p.contains("<key>UTTypeIconFile</key><string>launcher</string>"));
        assert!(p.contains("<string>launcher</string></array>"));
        assert!(p.contains("<key>CFBundleShortVersionString</key><string>1.2.3</string>"));
        assert!(p.contains("<string>Copyright (c) A &amp; B</string>"));
    }

    #[test]
    fn the_developer_id_identity_is_picked() {
        let listing = r#"  1) 3A1B2C3D4E5F60718293A4B5C6D7E8F901234567 "Apple Development: Chandan Singh (ABCDE12345)"
  2) 9F2C3D4E5F60718293A4B5C6D7E8F90123456789 "Developer ID Application: Chandan Singh (ABCDE12345)"
     2 valid identities found
"#;
        assert_eq!(
            developer_id(listing).as_deref(),
            Some("Developer ID Application: Chandan Singh (ABCDE12345)")
        );
        assert_eq!(developer_id("     0 valid identities found\n"), None);
    }

    #[test]
    fn the_notary_verdict_is_read_from_the_output() {
        let accepted = "Submission ID received\n  id: 2efe2717-52ef-43a5-96dc-0797e4ca1041\n\
            Waiting for processing to complete.\nCurrent status: Accepted....\n\
            Processing complete\n  id: 2efe2717-52ef-43a5-96dc-0797e4ca1041\n  status: Accepted\n";
        assert_eq!(
            notary_verdict(accepted),
            (Some("2efe2717-52ef-43a5-96dc-0797e4ca1041".into()), true)
        );
        let invalid = accepted.replace("status: Accepted", "status: Invalid");
        assert!(!notary_verdict(&invalid).1);
        assert_eq!(
            notary_verdict("Error: No Keychain password item found\n"),
            (None, false)
        );
    }

    #[test]
    fn every_notice_exists() {
        for (from, _) in NOTICES {
            assert!(util::root().join(from).is_file(), "{from} is missing");
        }
    }
}
