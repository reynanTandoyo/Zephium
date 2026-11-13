//! Finds an installed app by its command name, the way a terminal would,
//! plus the folders each app's own installer uses. Reads metadata only.

use std::path::{Path, PathBuf};

use super::CliApp;

/// The folders `app`'s official installer puts its command in, beyond PATH.
pub(crate) fn extra_folders(app: CliApp) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    let home =
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from);
    if let Some(home) = &home {
        folders.push(home.join(".local").join("bin"));
        if app == CliApp::OpenCode {
            folders.push(home.join(".opencode").join("bin"));
        }
    }
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            if app == CliApp::Antigravity {
                folders.push(local.join("agy").join("bin"));
            }
            folders.push(local.join("Microsoft").join("WinGet").join("Links"));
        }
        if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
            folders.push(roaming.join("npm"));
        }
    }
    #[cfg(unix)]
    folders.extend(
        ["/opt/homebrew/bin", "/usr/local/bin"]
            .into_iter()
            .map(PathBuf::from),
    );
    folders
}

#[cfg(windows)]
fn names(program: &str) -> Vec<String> {
    let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
    let mut names: Vec<String> = pathext
        .split(';')
        .map(str::trim)
        .filter(|extension| {
            [".exe", ".com", ".cmd"]
                .iter()
                .any(|supported| extension.eq_ignore_ascii_case(supported))
        })
        .map(|extension| format!("{program}{}", extension.to_ascii_lowercase()))
        .collect();
    names.dedup();
    names
}

#[cfg(not(windows))]
fn names(program: &str) -> Vec<String> {
    vec![program.to_owned()]
}

#[cfg(windows)]
fn executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// The first absolute `program` on PATH, then in `extra`.
pub(crate) fn locate(program: &str, extra: &[PathBuf]) -> Option<PathBuf> {
    let mut folders: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    folders.extend(extra.iter().cloned());
    // A relative or empty PATH entry would resolve against the current folder.
    folders.retain(|folder| folder.is_absolute());
    let names = names(program);
    folders
        .iter()
        .flat_map(|folder| names.iter().map(move |name| folder.join(name)))
        .find(|path| executable(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_programs_only_in_absolute_folders() {
        let root = tempfile::tempdir().unwrap();
        let name = if cfg!(windows) {
            "fakeapp.exe"
        } else {
            "fakeapp"
        };
        let program = root.path().join(name);
        std::fs::write(&program, b"").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        assert_eq!(
            locate("fakeapp", &[root.path().to_path_buf()]),
            Some(program)
        );
        assert_eq!(locate("fakeapp", &[PathBuf::from("relative")]), None);
        assert_eq!(locate("missing-app-zz", &[root.path().to_path_buf()]), None);
    }
}
