//! One bounded run of an app: an absolute program, an explicit environment,
//! no console window, stdin written then closed, stdout capped, a deadline,
//! and the whole process tree ended when the run is dropped or times out.

use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[cfg(windows)]
#[path = "spawn_windows.rs"]
mod windows;

const MAX_STDERR_BYTES: usize = 64 * 1024;
const MAX_ARGS: usize = 64;

/// The variables an app needs to find its own sign-in, settings and network,
/// copied from Zephium's environment when set. Nothing else is passed: API
/// keys in the environment would switch an app from the subscription to
/// billed API use.
#[cfg(windows)]
const INHERITED: &[&str] = &[
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "PATHEXT",
    "TEMP",
    "TMP",
    "HOMEDRIVE",
    "HOMEPATH",
    "USERNAME",
    "USERDOMAIN",
    "COMPUTERNAME",
    "ProgramData",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "CommonProgramFiles",
    "NUMBER_OF_PROCESSORS",
    "PROCESSOR_ARCHITECTURE",
    "OS",
    "ComSpec",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
];
#[cfg(not(windows))]
const INHERITED: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "LANG",
    "LC_ALL",
    "XDG_CONFIG_HOME",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "XDG_CACHE_HOME",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
];

pub(crate) struct Run<'a> {
    /// Absolute.
    pub program: &'a Path,
    pub args: Vec<OsString>,
    /// A private folder the app may treat as its workspace.
    pub cwd: &'a Path,
    /// Written whole, then closed. `None` gives the app no stdin at all.
    pub stdin: Option<Vec<u8>>,
    pub timeout: Duration,
    pub max_stdout: usize,
    /// Fixed variables for this app, on top of [`INHERITED`].
    pub extra_env: &'a [(&'a str, &'a str)],
}

#[derive(Debug)]
pub(crate) struct Output {
    pub success: bool,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SpawnError {
    /// The program could not be started or confined.
    Spawn,
    /// The run passed its deadline and was ended.
    Timeout,
    /// The app printed more than the run allows and was ended.
    TooLarge,
    /// Reading its output failed.
    Io,
}

/// The environment a run gets: the inherited allowlist, PATH with the
/// program's own folder first, and the app's fixed variables.
pub(crate) fn environment(program: &Path, extra: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = INHERITED
        .iter()
        .filter_map(|name| std::env::var_os(name).map(|value| (OsString::from(name), value)))
        .collect();
    let mut folders: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    folders.retain(|folder| folder.is_absolute());
    if let Some(folder) = program.parent() {
        folders.insert(0, folder.to_path_buf());
    }
    if let Ok(path) = std::env::join_paths(folders) {
        env.push(("PATH".into(), path));
    }
    env.push(("NO_COLOR".into(), "1".into()));
    env.extend(
        extra
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value))),
    );
    env
}

/// Ends the run's whole process tree when dropped.
struct Tree {
    child: tokio::process::Child,
    #[cfg(windows)]
    job: windows::Job,
    /// Cleared once the app is reaped, when its group id may be reused.
    #[cfg(unix)]
    group: Option<i32>,
}

impl Drop for Tree {
    fn drop(&mut self) {
        #[cfg(windows)]
        self.job.stop();
        #[cfg(unix)]
        if let Some(group) = self.group {
            // SAFETY: a negative pid names only the process group this run
            // created for its own child.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
        let _ = self.child.start_kill();
    }
}

pub(crate) async fn run(run: Run<'_>) -> Result<Output, SpawnError> {
    if !run.program.is_absolute() || !run.cwd.is_absolute() || run.args.len() > MAX_ARGS {
        return Err(SpawnError::Spawn);
    }
    let mut command = tokio::process::Command::new(run.program);
    command
        .args(&run.args)
        .env_clear()
        .envs(environment(run.program, run.extra_env))
        .current_dir(run.cwd)
        .stdin(if run.stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // No console window; suspended until the job admits the whole tree.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000 | 0x0000_0004);
    #[cfg(windows)]
    let job = windows::Job::new().map_err(|()| SpawnError::Spawn)?;
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn().map_err(|_| SpawnError::Spawn)?;
    #[cfg(windows)]
    let mut tree = {
        let assigned = job.assign_and_resume(&child);
        let tree = Tree { child, job };
        assigned.map_err(|()| SpawnError::Spawn)?;
        tree
    };
    #[cfg(unix)]
    let mut tree = {
        let group = child.id().and_then(|id| i32::try_from(id).ok());
        Tree { child, group }
    };
    #[cfg(not(any(windows, unix)))]
    let mut tree = Tree { child };
    let stdin = tree.child.stdin.take();
    let mut stdout = tree.child.stdout.take().ok_or(SpawnError::Spawn)?;
    let mut stderr = tree.child.stderr.take().ok_or(SpawnError::Spawn)?;
    let max_stdout = run.max_stdout;
    let input = run.stdin;
    let work = async {
        let write = async {
            if let (Some(mut pipe), Some(bytes)) = (stdin, input) {
                // An app that exits without reading its input is not an error here.
                let _ = pipe.write_all(&bytes).await;
                let _ = pipe.shutdown().await;
            }
            Ok::<(), SpawnError>(())
        };
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut stdout = (&mut stdout).take(max_stdout as u64 + 1);
        let mut stderr = (&mut stderr).take(MAX_STDERR_BYTES as u64);
        // Too much output ends the run at once, without waiting for stderr.
        let read_out = async {
            stdout
                .read_to_end(&mut out)
                .await
                .map_err(|_| SpawnError::Io)?;
            if out.len() > max_stdout {
                return Err(SpawnError::TooLarge);
            }
            Ok(())
        };
        // Stderr past its cap is drained, so a chatty app never blocks on it.
        let read_err = async {
            stderr
                .read_to_end(&mut err)
                .await
                .map_err(|_| SpawnError::Io)?;
            tokio::io::copy(stderr.get_mut(), &mut tokio::io::sink())
                .await
                .map(drop)
                .map_err(|_| SpawnError::Io)
        };
        tokio::try_join!(write, read_out, read_err)?;
        let status = tree.child.wait().await.map_err(|_| SpawnError::Io)?;
        #[cfg(unix)]
        {
            tree.group = None;
        }
        Ok(Output {
            success: status.success(),
            stdout: out,
            stderr: err,
        })
    };
    let result = tokio::time::timeout(run.timeout, work)
        .await
        .map_err(|_| SpawnError::Timeout)?;
    drop(tree);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_is_an_allowlist_with_the_program_folder_first() {
        let program = if cfg!(windows) {
            Path::new(r"C:\apps\claude\claude.exe")
        } else {
            Path::new("/opt/apps/claude")
        };
        let env = environment(program, &[("APP_FIXED", "1")]);
        let names: Vec<String> = env
            .iter()
            .map(|(name, _)| name.to_string_lossy().into_owned())
            .collect();
        for name in &names {
            assert!(
                INHERITED.contains(&name.as_str())
                    || ["PATH", "NO_COLOR", "APP_FIXED"].contains(&name.as_str()),
                "{name}"
            );
        }
        for secret in ["ANTHROPIC_API_KEY", "OPENAI_API_KEY", "GEMINI_API_KEY"] {
            assert!(!names.iter().any(|name| name == secret));
        }
        let path = &env.iter().find(|(name, _)| name == "PATH").unwrap().1;
        let first = std::env::split_paths(path).next().unwrap();
        assert_eq!(first, program.parent().unwrap());
    }
}
