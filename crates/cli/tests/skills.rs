//! `bua skills`, exercised by running the built binary.
//!
//! A subprocess rather than a unit test, because what is under test is partly the argument
//! dispatch in `main` and partly that the command needs no configuration to answer. Setting
//! `HOME` on a child process is also safe in a way setting it in-process is not.

use std::path::{Path, PathBuf};
use std::process::Command;

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("bua-cli-skills-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("home")).expect("home");
        std::fs::create_dir_all(path.join("project")).expect("project");
        Self { path }
    }

    /// What `HOME` is set to for the child.
    fn home(&self) -> PathBuf {
        self.path.join("home")
    }

    /// The directory the binary actually looks in, which is `$HOME/.bua`.
    fn dot_bua(&self) -> PathBuf {
        self.path.join("home/.bua")
    }

    fn project(&self) -> PathBuf {
        self.path.join("project")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn write_skill(root: &Path, dir: &str, name: &str, description: &str, body: &str) {
    let at = root.join("skills").join(dir);
    std::fs::create_dir_all(&at).expect("skill directory");
    std::fs::write(
        at.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n"),
    )
    .expect("write skill");
}

/// Run `bua skills` in `project` with `home` as HOME, returning stdout.
fn run(project: &Path, home: &Path) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_bua"))
        .arg("skills")
        .current_dir(project)
        .env("HOME", home)
        .output()
        .expect("run bua skills");

    assert!(
        output.status.success(),
        "bua skills failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

/// The command answers without configuration and without a model. Requiring an endpoint to ask
/// what is installed would make it useless in exactly the situation it exists for.
#[test]
fn listing_skills_needs_no_configuration_and_calls_nothing() {
    let scratch = Scratch::new("no-config");
    write_skill(
        &scratch.dot_bua(),
        "commit-style",
        "commit-style",
        "How commit messages are written here.",
        "the body",
    );

    let out = run(&scratch.project(), &scratch.home());

    assert!(out.contains("commit-style"), "{out}");
    assert!(
        out.contains("How commit messages are written here."),
        "{out}"
    );
    assert!(!out.contains("the body"), "a body was printed: {out}");
}

/// A one-shot command vouches for nothing, so a project's skills are not loaded. They must still
/// be listed, by path, or the user cannot tell "not trusted" from "not found".
#[test]
fn a_project_skill_is_listed_as_not_loaded_rather_than_omitted() {
    let scratch = Scratch::new("untrusted");
    write_skill(
        &scratch.project().join(".bua"),
        "local",
        "HOSTILE-NAME",
        "HOSTILE-DESCRIPTION",
        "b",
    );

    let out = run(&scratch.project(), &scratch.home());

    assert!(out.contains("not loaded"), "{out}");
    assert!(out.contains(".bua/skills/local/SKILL.md"), "{out}");
    assert!(out.contains("not trusted"), "{out}");
    assert!(
        !out.contains("HOSTILE-NAME") && !out.contains("HOSTILE-DESCRIPTION"),
        "the frontmatter of an untrusted skill was printed: {out}"
    );
    assert!(
        out.contains("/skills"),
        "the trusted route was not suggested: {out}"
    );
}

/// An escape sequence in a description reaches a terminal as bytes when this prints, unlike in
/// the session, where ratatui writes cells.
#[test]
fn control_characters_do_not_reach_the_terminal() {
    let scratch = Scratch::new("escapes");
    write_skill(
        &scratch.dot_bua(),
        "evil",
        "evil",
        "before\u{1b}[2Jafter",
        "b",
    );

    let out = run(&scratch.project(), &scratch.home());

    assert!(!out.contains('\u{1b}'), "an escape reached stdout: {out:?}");
    assert!(out.contains("evil"), "{out}");
}

/// Nothing installed is the first run. It should answer, not fail.
#[test]
fn an_empty_home_still_answers() {
    let scratch = Scratch::new("empty");
    let out = run(&scratch.project(), &scratch.home());
    assert!(out.contains("nothing loaded"), "{out}");
}
