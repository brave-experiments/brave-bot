//! Where the user's own files live.
//!
//! `~/.bravebot` holds what should outlive a session: prompt history, session records, and now
//! standing instructions and skills. One definition of where that is, so the interface and the
//! agent cannot drift apart about it.
//!
//! There is deliberately no fallback. A missing `HOME` yields `None` and every caller does
//! without, because inventing a directory would mean reading files from somewhere the user never
//! chose, and this is the one place whose contents are trusted for being the user's own.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The name of the directory inside the user's home.
const DIRECTORY: &str = ".bravebot";

/// The user's own directory, or `None` when there is no home to put it in.
///
/// This is the reading answer. Anything about to write wants [`writable`] instead.
pub fn directory() -> Option<PathBuf> {
    // Read directly rather than taking a dependency for one variable. Absent in some daemon and
    // container environments, which is a case that has to be handled anyway.
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    Some(Path::new(&home).join(DIRECTORY))
}

/// The user's own directory when something may be written into it, or `None` when nothing may be.
///
/// `None` for two different reasons that callers should treat the same way: there is no home to
/// write to, or the session is [incognito] and is not going to write to it. Both mean "do not
/// persist this", and every caller already handled the first, which is what makes the second cost
/// a line rather than a redesign.
///
/// # Why a second function rather than a flag inside the first
///
/// Reads must keep working in an incognito session: the chosen model, the theme, the standing
/// instructions and the credentials all come out of this directory, and a session that could not
/// read them would not be private but crippled. Making [`directory`] itself answer `None` would
/// have taken those away too. Splitting the question in two puts the mode exactly where it belongs,
/// on the writes, and makes each call site say which it is doing.
///
/// [incognito]: bravebot_core::incognito
pub fn writable() -> Option<PathBuf> {
    if bravebot_core::incognito::engaged() {
        return None;
    }
    directory()
}

/// Create `path` and everything between it and the state directory, reachable only by this user.
///
/// Every writer under `~/.bravebot` goes through this rather than `create_dir_all`, because a
/// directory keeps the mode it was made with and the state directory is made by whichever
/// subsystem happens to write first. Creating one at `0700` while another creates it at the
/// umask makes the mode of a directory holding prompt history a matter of call order.
///
/// An existing directory is tightened rather than left as it was found, which is what makes this
/// fix a machine that has already run an older build.
pub fn create_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
        if let Some(root) = directory() {
            tighten(&root, path);
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::create_dir_all(path)
    }
}

/// Narrow `path` and every directory above it as far as `root`, both included.
///
/// Stops at `root` rather than walking to `/`: the directories above the state directory are the
/// user's own home and are none of this program's business.
#[cfg(unix)]
fn tighten(root: &Path, path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if !path.starts_with(root) {
        return;
    }
    let mut level = Some(path);
    while let Some(here) = level {
        let _ = std::fs::set_permissions(here, std::fs::Permissions::from_mode(0o700));
        if here == root {
            return;
        }
        level = here.parent();
    }
}

/// Write `contents` to `path`, readable only by this user.
///
/// The mode is asked for as the file is created rather than set once it is written. Several
/// callers write a temporary file and rename it over the real one, so a mode applied afterwards
/// would leave the contents readable for the length of the write, and the rename would carry the
/// temporary file's mode onto the real name anyway.
///
/// A file that is already there is tightened too, since one written by an older build has whatever
/// the umask gave it.
pub fn write_file(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let mut file = open_private(std::fs::OpenOptions::new().write(true).truncate(true), path)?;
    file.write_all(contents)
}

/// Open `path` to add to the end of it, readable only by this user.
///
/// Appending rather than rewriting is worth a second opening for: the history file is added to
/// once per prompt, and rewriting it would make that cost grow with how much history there is.
pub fn append_to_file(path: &Path) -> std::io::Result<std::fs::File> {
    open_private(std::fs::OpenOptions::new().append(true), path)
}

/// Open a file under the state directory, reachable only by this user.
fn open_private(options: &mut std::fs::OpenOptions, path: &Path) -> std::io::Result<std::fs::File> {
    options.create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
    }
    Ok(file)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A scratch directory that removes itself.
    struct Scratch {
        path: PathBuf,
    }

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = crate::testutil::scratch_dir(name);
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create scratch");
            Self {
                path: path.canonicalize().expect("canonical scratch"),
            }
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path)
            .expect("exists")
            .permissions()
            .mode()
            & 0o777
    }

    fn loosen(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("loosen");
    }

    /// The prompt history is every path, branch name and pasted fragment somebody has typed. On a
    /// shared machine the umask decides who else can read it, and the umask is not this program's
    /// to rely on.
    #[test]
    fn a_directory_is_created_reachable_only_by_its_owner() {
        let scratch = Scratch::new("home-create");
        let made = scratch.path.join("state");
        create_directory(&made).expect("created");

        assert_eq!(mode_of(&made), 0o700);
    }

    /// A machine that has already run an older build has the directory at whatever the umask gave
    /// it, and creating it again would leave it there: `create_dir_all` succeeds without touching
    /// the mode of something that already exists.
    #[test]
    fn a_directory_left_open_by_an_older_build_is_narrowed() {
        let scratch = Scratch::new("home-tighten");
        let state = scratch.path.join("state");
        let inside = state.join("sessions");
        std::fs::create_dir_all(&inside).expect("create");
        loosen(&state, 0o755);
        loosen(&inside, 0o755);

        tighten(&state, &inside);

        assert_eq!(mode_of(&inside), 0o700);
        assert_eq!(
            mode_of(&state),
            0o700,
            "the directory above it was left open"
        );
    }

    /// Whose home this is, and what else is in it, is the user's business. Walking past the state
    /// directory would have this program narrowing directories it was never asked about.
    #[test]
    fn narrowing_stops_at_the_state_directory() {
        let scratch = Scratch::new("home-stops");
        let state = scratch.path.join("state");
        std::fs::create_dir_all(&state).expect("create");
        loosen(&scratch.path, 0o755);

        tighten(&state, &state);

        assert_eq!(mode_of(&state), 0o700);
        assert_eq!(
            mode_of(&scratch.path),
            0o755,
            "a directory above the state directory"
        );
    }

    #[test]
    fn a_file_is_written_readable_only_by_its_owner() {
        let scratch = Scratch::new("home-write");
        let file = scratch.path.join("history");
        write_file(&file, b"a prompt\n").expect("written");

        assert_eq!(mode_of(&file), 0o600);
        assert_eq!(std::fs::read_to_string(&file).expect("read"), "a prompt\n");
    }

    /// The file an older build left is the one holding the history worth protecting, so writing it
    /// again has to narrow it rather than keep the mode it was found with.
    #[test]
    fn a_file_left_readable_by_an_older_build_is_narrowed() {
        let scratch = Scratch::new("home-rewrite");
        let file = scratch.path.join("history");
        std::fs::write(&file, "old").expect("write");
        loosen(&file, 0o644);

        write_file(&file, b"new").expect("written");

        assert_eq!(mode_of(&file), 0o600);
    }

    #[test]
    fn an_appended_file_is_readable_only_by_its_owner() {
        let scratch = Scratch::new("home-append");
        let file = scratch.path.join("history");
        let mut opened = append_to_file(&file).expect("opened");
        opened.write_all(b"one\n").expect("written");

        assert_eq!(mode_of(&file), 0o600);
    }
}
