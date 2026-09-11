//! What every crate root in this workspace says about `unsafe`.
//!
//! [LAYER-4] is the clause. The attribute at a root is what makes "this crate contains no unsafe"
//! the compiler's decision, and a crate that was never given one compiles exactly as well as a
//! crate that was, so nothing about adding the next crate surfaces the omission. This is what
//! surfaces it.
//!
//! The property belongs to the workspace rather than to any one crate, and it lives here because
//! this is the crate the shipped binary is built from.
//!
//! [LAYER-4]: ../../../docs/specs/layering.md

use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

/// The crate roots Cargo finds in a member without being told, relative to its directory.
const DISCOVERED_ROOTS: [&str; 3] = ["src/lib.rs", "src/main.rs", "build.rs"];

/// The manifest sections whose `path` names a crate root as well.
///
/// `[[test]]` is absent deliberately. A file under `tests/` is its own crate that no root
/// attribute reaches, which is the known cost recorded beside the clause.
const TARGET_SECTIONS: [&str; 4] = ["[lib]", "[[bin]]", "[[example]]", "[[bench]]"];

/// What a crate root says about unsafe code.
#[derive(Debug, PartialEq)]
enum Declaration {
    /// `#![forbid(unsafe_code)]`: no `unsafe` in the crate, and nothing inside it can reopen that.
    Forbidden,
    /// `#![deny(unsafe_code)]`: an error except where a site allows it by name.
    Denied,
    /// Nothing at the root, so the compiler decides nothing.
    Absent,
}

/// Read the declaration off a crate root.
///
/// Only the crate's own prologue counts. An inner attribute may appear only before any item, so
/// the first line that is neither one nor a comment ends it: reading further would take an
/// attribute written inside a nested module for a statement about the crate.
///
/// `#![forbid(unsafe_code)]` is the crate's policy, whereas the same attribute without the `!` is
/// one item's, and an `allow` at the root is the shape of skipping the rule rather than obeying it.
fn declaration(source: &str) -> Declaration {
    for line in source.lines() {
        let line = line.split("//").next().unwrap_or(line).trim();
        if line.is_empty() {
            continue;
        }
        let Some(rest) = line.strip_prefix("#![") else {
            return Declaration::Absent;
        };
        // Still in the prologue, just not a lint attribute: `#![no_std]` and the like.
        let Some(rest) = rest.strip_suffix(")]") else {
            continue;
        };
        let Some((lint, lints)) = rest.split_once('(') else {
            continue;
        };
        if !lints.split(',').any(|name| name.trim() == "unsafe_code") {
            continue;
        }
        match lint {
            "forbid" => return Declaration::Forbidden,
            "deny" => return Declaration::Denied,
            _ => continue,
        }
    }
    Declaration::Absent
}

/// The workspace members, read from the root manifest rather than found by walking `crates/`: a
/// directory that is not a member is not compiled, and a member outside `crates/` would be missed
/// by a walk while still shipping in the binary.
fn members(manifest: &str) -> Vec<String> {
    let mut workspace_table = false;
    let mut assignment = String::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            workspace_table = line == "[workspace]";
            continue;
        }
        if !workspace_table || (assignment.is_empty() && !line.starts_with("members")) {
            continue;
        }
        assignment.push_str(line);
        if line.contains(']') {
            break;
        }
    }
    assignment
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

/// The crate roots a member's own manifest names, so a target at a path Cargo would not have found
/// is checked rather than skipped.
fn declared_roots(manifest: &str) -> Vec<String> {
    let mut section = String::new();
    let mut paths = Vec::new();
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line.to_string();
            continue;
        }
        if !TARGET_SECTIONS.contains(&section.as_str()) {
            continue;
        }
        if let Some(value) = line.strip_prefix("path")
            && let Some(path) = value.split('"').nth(1)
        {
            paths.push(path.to_string());
        }
    }
    paths
}

/// Every file in a member's `src/bin`, which Cargo compiles as a binary of its own with nothing
/// in the manifest saying so.
fn discovered_binaries(directory: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(directory.join("src/bin")) else {
        return Vec::new();
    };
    entries
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .map(|path| {
            let name = path.file_name().expect("a file name").to_string_lossy();
            format!("src/bin/{name}")
        })
        .collect()
}

/// Every crate root in the workspace, as a workspace-relative name and the file to read.
fn crate_roots() -> Vec<(String, PathBuf)> {
    // CARGO_MANIFEST_DIR is `<workspace>/crates/cli`, so two pops reach the root.
    let mut workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    workspace.pop();
    workspace.pop();

    let manifest =
        std::fs::read_to_string(workspace.join("Cargo.toml")).expect("the workspace manifest");
    let members = members(&manifest);
    assert!(
        !members.is_empty(),
        "no workspace members were read out of Cargo.toml, so this test would pass by checking \
         nothing. The `members` list is what it reads; follow it if it has moved"
    );

    let mut roots = Vec::new();
    for member in members {
        let directory = workspace.join(&member);
        let member_manifest = std::fs::read_to_string(directory.join("Cargo.toml"))
            .expect("a workspace member's manifest");

        let mut relative: BTreeSet<String> =
            DISCOVERED_ROOTS.iter().map(|&root| root.into()).collect();
        relative.extend(declared_roots(&member_manifest));
        relative.extend(discovered_binaries(&directory));

        let found: Vec<(String, PathBuf)> = relative
            .iter()
            .map(|root| (format!("{member}/{root}"), directory.join(root)))
            .filter(|(_, path)| path.exists())
            .collect();
        assert!(
            !found.is_empty(),
            "{member} has no crate root among {relative:?}, so this test says nothing about it. \
             Its root is somewhere else; name that path in the manifest or in DISCOVERED_ROOTS"
        );
        roots.extend(found);
    }
    roots
}

/// Whether unsafe code is allowed at a named site anywhere under a path.
fn names_an_exemption(path: &Path) -> bool {
    if path.is_dir() {
        return std::fs::read_dir(path)
            .expect("a source directory")
            .any(|entry| names_an_exemption(&entry.expect("a directory entry").path()));
    }
    if path.extension().is_none_or(|extension| extension != "rs") {
        return false;
    }
    std::fs::read_to_string(path)
        .expect("a source file")
        .lines()
        .any(|line| line.trim().starts_with("#[allow(") && line.contains("unsafe_code"))
}

/// The rule itself: nothing compiles here without having answered the question.
#[test]
fn every_crate_root_says_what_it_does_about_unsafe() {
    for (name, path) in crate_roots() {
        let source = std::fs::read_to_string(&path).expect("a crate root");
        assert!(
            declaration(&source) != Declaration::Absent,
            "{name} declares nothing about unsafe code in its own prologue. Add \
             `#![forbid(unsafe_code)]`, or `#![deny(unsafe_code)]` with an \
             `#[allow(unsafe_code)]` at each site that needs one"
        );
    }
}

/// `deny` is the weaker of the two, because an `allow` further down reopens what it closed. A
/// crate with nothing to exempt has no reason to accept that, so taking `deny` where `forbid`
/// would do is how the rule gets obeyed in letter and lost in substance.
#[test]
fn a_crate_that_exempts_nothing_forbids_rather_than_denies() {
    for (name, path) in crate_roots() {
        let source = std::fs::read_to_string(&path).expect("a crate root");
        if declaration(&source) != Declaration::Denied {
            continue;
        }
        // What the attribute reaches: the directory a `src/` root sits in, and nothing but itself
        // for a build script, which is one file.
        let reach = if path.ends_with("build.rs") {
            path.clone()
        } else {
            path.parent()
                .expect("a crate root has a parent")
                .to_path_buf()
        };
        assert!(
            names_an_exemption(&reach),
            "{name} denies unsafe code and exempts nothing, so it can say `forbid` instead and \
             close the crate to an `allow` added later"
        );
    }
}

/// A reader that accepts the wrong shapes enforces nothing, and the wrong shapes are the ones a
/// crate skipping the rule would have: an `allow` at the root, the attribute named in prose, the
/// attribute on one item rather than on the crate, and the attribute inside a module, which says
/// nothing about the crate around it.
#[test]
fn allowing_unsafe_at_a_root_is_not_a_declaration() {
    assert_eq!(declaration("#![allow(unsafe_code)]\n"), Declaration::Absent);
    assert_eq!(
        declaration("//! This crate has #![forbid(unsafe_code)].\n"),
        Declaration::Absent
    );
    assert_eq!(
        declaration("#[forbid(unsafe_code)]\nfn f() {}\n"),
        Declaration::Absent
    );
    assert_eq!(
        declaration("mod inner {\n    #![forbid(unsafe_code)]\n}\n"),
        Declaration::Absent
    );
    assert_eq!(
        declaration("//! Docs.\n\n#![no_std]\n#![warn(missing_docs)]\n#![forbid(unsafe_code)]\n"),
        Declaration::Forbidden
    );
    assert_eq!(
        declaration("#![deny(unsafe_code, unused_crate_dependencies)] // why\n"),
        Declaration::Denied
    );
}
