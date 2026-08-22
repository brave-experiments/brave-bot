//! Programs whose output is a function of their input.
//!
//! Most of what [`crate::command`] runs is opaque: it might write, it might reach the network, and
//! nothing here can tell, so its output is untrusted and a person approves it. A few programs are
//! different. `wc -l` reading stdin cannot do anything except count lines, so its output is
//! determined by what went in, and a stage like that needs neither a prompt nor a pessimistic
//! label. Trusted input gives trusted output; untrusted input gives untrusted output.
//!
//! That is not a relabel and grants nothing. It is the same reasoning
//! [`crate::policy::Policy::label_model_output`] rests on, applied to a process instead of a model:
//! when the output is a function of the inputs, the output's first label is the meet of theirs.
//!
//! # Eligibility is about (program, argv), never a program alone
//!
//! The temptation is a list of trustworthy program names. That would be wrong, and wrong in a way
//! that fails open, because the two programs anyone would list first are the two that must never be
//! on it.
//!
//! `sed` and `awk` are not filters, they are interpreters, and the program they run is an argument:
//!
//! ```text
//! printf 'x\n' | sed 'w leaked.txt'          # writes a file
//! printf 'x\n' | awk '{system("...")}'       # runs an arbitrary command
//! ```
//!
//! Awk's `system()` reaches the shell this repository excludes, so admitting `awk` would undo the
//! exclusion sideways. Recognising a *safe* sed or awk script means parsing sed's and awk's
//! languages, which is the same trap as parsing a shell string: a parser racing an interpreter it
//! does not control. Neither is eligible, and neither ever becomes eligible.
//!
//! `sort` and `uniq` are ordinary filters excluded for a smaller reason: both take an output file
//! (`sort -o`, and `uniq`'s second positional).
//!
//! # What eligibility requires
//!
//! All three, and a failure of any falls back to the opaque default:
//!
//! 1. The program is in [`FILTERS`].
//! 2. No argument is a file operand. Every input must arrive on stdin, so `head -1 /etc/hosts` does
//!    not qualify: it reads a file whose contents the label would not account for.
//! 3. No denied flag appears. These are the ones that would break rule 2 or 1 for a program that is
//!    otherwise fine, such as `grep -f`, which names an input file without looking like it, and
//!    `tail -f`, which never terminates.
//!
//! # Resolve the program, do not trust the name
//!
//! A name is not a program. On the machine this was developed against, `grep` resolves to `ugrep`, a
//! different implementation with a far larger option surface. `$PATH` and shell aliases both decide
//! what a name means, so a caller must match on what the name resolved to and record it. This module
//! judges a resolved program; finding out what a name resolves to is the caller's job.

/// A program that transforms stdin and does nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Filter {
    /// The program's name, as resolved.
    pub program: &'static str,
    /// Flags that disqualify a call to it.
    ///
    /// Matched exactly against whole arguments, and against the letters of a bundled short flag, so
    /// `-nf` is caught as `-f` would be.
    pub denied: &'static [&'static str],
    /// How many non-flag arguments are *not* files.
    ///
    /// Operand meaning is per-program and cannot be generalised. `wc FILE` reads a file, while
    /// `grep PATTERN` takes a pattern and `tr SET1 SET2` takes two character sets. So each program
    /// says how many operands are part of its instruction, and anything beyond that count is a file
    /// to read: an input the label would not account for.
    pub operands: usize,
}

/// Every program eligible for label pass-through.
///
/// Short on purpose. Each entry is a claim that no argument can make the program write, execute, or
/// read a file, and that claim was checked against the program's own option list rather than assumed
/// from what it is usually used for.
pub const FILTERS: &[Filter] = &[
    // Options are only -Lclmw. Every operand is a file to read.
    Filter {
        program: "wc",
        denied: &[],
        operands: 0,
    },
    Filter {
        program: "head",
        denied: &[],
        operands: 0,
    },
    // -f and -F follow forever, so a stage using them would never finish.
    Filter {
        program: "tail",
        denied: &["-f", "-F", "--follow"],
        operands: 0,
    },
    Filter {
        program: "cut",
        denied: &[],
        operands: 0,
    },
    // SET1 and SET2 are character sets, not files. tr reads only stdin.
    Filter {
        program: "tr",
        denied: &[],
        operands: 2,
    },
    // Writes nothing, checked against BSD grep's full option set. The first operand is the pattern;
    // anything after it is a file. And -f names a *pattern* file, so it is a file operand that does
    // not look like one: `grep -f ~/.ssh/id_rsa` would read a key in as patterns while the label came
    // from stdin.
    Filter {
        program: "grep",
        denied: &[
            "-f",
            "--file",
            // Recursion ignores stdin entirely and walks the filesystem instead, so the output would
            // be labelled from stdin while the data came from disk. Found by testing rather than by
            // reading the option list, which is why the list is checked against the binary.
            "-r",
            "-R",
            "--recursive",
            "-d",
            "--directories",
        ],
        operands: 1,
    },
    // Operates on the string it is given rather than on a file of that name.
    Filter {
        program: "basename",
        denied: &[],
        operands: 2,
    },
    Filter {
        program: "dirname",
        denied: &[],
        operands: 1,
    },
    // Takes no input at all: it reports the working directory, which the user established. Its
    // output is therefore trusted whatever was piped past it.
    Filter {
        program: "pwd",
        denied: &[],
        operands: 0,
    },
];

/// Programs that must never be eligible, whatever their arguments.
///
/// Held explicitly rather than left absent so the reason survives, and so a test can assert they
/// stay out. Absence is easy to reverse by accident; a named exclusion is not.
pub const NEVER: &[&str] = &[
    "sed", "awk", "gawk", "perl", "python", "python3", "ruby", "sh", "bash", "zsh", "sort", "uniq",
    "tee", "dd", "xargs", "find",
];

/// Whether a resolved program with these arguments only transforms stdin.
///
/// Conservative by construction: unknown program, unknown-looking operand, or denied flag all
/// return false, and false means the caller keeps the opaque default.
pub fn is_pure_filter(program: &str, args: &[String]) -> bool {
    // A path resolves to a file name; the table names programs.
    let name = program_name(program);

    if NEVER.contains(&name) {
        return false;
    }

    let Some(filter) = FILTERS.iter().find(|f| f.program == name) else {
        return false;
    };

    let mut operands_seen = 0usize;
    args.iter()
        .all(|arg| permitted_argument(filter, arg, &mut operands_seen))
}

/// The file name part of a program path, so `/usr/bin/wc` matches `wc`.
fn program_name(program: &str) -> &str {
    program
        .rsplit_once('/')
        .map(|(_, name)| name)
        .unwrap_or(program)
}

/// Whether one argument keeps a call eligible.
///
/// `operands_seen` counts the non-flag arguments so far, since a program's first operand may be part
/// of its instruction while a later one is a file.
fn permitted_argument(filter: &Filter, arg: &str, operands_seen: &mut usize) -> bool {
    if let Some(long) = arg.strip_prefix("--") {
        // `--file=x` is `--file`, so the value is stripped before matching.
        let flag = format!("--{}", long.split('=').next().unwrap_or(long));
        return !filter.denied.contains(&flag.as_str());
    }

    if let Some(letters) = arg.strip_prefix('-') {
        // `-` alone means stdin, which is exactly what is wanted.
        if letters.is_empty() {
            return true;
        }
        // Bundled short flags: `-nf` contains `-f`, so each letter is checked separately.
        return letters.chars().all(|letter| {
            let flag = format!("-{letter}");
            !filter.denied.contains(&flag.as_str())
        });
    }

    // A non-flag argument. The first `operands` of them are part of the instruction, such as grep's
    // pattern or tr's character sets. Beyond that they are files to read, and a file read is an
    // input the label would not account for.
    *operands_seen += 1;
    *operands_seen <= filter.operands
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn a_plain_filter_reading_stdin_qualifies() {
        assert!(is_pure_filter("wc", &args(&["-l"])));
        assert!(is_pure_filter("head", &args(&["-20"])));
        assert!(is_pure_filter("cut", &args(&["-d:", "-f1"])));
        assert!(is_pure_filter("grep", &args(&["-i", "error"])));
    }

    /// A resolved absolute path is what a caller will have, so the table has to match on the file
    /// name rather than the whole path.
    #[test]
    fn a_resolved_path_matches_its_program_name() {
        assert!(is_pure_filter("/usr/bin/wc", &args(&["-l"])));
        assert!(!is_pure_filter("/usr/bin/sed", &args(&["s/a/b/"])));
    }

    /// The most important test here. These are the two programs a reader would list first, and both
    /// can write files and run commands from an argument, so neither may ever qualify.
    #[test]
    fn interpreters_never_qualify_however_harmless_they_look() {
        for program in ["sed", "awk", "gawk"] {
            assert!(
                !is_pure_filter(program, &args(&[])),
                "{program} qualified with no arguments"
            );
            assert!(
                !is_pure_filter(program, &args(&["-n", "1p"])),
                "{program} qualified with innocuous-looking arguments"
            );
        }
    }

    /// Awk's `system()` reaches the shell this repository excludes, so admitting awk would undo that
    /// exclusion by another route.
    #[test]
    fn awk_cannot_qualify_even_though_it_looks_like_a_filter() {
        assert!(!is_pure_filter("awk", &args(&["{print $1}"])));
        assert!(!is_pure_filter("awk", &args(&["{system(\"rm -rf /\")}"])));
    }

    /// Ordinary filters with an output-file option are excluded, and stay excluded even when the
    /// option is absent: eligibility is not decided by whether this particular call looked safe.
    #[test]
    fn filters_with_an_output_file_option_are_excluded() {
        assert!(!is_pure_filter("sort", &args(&[])));
        assert!(!is_pure_filter("sort", &args(&["-o", "out.txt"])));
        assert!(!is_pure_filter("uniq", &args(&[])));
        assert!(!is_pure_filter("tee", &args(&["out.txt"])));
    }

    /// A shell is never a filter, whatever it is asked to do.
    #[test]
    fn shells_never_qualify() {
        for shell in ["sh", "bash", "zsh"] {
            assert!(!is_pure_filter(shell, &args(&["-c", "echo hi"])));
        }
    }

    /// An unrecognised program falls through to the opaque default rather than being guessed at.
    #[test]
    fn an_unknown_program_does_not_qualify() {
        assert!(!is_pure_filter("some-tool-nobody-listed", &args(&[])));
        assert!(!is_pure_filter("git", &args(&["log"])));
        assert!(!is_pure_filter("curl", &args(&["https://example.com"])));
    }

    /// A file operand is an input the label would not account for, so it disqualifies even a program
    /// that is otherwise eligible.
    #[test]
    fn a_file_operand_disqualifies_an_otherwise_pure_filter() {
        assert!(!is_pure_filter("head", &args(&["-1", "/etc/hosts"])));
        assert!(!is_pure_filter("wc", &args(&["-l", "secrets.txt"])));
        assert!(!is_pure_filter("grep", &args(&["error", "log.txt"])));
    }

    /// A bare `-` means stdin, which is the case this exists for.
    #[test]
    fn a_lone_dash_is_stdin_not_an_operand() {
        assert!(is_pure_filter("wc", &args(&["-l", "-"])));
    }

    /// `grep -f` names a pattern file, so it reads from disk without looking like it does.
    #[test]
    fn grep_reading_a_pattern_file_does_not_qualify() {
        assert!(!is_pure_filter("grep", &args(&["-f", "patterns.txt"])));
        assert!(!is_pure_filter("grep", &args(&["--file=patterns.txt"])));
    }

    /// The hole adversarial testing found: `grep -r pattern` with no file operand does not read
    /// stdin at all. It recurses the working directory, so the output would be labelled from stdin
    /// while the data came from the filesystem.
    #[test]
    fn grep_recursing_the_filesystem_does_not_qualify() {
        assert!(!is_pure_filter("grep", &args(&["-r", "secret"])));
        assert!(!is_pure_filter("grep", &args(&["-R", "secret"])));
        assert!(!is_pure_filter("grep", &args(&["--recursive", "secret"])));
        // Bundled with another flag, it must still be caught.
        assert!(!is_pure_filter("grep", &args(&["-ir", "secret"])));
    }

    /// Directory traversal flags are the same hazard: they name inputs the label cannot account for.
    #[test]
    fn grep_directory_traversal_flags_do_not_qualify() {
        assert!(!is_pure_filter("grep", &args(&["-d", "recurse", "x"])));
        assert!(!is_pure_filter(
            "grep",
            &args(&["--directories=recurse", "x"])
        ));
    }

    /// A denied flag bundled with others must still be caught, or `-nf` would slip past a check that
    /// only compared whole arguments.
    #[test]
    fn a_denied_flag_bundled_with_others_is_still_caught() {
        assert!(!is_pure_filter("grep", &args(&["-if", "patterns.txt"])));
        assert!(!is_pure_filter("tail", &args(&["-nf"])));
    }

    /// Following a file never terminates, so a stage doing it would hang the turn.
    #[test]
    fn tail_following_a_file_does_not_qualify() {
        assert!(!is_pure_filter("tail", &args(&["-f"])));
        assert!(!is_pure_filter("tail", &args(&["-F"])));
        assert!(!is_pure_filter("tail", &args(&["--follow"])));
        // But an ordinary tail is fine.
        assert!(is_pure_filter("tail", &args(&["-5"])));
    }

    /// The string transforms take their input as arguments rather than reading files, so an operand
    /// is not a file for them.
    #[test]
    fn string_transforms_take_operands_rather_than_files() {
        assert!(is_pure_filter("basename", &args(&["/a/b/c.txt"])));
        assert!(is_pure_filter("dirname", &args(&["/a/b/c.txt"])));
        assert!(is_pure_filter("tr", &args(&["a-z", "A-Z"])));
    }

    /// An operand's meaning is per-program, so the count is what decides. grep's first operand is a
    /// pattern and its second is a file, and only the second disqualifies.
    #[test]
    fn a_pattern_is_an_instruction_but_a_second_operand_is_a_file() {
        assert!(is_pure_filter("grep", &args(&["error"])));
        assert!(!is_pure_filter("grep", &args(&["error", "log.txt"])));
    }

    /// The counts have to be exact in both directions, or a file would slip past as an instruction.
    #[test]
    fn an_operand_beyond_the_count_is_a_file() {
        // tr takes two sets and reads only stdin, so a third operand is a file.
        assert!(is_pure_filter("tr", &args(&["a-z", "A-Z"])));
        assert!(!is_pure_filter("tr", &args(&["a-z", "A-Z", "input.txt"])));

        // dirname takes one path.
        assert!(is_pure_filter("dirname", &args(&["/a/b"])));
        assert!(!is_pure_filter("dirname", &args(&["/a/b", "/c/d"])));
    }

    /// wc takes no operands at all: every one of them is a file it would read.
    #[test]
    fn a_program_with_no_operands_rejects_the_first_one() {
        assert!(is_pure_filter("wc", &args(&["-l"])));
        assert!(!is_pure_filter("wc", &args(&["anything"])));
    }

    /// Flags do not consume the operand budget, or `grep -i error` would look like two operands.
    #[test]
    fn flags_do_not_count_against_the_operand_budget() {
        assert!(is_pure_filter("grep", &args(&["-i", "-v", "error"])));
        assert!(!is_pure_filter(
            "grep",
            &args(&["-i", "-v", "error", "f.txt"])
        ));
    }

    /// `pwd` takes nothing and reports what the user established.
    #[test]
    fn pwd_qualifies() {
        assert!(is_pure_filter("pwd", &args(&[])));
        assert!(is_pure_filter("pwd", &args(&["-P"])));
    }

    /// Nothing may appear in both tables, or the answer would depend on which was consulted first.
    #[test]
    fn the_tables_do_not_overlap() {
        for filter in FILTERS {
            assert!(
                !NEVER.contains(&filter.program),
                "{} is both eligible and forbidden",
                filter.program
            );
        }
    }

    /// Every denied flag must belong to a program that is otherwise eligible, or it is describing a
    /// rule nothing applies.
    #[test]
    fn denied_flags_look_like_flags() {
        for filter in FILTERS {
            for flag in filter.denied {
                assert!(
                    flag.starts_with('-'),
                    "{}: '{flag}' is not a flag",
                    filter.program
                );
            }
        }
    }
}
