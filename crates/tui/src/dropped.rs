//! Recognising a file dropped on the window.
//!
//! A terminal has no notion of a drop. What arrives is a bracketed paste of whatever the window
//! manager handed over, which is the path, so recognising one means looking at a paste and
//! deciding whether it is a path or prose. That decision is made here and nowhere else.
//!
//! Getting it wrong in one direction costs a paste: a paragraph that mentions a filename would
//! turn into an attachment and the words would vanish. So the test is deliberately strict, and a
//! paste is a drop only when **every** token in it names a file that exists. Prose containing a
//! real path stays prose, because prose has other words in it.
//!
//! Nothing here decides anything from the contents of a file. It reads names, which the user is
//! about to see on their own screen, and asks the filesystem whether they exist.

use std::path::Path;

/// What a dropped file is, and therefore what happens to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Carried to the model as bytes, in the media type named here.
    Attachment(&'static str),
    /// Read into the turn as text, the way a file named with `@` is.
    Text,
}

/// A file the user dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    /// The path as the filesystem names it, which is what the terminal handed over.
    pub path: String,
    pub kind: Kind,
}

impl Dropped {
    /// The word the marker uses, so a user can tell one dropped thing from another.
    /// Not from a catalog: this word goes into the marker the planner is sent, so it is part of
    /// what the model reads rather than something a person is being told. See [`Session::attach`].
    pub fn noun(&self) -> &'static str {
        match self.kind {
            Kind::Attachment("application/pdf") => "PDF",
            Kind::Attachment(_) => "Image",
            Kind::Text => "File",
        }
    }
}

/// Extensions carried as bytes, with the type to name in the URI.
///
/// The set Claude Code takes. Deciding by extension rather than by sniffing the file: naming a
/// type from the bytes would be a decision taken from content nobody has vouched for, and it would
/// be taken here, in the driver.
const ATTACHABLE: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("pdf", "application/pdf"),
];

/// Extensions read as text, which is what the model wants of them anyway.
const TEXTUAL: &[&str] = &[
    "txt",
    "md",
    "markdown",
    "rst",
    "adoc",
    "org",
    "rs",
    "py",
    "js",
    "jsx",
    "mjs",
    "cjs",
    "ts",
    "tsx",
    "vue",
    "svelte",
    "json",
    "jsonc",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "properties",
    "env",
    "html",
    "htm",
    "xml",
    "svg",
    "css",
    "scss",
    "sass",
    "less",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "bat",
    "c",
    "h",
    "cc",
    "cpp",
    "cxx",
    "hpp",
    "hh",
    "java",
    "kt",
    "kts",
    "go",
    "rb",
    "php",
    "swift",
    "m",
    "mm",
    "cs",
    "scala",
    "clj",
    "cljs",
    "ex",
    "exs",
    "erl",
    "hs",
    "lua",
    "pl",
    "pm",
    "r",
    "jl",
    "dart",
    "zig",
    "nim",
    "sql",
    "graphql",
    "proto",
    "csv",
    "tsv",
    "log",
    "diff",
    "patch",
    "lock",
    "gradle",
    "tf",
    "tfvars",
    "dockerfile",
    "mk",
    "cmake",
];

/// Names that are text without an extension to say so.
const TEXTUAL_NAMES: &[&str] = &[
    "makefile",
    "dockerfile",
    "readme",
    "license",
    "licence",
    "changelog",
    "authors",
    "notice",
    "gemfile",
    "rakefile",
    "procfile",
    "justfile",
    "vagrantfile",
    "brewfile",
];

/// What a path's name says it is, or `None` for something neither carried nor read.
///
/// A `.dmg` lands here, and the interface writes its path out rather than pretending to attach it.
pub fn kind_of(path: &str) -> Option<Kind> {
    let name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())?;

    // Split on the last dot ourselves rather than asking for an extension, so a name that is all
    // extension, `.gitignore`, is read as a name and not as an extension of nothing.
    let extension = name.rsplit_once('.').map(|(stem, ext)| {
        if stem.is_empty() {
            String::new()
        } else {
            ext.to_string()
        }
    });

    if let Some(extension) = &extension {
        if let Some((_, media)) = ATTACHABLE.iter().find(|(ext, _)| ext == extension) {
            return Some(Kind::Attachment(media));
        }
        if TEXTUAL.contains(&extension.as_str()) {
            return Some(Kind::Text);
        }
    }

    // A name with no usable extension: `Makefile`, or `.gitignore`, whose whole name is the name.
    let bare = name.strip_prefix('.').unwrap_or(&name);
    if TEXTUAL_NAMES.contains(&bare) || bare.starts_with("gitignore") || bare.starts_with("gitattr")
    {
        return Some(Kind::Text);
    }

    None
}

/// The files a paste names, or nothing at all when the paste is not a drop.
///
/// `exists` answers whether a path names a file, taken as an argument so the decision can be
/// tested without touching a disk.
pub fn dropped_with(text: &str, exists: impl Fn(&str) -> bool) -> Vec<Dropped> {
    let tokens = tokenise(text);
    if tokens.is_empty() {
        return Vec::new();
    }

    // Every token or none. One word of prose is enough to make this a paste, and treating a
    // paragraph as a drop would silently swallow it.
    let mut found = Vec::new();
    for token in tokens {
        let path = unwrap_uri(&token);
        if !exists(&path) {
            return Vec::new();
        }
        // An unrecognised type is still a drop, and the caller writes its path out. Refusing the
        // whole paste here would turn a dropped `.dmg` beside a dropped `.png` into prose.
        let kind = kind_of(&path);
        found.push((path, kind));
    }

    found
        .into_iter()
        .filter_map(|(path, kind)| kind.map(|kind| Dropped { path, kind }))
        .collect()
}

/// The paths a drop names, in the order they were dropped, whatever their type.
///
/// The caller needs the whole list, not just the attachable ones: an unsupported file has its path
/// written out where its marker would have gone, so the order has to survive.
pub fn paths(text: &str) -> Vec<String> {
    tokenise(text)
        .iter()
        .map(|token| unwrap_uri(token))
        .collect()
}

/// Whether a paste is a drop at all, whether or not each file is a type we take.
///
/// Separate from [`dropped_with`] because the caller needs to know a drop happened even when
/// nothing in it was attachable: that is the case where the path is written out.
pub fn is_drop(text: &str, exists: impl Fn(&str) -> bool) -> bool {
    let tokens = tokenise(text);
    !tokens.is_empty() && tokens.iter().all(|token| exists(&unwrap_uri(token)))
}

/// Split a dropped line into paths, undoing the quoting terminals apply.
///
/// Three forms, because three families of terminal do three different things: a backslash before
/// each awkward character, single quotes around the whole path, or double quotes.
fn tokenise(text: &str) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() || text.contains('\n') {
        // A drop is one line. Anything with a newline in it is a paste of something else.
        return Vec::new();
    }

    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut any = false;

    for character in text.chars() {
        if escaped {
            current.push(character);
            escaped = false;
            any = true;
            continue;
        }
        match (quote, character) {
            (None, '\\') => escaped = true,
            (None, '\'' | '"') => {
                quote = Some(character);
                any = true;
            }
            (Some(open), c) if c == open => quote = None,
            (None, ' ' | '\t') => {
                if any {
                    tokens.push(std::mem::take(&mut current));
                    any = false;
                }
            }
            (_, c) => {
                current.push(c);
                any = true;
            }
        }
    }

    // An unterminated quote or a trailing backslash is not something a terminal emits for a drop.
    if quote.is_some() || escaped {
        return Vec::new();
    }
    if any {
        tokens.push(current);
    }

    tokens
}

/// Turn a `file://` URI into a path, which is what some terminals hand over instead.
fn unwrap_uri(token: &str) -> String {
    let Some(rest) = token.strip_prefix("file://") else {
        return token.to_string();
    };

    // `file:///path` has an empty authority; anything else names a host we have no business
    // reaching for, so it is left alone and will simply fail to exist.
    let Some(path) = rest.strip_prefix('/') else {
        return token.to_string();
    };

    percent_decoded(&format!("/{path}"))
}

/// Undo percent-encoding, by hand rather than by dependency.
///
/// Anything that is not a well-formed escape is left exactly as it was: a literal `%` in a
/// filename is a real thing, and mangling it would name a file that does not exist.
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let high = (bytes[index + 1] as char).to_digit(16);
            let low = (bytes[index + 2] as char).to_digit(16);
            if let (Some(high), Some(low)) = (high, low) {
                out.push((high * 16 + low) as u8);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }

    String::from_utf8(out).unwrap_or_else(|_| text.to_string())
}
/// The name to give a task for a dropped file, or `None` for something that is not a file.
///
/// A drop hands over an absolute path, and it is nearly always `~/Downloads` or `~/Desktop`. That
/// is the case the feature exists for, so it is named as it is and carried: what makes an
/// attachment safe is that a person made the gesture, not where the file happens to sit.
///
/// A file inside the workspace is named relative to it instead. Not a restriction, a tidiness: the
/// trust map and everything a user reads are in workspace-relative terms, and an absolute path for
/// a file two directories away would be the odd one out.
pub fn name_for(root: &Path, path: &str) -> Option<String> {
    let canonical = Path::new(path).canonicalize().ok()?;
    if !canonical.is_file() {
        return None;
    }

    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    if let Ok(relative) = canonical.strip_prefix(&root) {
        return Some(relative.to_string_lossy().to_string());
    }

    Some(canonical.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Everything exists, for the cases that are about parsing rather than about the filesystem.
    fn all(_: &str) -> bool {
        true
    }

    fn only(paths: &'static [&'static str]) -> impl Fn(&str) -> bool {
        move |candidate: &str| paths.contains(&candidate)
    }

    #[test]
    fn a_plain_path_is_a_drop() {
        let found = dropped_with("/tmp/shot.png", all);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "/tmp/shot.png");
        assert_eq!(found[0].kind, Kind::Attachment("image/png"));
    }

    /// macOS escapes the awkward characters rather than quoting the whole path.
    #[test]
    fn a_backslash_escaped_path_is_unescaped() {
        let found = dropped_with(r"/tmp/my\ shot.png", all);
        assert_eq!(found[0].path, "/tmp/my shot.png");
    }

    #[test]
    fn a_quoted_path_is_unquoted() {
        assert_eq!(
            dropped_with("'/tmp/my shot.png'", all)[0].path,
            "/tmp/my shot.png"
        );
        assert_eq!(
            dropped_with("\"/tmp/my shot.png\"", all)[0].path,
            "/tmp/my shot.png"
        );
    }

    /// Some terminals hand over a URI, percent-encoded, rather than a path.
    #[test]
    fn a_file_uri_becomes_a_path() {
        let found = dropped_with("file:///tmp/my%20shot.png", all);
        assert_eq!(found[0].path, "/tmp/my shot.png");
    }

    /// A percent that is not an escape is part of the name, and mangling it would name a file
    /// that does not exist.
    #[test]
    fn a_literal_percent_in_a_name_survives() {
        assert_eq!(percent_decoded("/tmp/100%.png"), "/tmp/100%.png");
        assert_eq!(percent_decoded("/tmp/a%zz.png"), "/tmp/a%zz.png");
    }

    #[test]
    fn several_files_dropped_at_once_are_all_taken() {
        let found = dropped_with("/tmp/a.png /tmp/b.pdf", all);
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].kind, Kind::Attachment("application/pdf"));
    }

    /// The load-bearing test. A paragraph that happens to mention a real path must stay a
    /// paragraph: turning it into an attachment would swallow everything the user pasted.
    #[test]
    fn prose_mentioning_a_real_file_is_not_a_drop() {
        let exists = only(&["/tmp/shot.png"]);
        assert!(dropped_with("please look at /tmp/shot.png and tell me", &exists).is_empty());
        assert!(!is_drop(
            "please look at /tmp/shot.png and tell me",
            &exists
        ));
    }

    /// And a path that does not exist is somebody typing, not somebody dropping.
    #[test]
    fn a_path_that_names_nothing_is_not_a_drop() {
        let exists = only(&["/tmp/real.png"]);
        assert!(dropped_with("/tmp/imagined.png", &exists).is_empty());
    }

    /// Several paths where one is prose is still prose.
    #[test]
    fn one_word_of_prose_is_enough_to_make_it_a_paste() {
        let exists = only(&["/tmp/a.png", "/tmp/b.png"]);
        assert!(dropped_with("/tmp/a.png and /tmp/b.png", &exists).is_empty());
    }

    /// A pasted paragraph has newlines. A drop does not.
    #[test]
    fn a_multi_line_paste_is_never_a_drop() {
        assert!(dropped_with("/tmp/a.png\n/tmp/b.png", all).is_empty());
    }

    /// The behaviour asked for: an unsupported type is a drop, but nothing is attached, so the
    /// caller writes its path out the way it always did.
    #[test]
    fn an_unsupported_type_is_a_drop_that_attaches_nothing() {
        assert!(is_drop("/tmp/thing.dmg", all));
        assert!(dropped_with("/tmp/thing.dmg", all).is_empty());
        assert_eq!(kind_of("/tmp/thing.dmg"), None);
    }

    /// And an unsupported file beside a supported one does not spoil the supported one.
    #[test]
    fn an_unsupported_file_beside_a_supported_one_leaves_it_attachable() {
        let found = dropped_with("/tmp/a.png /tmp/b.dmg", all);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, "/tmp/a.png");
    }

    #[test]
    fn the_recognised_types_are_the_ones_claude_code_takes() {
        for name in ["a.png", "a.jpg", "a.jpeg", "a.gif", "a.webp"] {
            assert!(
                matches!(kind_of(name), Some(Kind::Attachment(media)) if media.starts_with("image/")),
                "{name} is not an image"
            );
        }
        assert_eq!(kind_of("a.pdf"), Some(Kind::Attachment("application/pdf")));
        for name in [
            "a.rs",
            "a.md",
            "a.json",
            "Makefile",
            "Dockerfile",
            ".gitignore",
        ] {
            assert_eq!(kind_of(name), Some(Kind::Text), "{name} is not text");
        }
        for name in ["a.dmg", "a.zip", "a.mp4", "a.so"] {
            assert_eq!(kind_of(name), None, "{name} should not be taken");
        }
    }

    /// Case is the filesystem's business, not the user's.
    #[test]
    fn an_extension_is_recognised_whatever_its_case() {
        assert_eq!(
            kind_of("/tmp/SHOT.PNG"),
            Some(Kind::Attachment("image/png"))
        );
    }

    #[test]
    fn the_noun_names_what_was_dropped() {
        let image = Dropped {
            path: "a.png".into(),
            kind: Kind::Attachment("image/png"),
        };
        let pdf = Dropped {
            path: "a.pdf".into(),
            kind: Kind::Attachment("application/pdf"),
        };
        let text = Dropped {
            path: "a.rs".into(),
            kind: Kind::Text,
        };
        assert_eq!(image.noun(), "Image");
        assert_eq!(pdf.noun(), "PDF");
        assert_eq!(text.noun(), "File");
    }

    /// An unterminated quote is not something a terminal emits, so it is a paste.
    #[test]
    fn an_unterminated_quote_is_not_a_drop() {
        assert!(dropped_with("'/tmp/a.png", all).is_empty());
    }

    #[test]
    fn an_empty_paste_is_not_a_drop() {
        assert!(dropped_with("", all).is_empty());
        assert!(dropped_with("   ", all).is_empty());
    }
}
