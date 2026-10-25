//! Environments that come from scripts:
//!
//! * `env-from` (N5): run a setup script (vcvarsall.bat, a conda activate script...) and keep what
//!   it changed. The script runs in its own shell with the run's environment, the shell prints the
//!   result, and only the differences from a plain run of the same shell are applied, so the
//!   shell's own variables don't leak in.
//! * The macOS login shell (N14): an app started from Finder doesn't get the PATH that `.zshrc`
//!   sets up, so the launcher captures the login shell's environment once at startup.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use super::child::{self, Io, Spawn};

/// What a script changed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EnvChange {
    pub set: Vec<(String, String)>,
    pub removed: Vec<String>,
}

impl EnvChange {
    pub fn apply(&self, env: &mut BTreeMap<String, String>) {
        for k in &self.removed {
            env.remove(k);
        }
        for (k, v) in &self.set {
            // Windows names are case-insensitive: replace whatever spelling was there.
            if cfg!(windows) {
                let old: Vec<String> = env
                    .keys()
                    .filter(|e| e.eq_ignore_ascii_case(k))
                    .cloned()
                    .collect();
                for o in old {
                    env.remove(&o);
                }
            }
            env.insert(k.clone(), v.clone());
        }
    }
}

/// Run a program with pipes and return its output (stdout and stderr together).
fn output(
    program: &Path,
    args: Vec<String>,
    raw: Option<String>,
    cwd: &Path,
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<(i32, String), String> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let c = child::spawn(
        &Spawn {
            program: program.to_path_buf(),
            args,
            raw_command_line: raw,
            cwd: cwd.to_path_buf(),
            env: env
                .iter()
                .map(|(k, v)| (OsString::from(k), OsString::from(v)))
                .collect(),
            stdin: None,
            io: Io::Pipes,
        },
        Box::new(move |b| {
            let _ = tx.send(b.to_vec());
        }),
    )
    .map_err(|e| format!("could not start {}: {e}", program.display()))?;
    let Some(code) = c.wait_timeout(timeout) else {
        c.kill();
        return Err(format!(
            "{} didn't finish within {} s",
            program.display(),
            timeout.as_secs()
        ));
    };
    let mut bytes = Vec::new();
    while let Ok(chunk) = rx.recv_timeout(Duration::from_secs(5)) {
        if chunk.is_empty() {
            break;
        }
        bytes.extend(chunk);
    }
    Ok((code, String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(windows)]
fn parse_lines(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (k, v) = l.split_once('=')?;
            // `=C:=C:\x` entries are cmd's per-drive folders, not variables.
            (!k.is_empty()).then(|| (k.to_string(), v.to_string()))
        })
        .collect()
}

fn parse_nul(text: &str) -> BTreeMap<String, String> {
    text.split('\0')
        .filter_map(|e| {
            let (k, v) = e.split_once('=')?;
            (!k.is_empty()).then(|| (k.trim_start_matches('\n').to_string(), v.to_string()))
        })
        .collect()
}

fn diff(before: &BTreeMap<String, String>, after: &BTreeMap<String, String>) -> EnvChange {
    let find = |m: &BTreeMap<String, String>, k: &str| -> Option<String> {
        m.get(k).cloned().or_else(|| {
            cfg!(windows)
                .then(|| {
                    m.iter()
                        .find(|(n, _)| n.eq_ignore_ascii_case(k))
                        .map(|(_, v)| v.clone())
                })
                .flatten()
        })
    };
    EnvChange {
        set: after
            .iter()
            .filter(|(k, v)| find(before, k).as_ref() != Some(v))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        removed: before
            .keys()
            .filter(|k| find(after, k).is_none())
            .cloned()
            .collect(),
    }
}

/// Run `script` with `args` and report what it did to the environment.
pub fn env_from(
    script: &Path,
    args: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    timeout: Duration,
) -> Result<EnvChange, String> {
    #[cfg(windows)]
    {
        let cmd: PathBuf = std::env::var_os("ComSpec")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\Windows\System32\cmd.exe"));
        let unsafe_arg = args
            .iter()
            .find(|a| a.contains(['"', '%', '^', '&', '|', '<', '>', '!', '\r', '\n']));
        if let Some(a) = unsafe_arg {
            return Err(format!(
                "the argument {a:?} can't be passed to a setup script: it holds a character cmd.exe would act on"
            ));
        }
        let mut quoted_args = String::new();
        for a in args {
            quoted_args.push(' ');
            super::process::quote_windows_arg(a, &mut quoted_args);
        }
        let (_, base) = output(&cmd, vec![], Some("/d /c set".into()), cwd, env, timeout)?;
        let line = format!(
            "/d /s /c \"call \"{}\"{quoted_args} >nul 2>&1 && set\"",
            script.display()
        );
        let (code, after) = output(&cmd, vec![], Some(line), cwd, env, timeout)?;
        if code != 0 {
            return Err(format!("{} failed (exit code {code})", script.display()));
        }
        Ok(diff(&parse_lines(&base), &parse_lines(&after)))
    }
    #[cfg(not(windows))]
    {
        // The baseline is the same shell doing nothing: what zsh sets up on its own (PATH from
        // ~/.zshenv, PWD, OLDPWD, SHLVL) isn't the script's doing.
        let zsh = PathBuf::from("/bin/zsh");
        let (_, base) = output(
            &zsh,
            vec!["-c".into(), "/usr/bin/env -0".into()],
            None,
            cwd,
            env,
            timeout,
        )?;
        let mut a = vec![
            "-c".to_string(),
            "source \"$0\" \"$@\" >/dev/null 2>&1 && /usr/bin/env -0".to_string(),
            script.display().to_string(),
        ];
        a.extend(args.iter().cloned());
        let (code, after) = output(&zsh, a, None, cwd, env, timeout)?;
        if code != 0 {
            return Err(format!("{} failed (exit code {code})", script.display()));
        }
        Ok(diff(&parse_nul(&base), &parse_nul(&after)))
    }
}

/// macOS: the login shell's environment (PATH from `.zshrc` and friends), or None when the shell
/// doesn't answer within 3 s.
pub fn login_shell_env() -> Option<BTreeMap<String, String>> {
    if cfg!(windows) {
        return None;
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    const MARK: &str = "__LAUNCHER_ENV__";
    let env: BTreeMap<String, String> = std::env::vars().collect();
    let (_, out) = output(
        Path::new(&shell),
        vec![
            "-l".into(),
            "-i".into(),
            "-c".into(),
            format!("printf '\\n{MARK}\\n'; /usr/bin/env -0"),
        ],
        None,
        Path::new("/"),
        &env,
        Duration::from_secs(3),
    )
    .ok()?;
    let after = out.split(&format!("{MARK}\n")).nth(1)?;
    let parsed = parse_nul(after);
    (!parsed.is_empty()).then_some(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_setup_script_changes_the_environment() {
        let dir = std::env::temp_dir().join(format!("launcher-envfrom-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (script, body) = if cfg!(windows) {
            (
                dir.join("setup.bat"),
                "@echo off\r\nset LAUNCHER_TEST_A=from script %1\r\nset LAUNCHER_TEST_GONE=\r\n",
            )
        } else {
            (
                dir.join("setup.sh"),
                "export LAUNCHER_TEST_A=\"from script $1\"\nunset LAUNCHER_TEST_GONE\n",
            )
        };
        std::fs::write(&script, body).unwrap();
        let mut env: BTreeMap<String, String> = std::env::vars().collect();
        env.insert("LAUNCHER_TEST_GONE".into(), "x".into());
        let change = env_from(
            &script,
            &["x64".into()],
            &dir,
            &env,
            Duration::from_secs(20),
        )
        .unwrap();
        assert!(
            change
                .set
                .contains(&("LAUNCHER_TEST_A".to_string(), "from script x64".to_string())),
            "{change:?}"
        );
        assert!(
            change.removed.contains(&"LAUNCHER_TEST_GONE".to_string()),
            "{change:?}"
        );
        // The shell's own bookkeeping doesn't count as a change: on macOS, not even the PATH a
        // ~/.zshenv sets up, or PWD and OLDPWD.
        if cfg!(windows) {
            assert!(change.set.len() <= 3, "{change:?}");
        } else {
            assert_eq!(change.set.len(), 1, "{change:?}");
        }
        change.apply(&mut env);
        assert_eq!(env["LAUNCHER_TEST_A"], "from script x64");
        assert!(!env.contains_key("LAUNCHER_TEST_GONE"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
