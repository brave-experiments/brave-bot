//! Working out which binary a program name means.
//!
//! `bua_core::programs::TrustedPrograms` is keyed by resolved path rather than by the name a
//! planner typed, and the reason is the one `bua_core::pure` states: a name is not a program.
//! `$PATH` decides what `grep` means, and on the machine this was developed against it means
//! `ugrep`, a different implementation with a far larger option surface. An approval recorded
//! against the string would follow the name onto whatever it later pointed at.
//!
//! So resolution happens once, before the person is asked. What they are shown is the resolved
//! path, what the list records is the resolved path, and what is executed is the resolved path.
//! Those being the same value is the point: resolving again after the approval would leave a
//! window in which `$PATH` changed and something else ran.
//!
//! This crate does the looking up because `bua-core` performs no I/O.

use std::path::{Path, PathBuf};

/// Work out which file `program` names, or `None` if nothing usable was found.
///
/// A name with no separator in it is looked up in `$PATH`. A name with one is taken as a path,
/// relative to `working` rather than to this process's own directory: the stage is going to run in
/// `working`, so that is the directory `./script.sh` means to whoever wrote it.
///
/// Returns an absolute path, so what is recorded and what runs cannot differ from what was shown.
pub fn resolve(program: &str, working: &Path) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }

    if has_separator(program) {
        let candidate = if Path::new(program).is_absolute() {
            PathBuf::from(program)
        } else {
            working.join(program)
        };
        return usable(&candidate);
    }

    for directory in std::env::split_paths(&std::env::var_os("PATH")?) {
        // An empty entry in $PATH means the current directory, which is a long-standing footgun:
        // it would let a file in the workspace shadow a system program. Skipped rather than
        // honoured, since nothing here needs it and the surprise is all downside.
        if directory.as_os_str().is_empty() {
            continue;
        }
        for name in candidates(program) {
            if let Some(found) = usable(&directory.join(&name)) {
                return Some(found);
            }
        }
    }
    None
}

/// Whether the name is a path rather than something to look up.
fn has_separator(program: &str) -> bool {
    program.contains('/') || (cfg!(windows) && program.contains('\\'))
}

/// The filenames to try for one program name.
///
/// One on unix. On Windows a bare name may mean any of the extensions in `%PATHEXT%`, and the name
/// as given is tried first so an extension already written out is not doubled.
fn candidates(program: &str) -> Vec<String> {
    if !cfg!(windows) {
        return vec![program.to_string()];
    }
    let mut names = vec![program.to_string()];
    if let Some(pathext) = std::env::var_os("PATHEXT") {
        for extension in pathext.to_string_lossy().split(';') {
            let extension = extension.trim();
            if !extension.is_empty() {
                names.push(format!("{program}{extension}"));
            }
        }
    }
    names
}

/// The absolute path of `candidate` if it is a file this user could execute.
///
/// Canonicalised, so a path reached through `..` or a symlink is recorded as the file it actually
/// is. Two names for one binary would otherwise be two entries in the list, and vouching for one
/// would leave the other still asking.
fn usable(candidate: &Path) -> Option<PathBuf> {
    let resolved = candidate.canonicalize().ok()?;
    if !resolved.is_file() {
        return None;
    }
    if !executable(&resolved) {
        return None;
    }
    Some(resolved)
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Windows has no executable bit; being a file with a known extension is as far as it goes.
#[cfg(not(unix))]
fn executable(_path: &Path) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ordinary case: a bare name is found on `$PATH`, as an absolute path.
    #[test]
    fn a_program_on_the_path_resolves_to_an_absolute_file() {
        let found = resolve("sh", Path::new("/")).expect("sh is on the path");
        assert!(found.is_absolute());
        assert!(found.is_file());
    }

    #[test]
    fn a_program_that_is_not_installed_resolves_to_nothing() {
        assert!(resolve("bua-no-such-program-anywhere", Path::new("/")).is_none());
    }

    #[test]
    fn an_empty_name_resolves_to_nothing() {
        assert!(resolve("", Path::new("/")).is_none());
    }

    /// A name with a separator is a path, and it means what it means from the directory the stage
    /// will run in rather than from wherever this process happens to be.
    #[test]
    fn a_relative_path_resolves_against_the_working_directory() {
        let scratch = std::env::temp_dir().join("bua-programs-relative");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let script = scratch.join("tool.sh");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let found = resolve("./tool.sh", &scratch).expect("resolves in the working directory");
        assert_eq!(found, script.canonicalize().unwrap());
        assert!(
            resolve("./tool.sh", Path::new("/")).is_none(),
            "a relative path resolved against the wrong directory"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// A file that cannot be executed is not a program, so it is not something to record an
    /// approval against.
    #[cfg(unix)]
    #[test]
    fn a_file_without_the_executable_bit_is_not_a_program() {
        let scratch = std::env::temp_dir().join("bua-programs-notexec");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let plain = scratch.join("notes.txt");
        std::fs::write(&plain, "not a program").unwrap();

        assert!(resolve("./notes.txt", &scratch).is_none());
        let _ = std::fs::remove_dir_all(&scratch);
    }

    /// Two names for one binary must record as one entry, or vouching for a program would leave
    /// another spelling of it still asking.
    #[test]
    fn a_path_through_a_traversal_resolves_to_the_same_file() {
        let direct = resolve("sh", Path::new("/")).expect("sh is on the path");
        let parent = direct.parent().expect("sh has a directory");
        let roundabout = parent.join("..").join(
            parent
                .file_name()
                .expect("the directory has a name")
                .to_string_lossy()
                .to_string(),
        );
        let through = resolve(
            roundabout.join("sh").to_string_lossy().as_ref(),
            Path::new("/"),
        )
        .expect("the same file by a longer road");
        assert_eq!(direct, through);
    }
}
