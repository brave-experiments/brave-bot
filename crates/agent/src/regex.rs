//! Regular expressions for `search`, matched without backtracking.
//!
//! A pattern arrives through a turn, so the engine that runs it is attack surface. The usual
//! objection to regular expressions here is catastrophic backtracking, where `(a+)+$` against a
//! run of `a`s costs exponential time. That is a property of backtracking implementations, not of
//! regular expressions: this one simulates a Thompson NFA, advancing a set of states one
//! character at a time, so a pattern is matched in time proportional to the length of the line
//! times the size of the pattern and there is no input that makes it blow up.
//!
//! Hand-written rather than a dependency, and deliberately small. Supported, and nothing else:
//!
//! - literal characters
//! - `.`, any character except a newline
//! - `*`, `+`, `?`
//! - `|`, and `(...)` to group
//! - `[abc]`, `[a-z]`, `[^abc]`
//! - `\d`, `\w`, `\s` and their negations `\D`, `\W`, `\S`
//! - `^` and `$`, anchoring to the ends of a line
//! - `\b` and `\B`, word boundaries
//! - a backslash before punctuation, matching it literally
//!
//! Counted repetition (`a{3}`, `a{2,9}`) is absent on purpose, and `{` is an ordinary character.
//! Nesting two of them multiplies the states one expands to, which is the one construct that
//! turns a short pattern into an enormous program, and no bound on the pattern's length would
//! bound the work. Backreferences are absent because they are not regular: no automaton
//! recognises them, and matching one needs the backtracking this engine exists to avoid.
//!
//! Captures are not extracted. `search` reports the line a match was found on, so whether a
//! pattern matched is the whole question, and a boolean is all any of this has to answer.

use std::fmt;

/// The longest pattern that will be compiled.
///
/// Every construct here expands to a number of states linear in the pattern's length, so this is
/// also the bound on the program, and with it the bound on the work any one line costs.
const MAX_PATTERN: usize = 1_000;

/// How deeply groups may nest.
///
/// The parser is recursive descent, so this is what keeps `((((...))))` from exhausting the
/// stack. The matcher itself does not recurse.
const MAX_DEPTH: usize = 50;

#[derive(Debug, PartialEq, Eq)]
pub enum PatternError {
    TooLong {
        length: usize,
    },
    TooDeep,
    UnclosedGroup,
    UnmatchedParen,
    UnclosedClass,
    EmptyClass,
    /// A `*`, `+` or `?` with nothing before it to repeat.
    NothingToRepeat {
        at: usize,
    },
    /// A backslash at the end of the pattern, promising an escape that never arrived.
    DanglingEscape,
    /// A range whose end sorts before its start, like `[z-a]`.
    ReversedRange {
        start: char,
        end: char,
    },
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLong { length } => write!(
                f,
                "the pattern is {length} characters, and {MAX_PATTERN} is the most that is matched"
            ),
            Self::TooDeep => write!(f, "the pattern nests groups more than {MAX_DEPTH} deep"),
            Self::UnclosedGroup => write!(f, "a '(' was never closed"),
            Self::UnmatchedParen => write!(f, "a ')' closes a group that was never opened"),
            Self::UnclosedClass => write!(f, "a '[' was never closed"),
            Self::EmptyClass => write!(f, "'[]' matches nothing at all"),
            Self::NothingToRepeat { at } => write!(
                f,
                "the repeat at character {at} has nothing before it to repeat"
            ),
            Self::DanglingEscape => write!(f, "the pattern ends with a '\\' and nothing to escape"),
            Self::ReversedRange { start, end } => {
                write!(f, "the range '{start}-{end}' runs backwards")
            }
        }
    }
}

impl std::error::Error for PatternError {}

/// A named set of characters, as `\d` and friends abbreviate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shorthand {
    Digit,
    Word,
    Space,
}

impl Shorthand {
    fn holds(self, c: char) -> bool {
        match self {
            Self::Digit => c.is_ascii_digit(),
            // `_` counts as a word character, as it does everywhere else, so `\w+` matches an
            // identifier rather than stopping in the middle of one.
            Self::Word => c.is_alphanumeric() || c == '_',
            Self::Space => c.is_whitespace(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Member {
    /// Inclusive, and a single character is a range of one.
    Range(char, char),
    Named(Shorthand, bool),
}

impl Member {
    fn holds(&self, c: char) -> bool {
        match self {
            Self::Range(start, end) => c >= *start && c <= *end,
            Self::Named(named, negated) => named.holds(c) != *negated,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Class {
    negated: bool,
    members: Vec<Member>,
}

impl Class {
    fn holds(&self, c: char) -> bool {
        self.members.iter().any(|m| m.holds(c)) != self.negated
    }

    /// Whether the class matches `c` with case ignored.
    ///
    /// The fold applies to the membership test and the negation is applied to the result, which is
    /// the whole subtlety: trying both cases against `holds` would make `[^a-z]` match `A`,
    /// because the lowercase of `A` is outside the negated set. Widening a set has to widen what
    /// it excludes too.
    fn holds_folded(&self, c: char) -> bool {
        let member = self
            .members
            .iter()
            .any(|m| m.holds(c) || m.holds(lower(c)) || m.holds(upper(c)));
        member != self.negated
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    /// Matches without consuming anything, which is what an empty alternative branch is.
    Empty,
    Char(char),
    Any,
    Class(Class),
    Concat(Vec<Node>),
    Alt(Vec<Node>),
    Star(Box<Node>),
    Plus(Box<Node>),
    Opt(Box<Node>),
    LineStart,
    LineEnd,
    /// `\b`, or `\B` when negated.
    Boundary(bool),
}

/// One step of the compiled program.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Inst {
    Char(char),
    Any,
    Class(Class),
    /// Continue as both threads. The whole reason the running time is bounded: a split is two
    /// states in one set, not two attempts one after the other.
    Split(usize, usize),
    Jump(usize),
    LineStart,
    LineEnd,
    Boundary(bool),
    Match,
}

/// A compiled pattern.
#[derive(Debug, Clone)]
pub struct Regex {
    program: Vec<Inst>,
    /// Fold both the pattern and the subject before comparing.
    ///
    /// Handled here rather than by lowercasing the pattern string, which would rewrite `\D`, `\W`
    /// and `\S` into the classes they are the negations of and quietly inverting what the search
    /// asked for.
    folded: bool,
}

impl Regex {
    /// Compile `pattern`, or say why it cannot be.
    pub fn compile(pattern: &str) -> Result<Self, PatternError> {
        Self::build(pattern, false)
    }

    /// Compile `pattern` to match without regard to case.
    pub fn compile_folded(pattern: &str) -> Result<Self, PatternError> {
        Self::build(pattern, true)
    }

    fn build(pattern: &str, folded: bool) -> Result<Self, PatternError> {
        let chars: Vec<char> = pattern.chars().collect();
        if chars.len() > MAX_PATTERN {
            return Err(PatternError::TooLong {
                length: chars.len(),
            });
        }

        let mut parser = Parser {
            chars: &chars,
            at: 0,
            depth: 0,
        };
        let node = parser.alternation()?;
        if parser.at < chars.len() {
            // The only way to stop early is a ')' with nothing open.
            return Err(PatternError::UnmatchedParen);
        }

        let mut program = Vec::new();
        emit(&node, &mut program);
        program.push(Inst::Match);
        Ok(Self { program, folded })
    }

    /// Whether `text` holds a match anywhere in it.
    ///
    /// Unanchored, so a thread starts at every position unless `^` says otherwise. The state set
    /// is deduplicated at each position, so the work is bounded by the line's length times the
    /// program's size however many threads the pattern would like to spawn.
    pub fn matches(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().collect();
        let mut current: Vec<usize> = Vec::new();
        let mut next: Vec<usize> = Vec::new();
        let mut on_current = vec![false; self.program.len()];
        let mut on_next = vec![false; self.program.len()];

        for position in 0..=chars.len() {
            // A fresh attempt beginning here, which is what makes the search unanchored. Where
            // the pattern starts with `^` the assertion kills this thread at once for every
            // position but the first.
            self.follow(0, position, &chars, &mut current, &mut on_current);

            if current.iter().any(|&pc| self.program[pc] == Inst::Match) {
                return true;
            }
            if position == chars.len() {
                break;
            }

            let c = chars[position];
            next.clear();
            on_next.iter_mut().for_each(|seen| *seen = false);
            for &pc in &current {
                let consumes = match &self.program[pc] {
                    Inst::Char(want) => self.same(*want, c),
                    // A line never holds its own terminator, but a search may be handed text
                    // that does, and `.` is not supposed to cross one.
                    Inst::Any => c != '\n',
                    Inst::Class(class) => {
                        if self.folded {
                            class.holds_folded(c)
                        } else {
                            class.holds(c)
                        }
                    }
                    _ => continue,
                };
                if consumes {
                    self.follow(pc + 1, position + 1, &chars, &mut next, &mut on_next);
                }
            }

            std::mem::swap(&mut current, &mut next);
            std::mem::swap(&mut on_current, &mut on_next);
        }

        false
    }

    /// Whether the pattern's `want` matches the subject's `c`, folding if the search asked for it.
    fn same(&self, want: char, c: char) -> bool {
        want == c || (self.folded && lower(want) == lower(c))
    }

    /// Add `pc` and everything reachable from it without consuming a character.
    ///
    /// Iterative rather than recursive: the closure of a pattern like `(a*)*` reaches deep, and a
    /// pattern arriving through a turn must not be able to choose the depth of this stack.
    fn follow(
        &self,
        pc: usize,
        position: usize,
        chars: &[char],
        list: &mut Vec<usize>,
        seen: &mut [bool],
    ) {
        let mut pending = vec![pc];
        while let Some(pc) = pending.pop() {
            if seen[pc] {
                continue;
            }
            seen[pc] = true;
            match &self.program[pc] {
                Inst::Jump(to) => pending.push(*to),
                Inst::Split(a, b) => {
                    pending.push(*a);
                    pending.push(*b);
                }
                Inst::LineStart => {
                    if position == 0 {
                        pending.push(pc + 1);
                    }
                }
                Inst::LineEnd => {
                    if position == chars.len() {
                        pending.push(pc + 1);
                    }
                }
                Inst::Boundary(negated) => {
                    let word = |c: char| Shorthand::Word.holds(c);
                    let before = position > 0 && word(chars[position - 1]);
                    let after = position < chars.len() && word(chars[position]);
                    if ((before != after) != *negated) || (before && after && *negated) {
                        pending.push(pc + 1);
                    }
                }
                // Consumes a character, so it stays in the set for the step to deal with.
                Inst::Char(_) | Inst::Any | Inst::Class(_) | Inst::Match => list.push(pc),
            }
        }
    }
}

/// Compile `node` onto the end of `program`.
fn emit(node: &Node, program: &mut Vec<Inst>) {
    match node {
        Node::Empty => {}
        Node::Char(c) => program.push(Inst::Char(*c)),
        Node::Any => program.push(Inst::Any),
        Node::Class(class) => program.push(Inst::Class(class.clone())),
        Node::LineStart => program.push(Inst::LineStart),
        Node::LineEnd => program.push(Inst::LineEnd),
        Node::Boundary(negated) => program.push(Inst::Boundary(*negated)),
        Node::Concat(nodes) => {
            for node in nodes {
                emit(node, program);
            }
        }
        Node::Alt(branches) => {
            // Each branch gets a split choosing between it and everything after it, and ends with
            // a jump to the common exit, which is patched once every branch is placed.
            let mut exits = Vec::new();
            for (index, branch) in branches.iter().enumerate() {
                if index + 1 == branches.len() {
                    emit(branch, program);
                    break;
                }
                let split = program.len();
                program.push(Inst::Jump(0));
                emit(branch, program);
                let exit = program.len();
                program.push(Inst::Jump(0));
                exits.push(exit);
                let rest = program.len();
                program[split] = Inst::Split(split + 1, rest);
            }
            let end = program.len();
            for exit in exits {
                program[exit] = Inst::Jump(end);
            }
        }
        Node::Star(inner) => {
            let split = program.len();
            program.push(Inst::Jump(0));
            emit(inner, program);
            program.push(Inst::Jump(split));
            let after = program.len();
            program[split] = Inst::Split(split + 1, after);
        }
        Node::Plus(inner) => {
            let start = program.len();
            emit(inner, program);
            let split = program.len();
            program.push(Inst::Jump(0));
            let after = program.len();
            program[split] = Inst::Split(start, after);
        }
        Node::Opt(inner) => {
            let split = program.len();
            program.push(Inst::Jump(0));
            emit(inner, program);
            let after = program.len();
            program[split] = Inst::Split(split + 1, after);
        }
    }
}

struct Parser<'p> {
    chars: &'p [char],
    at: usize,
    depth: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    /// `a|b|c`, the loosest-binding construct and so the outermost.
    fn alternation(&mut self) -> Result<Node, PatternError> {
        let mut branches = vec![self.concatenation()?];
        while self.peek() == Some('|') {
            self.at += 1;
            branches.push(self.concatenation()?);
        }
        if branches.len() == 1 {
            // The length was checked on the line above.
            // nosemgrep: trailofbits.rs.panic-in-function-returning-result.panic-in-function-returning-result
            return Ok(branches.pop().expect("one branch"));
        }
        Ok(Node::Alt(branches))
    }

    /// A run of repeated atoms, stopping at `|` or the `)` that closes the group above.
    fn concatenation(&mut self) -> Result<Node, PatternError> {
        let mut nodes = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            nodes.push(self.repeated()?);
        }
        match nodes.len() {
            0 => Ok(Node::Empty),
            // The arm matched on the length, so the pop is checked by construction.
            // nosemgrep: trailofbits.rs.panic-in-function-returning-result.panic-in-function-returning-result
            1 => Ok(nodes.pop().expect("one node")),
            _ => Ok(Node::Concat(nodes)),
        }
    }

    /// An atom and whatever repeats it.
    fn repeated(&mut self) -> Result<Node, PatternError> {
        let at = self.at;
        let (atom, repeatable) = self.atom()?;

        let mut node = atom;
        while let Some(c) = self.peek() {
            let wrap = match c {
                '*' => Node::Star(Box::new(node)),
                '+' => Node::Plus(Box::new(node)),
                '?' => Node::Opt(Box::new(node)),
                _ => break,
            };
            if !repeatable {
                return Err(PatternError::NothingToRepeat { at });
            }
            self.at += 1;
            node = wrap;
        }
        Ok(node)
    }

    /// One atom, and whether a `*`, `+` or `?` may follow it.
    ///
    /// A bare anchor may not be repeated: `^*` is a mistake rather than a pattern, and saying so
    /// beats compiling something nobody intended. Wrapped in a group it may, because `(\B)?` is
    /// then an ordinary optional subexpression and every engine accepts it.
    fn atom(&mut self) -> Result<(Node, bool), PatternError> {
        let Some(c) = self.peek() else {
            return Ok((Node::Empty, true));
        };
        match c {
            '(' => {
                self.at += 1;
                self.depth += 1;
                if self.depth > MAX_DEPTH {
                    return Err(PatternError::TooDeep);
                }
                let inner = self.alternation()?;
                if self.peek() != Some(')') {
                    return Err(PatternError::UnclosedGroup);
                }
                self.at += 1;
                self.depth -= 1;
                Ok((inner, true))
            }
            '[' => Ok((self.class()?, true)),
            '.' => {
                self.at += 1;
                Ok((Node::Any, true))
            }
            '^' => {
                self.at += 1;
                Ok((Node::LineStart, false))
            }
            '$' => {
                self.at += 1;
                Ok((Node::LineEnd, false))
            }
            '*' | '+' | '?' => Err(PatternError::NothingToRepeat { at: self.at }),
            '\\' => {
                self.at += 1;
                let node = self.escape()?;
                let repeatable = !matches!(node, Node::Boundary(_));
                Ok((node, repeatable))
            }
            _ => {
                self.at += 1;
                Ok((Node::Char(c), true))
            }
        }
    }

    /// What a `\` introduces, the backslash already consumed.
    fn escape(&mut self) -> Result<Node, PatternError> {
        let Some(c) = self.peek() else {
            return Err(PatternError::DanglingEscape);
        };
        self.at += 1;
        Ok(match c {
            'd' => named(Shorthand::Digit, false),
            'D' => named(Shorthand::Digit, true),
            'w' => named(Shorthand::Word, false),
            'W' => named(Shorthand::Word, true),
            's' => named(Shorthand::Space, false),
            'S' => named(Shorthand::Space, true),
            'b' => Node::Boundary(false),
            'B' => Node::Boundary(true),
            'n' => Node::Char('\n'),
            't' => Node::Char('\t'),
            'r' => Node::Char('\r'),
            // Anything else stands for itself, which is what `\.` and `\\` are for.
            other => Node::Char(other),
        })
    }

    /// `[...]`, the `[` not yet consumed.
    fn class(&mut self) -> Result<Node, PatternError> {
        self.at += 1;
        let negated = self.peek() == Some('^');
        if negated {
            self.at += 1;
        }

        let mut members = Vec::new();
        // A `]` first thing is the character, not the end of an empty class, which is what every
        // other engine does with it.
        if self.peek() == Some(']') {
            members.push(Member::Range(']', ']'));
            self.at += 1;
        }

        loop {
            let Some(c) = self.peek() else {
                return Err(PatternError::UnclosedClass);
            };
            if c == ']' {
                self.at += 1;
                break;
            }
            self.at += 1;

            let start = if c == '\\' {
                match self.escaped_member()? {
                    Ok(named) => {
                        members.push(named);
                        continue;
                    }
                    Err(literal) => literal,
                }
            } else {
                c
            };

            // `a-z`, unless the `-` is the last thing before the `]`, where it is a character.
            if self.peek() == Some('-') && self.chars.get(self.at + 1).copied() != Some(']') {
                self.at += 1;
                let Some(end) = self.peek() else {
                    return Err(PatternError::UnclosedClass);
                };
                self.at += 1;
                let end = if end == '\\' {
                    match self.escaped_member()? {
                        // `[a-\d]` is not a range anybody meant, so the shorthand joins the class
                        // on its own and the `-` is left as a character.
                        Ok(named) => {
                            members.push(Member::Range(start, start));
                            members.push(Member::Range('-', '-'));
                            members.push(named);
                            continue;
                        }
                        Err(literal) => literal,
                    }
                } else {
                    end
                };
                if end < start {
                    return Err(PatternError::ReversedRange { start, end });
                }
                members.push(Member::Range(start, end));
            } else {
                members.push(Member::Range(start, start));
            }
        }

        if members.is_empty() {
            return Err(PatternError::EmptyClass);
        }
        Ok(Node::Class(Class { negated, members }))
    }

    /// An escape inside a class: either a named set, or a character standing for itself.
    ///
    /// The backslash is consumed by the caller. `Ok` is a set that joins the class whole, `Err`
    /// is a plain character that may still turn out to be one end of a range.
    fn escaped_member(&mut self) -> Result<Result<Member, char>, PatternError> {
        let Some(c) = self.peek() else {
            return Err(PatternError::DanglingEscape);
        };
        self.at += 1;
        Ok(match c {
            'd' => Ok(Member::Named(Shorthand::Digit, false)),
            'D' => Ok(Member::Named(Shorthand::Digit, true)),
            'w' => Ok(Member::Named(Shorthand::Word, false)),
            'W' => Ok(Member::Named(Shorthand::Word, true)),
            's' => Ok(Member::Named(Shorthand::Space, false)),
            'S' => Ok(Member::Named(Shorthand::Space, true)),
            'n' => Err('\n'),
            't' => Err('\t'),
            'r' => Err('\r'),
            other => Err(other),
        })
    }
}

/// The single-character lowercase of `c`, or `c` where folding it is not one character.
///
/// Anything whose mapping is longer stays as it is, which keeps a fold a comparison between two
/// characters rather than a rewrite of the subject.
fn lower(c: char) -> char {
    let mut mapped = c.to_lowercase();
    match (mapped.next(), mapped.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

fn upper(c: char) -> char {
    let mut mapped = c.to_uppercase();
    match (mapped.next(), mapped.next()) {
        (Some(single), None) => single,
        _ => c,
    }
}

fn named(shorthand: Shorthand, negated: bool) -> Node {
    Node::Class(Class {
        negated: false,
        members: vec![Member::Named(shorthand, negated)],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, text: &str) -> bool {
        Regex::compile(pattern)
            .expect("the pattern compiles")
            .matches(text)
    }

    #[test]
    fn a_literal_pattern_matches_the_substring_it_spells() {
        assert!(matches("needle", "a needle in a haystack"));
        assert!(!matches("needle", "a haystack"));
    }

    #[test]
    fn a_dot_matches_any_character_but_a_newline() {
        assert!(matches("a.c", "abc"));
        assert!(matches("a.c", "a c"));
        assert!(!matches("a.c", "ac"));
        assert!(!matches("a.c", "a\nc"));
    }

    #[test]
    fn a_star_matches_a_run_of_any_length_including_none() {
        assert!(matches("ab*c", "ac"));
        assert!(matches("ab*c", "abc"));
        assert!(matches("ab*c", "abbbbc"));
        assert!(!matches("ab*c", "adc"));
    }

    #[test]
    fn a_plus_needs_at_least_one() {
        assert!(!matches("ab+c", "ac"));
        assert!(matches("ab+c", "abc"));
    }

    #[test]
    fn a_question_mark_makes_what_precedes_it_optional() {
        assert!(matches("colou?r", "color"));
        assert!(matches("colou?r", "colour"));
    }

    #[test]
    fn alternation_matches_any_of_its_branches() {
        assert!(matches("cat|dog", "a dog"));
        assert!(matches("cat|dog", "a cat"));
        assert!(!matches("cat|dog", "a bird"));
    }

    #[test]
    fn a_group_can_be_repeated_as_a_unit() {
        assert!(matches("(ab)+c", "ababc"));
        assert!(!matches("(ab)+c", "abac"));
    }

    #[test]
    fn alternation_inside_a_group_binds_to_the_group() {
        assert!(matches("gr(a|e)y", "grey"));
        assert!(matches("gr(a|e)y", "gray"));
        assert!(!matches("gr(a|e)y", "groy"));
    }

    #[test]
    fn a_character_class_matches_any_member() {
        assert!(matches("[abc]x", "bx"));
        assert!(!matches("[abc]x", "dx"));
    }

    #[test]
    fn a_class_range_covers_its_endpoints() {
        assert!(matches("[a-f]", "f"));
        assert!(matches("[a-f]", "a"));
        assert!(!matches("[a-f]", "g"));
    }

    #[test]
    fn a_negated_class_matches_what_it_excludes() {
        assert!(matches("[^0-9]", "a"));
        assert!(!matches("[^0-9]", "7"));
    }

    #[test]
    fn a_shorthand_class_matches_its_set() {
        assert!(matches(r"\d\d", "x42"));
        assert!(!matches(r"\d\d", "x4y"));
        assert!(matches(r"\w+", "name_1"));
        assert!(matches(r"a\sb", "a b"));
        assert!(!matches(r"a\sb", "ab"));
    }

    #[test]
    fn a_negated_shorthand_matches_the_complement() {
        assert!(matches(r"\D", "a"));
        assert!(!matches(r"\D", "5"));
        assert!(matches(r"\S", "a"));
        assert!(!matches(r"\S", " "));
    }

    #[test]
    fn a_shorthand_can_be_a_member_of_a_class() {
        assert!(matches(r"[\d_]+", "4_2"));
        assert!(!matches(r"[\d_]", "a"));
        assert!(matches(r"[^\d]", "a"));
        assert!(!matches(r"[^\d]", "4"));
    }

    #[test]
    fn anchors_tie_a_match_to_the_ends_of_the_line() {
        assert!(matches("^fn ", "fn main"));
        assert!(!matches("^fn ", "  fn main"));
        assert!(matches(";$", "let x = 1;"));
        assert!(!matches(";$", "let x = 1; // trailing"));
        assert!(matches("^$", ""));
        assert!(!matches("^$", "x"));
    }

    #[test]
    fn a_word_boundary_matches_between_a_word_and_what_is_not_one() {
        assert!(matches(r"\bcat\b", "the cat sat"));
        assert!(!matches(r"\bcat\b", "concatenate"));
        assert!(matches(r"\Bcat\B", "concatenate"));
        assert!(!matches(r"\Bcat\B", "the cat sat"));
    }

    #[test]
    fn an_escaped_metacharacter_matches_itself() {
        assert!(matches(r"a\.c", "a.c"));
        assert!(!matches(r"a\.c", "abc"));
        assert!(matches(r"\(\)", "()"));
        assert!(matches(r"2\+2", "2+2"));
        assert!(matches(r"a\\b", r"a\b"));
    }

    #[test]
    fn a_brace_is_an_ordinary_character() {
        // Counted repetition is not supported, so this is a literal and says so by matching one.
        assert!(matches("a{2}", "a{2}"));
        assert!(!matches("a{2}", "aa"));
    }

    /// Folding by lowercasing the pattern string would turn `\D` into `\d` and match the opposite
    /// of what was asked for, which is why the engine folds the comparison instead.
    #[test]
    fn a_folded_pattern_keeps_a_negated_shorthand_negated() {
        let folded = Regex::compile_folded(r"\D").expect("compiles");
        assert!(folded.matches("a"), "a letter is not a digit");
        assert!(!folded.matches("5"), "a digit matched the negation of one");

        let folded = Regex::compile_folded(r"\W+").expect("compiles");
        assert!(folded.matches("!!"));
        assert!(!folded.matches("abc"));
    }

    /// Folding widens what a class matches, so it has to widen what a negated class excludes by
    /// the same set. Testing both cases of the subject against the finished membership decision
    /// instead makes `[^a-z]` match `A`, because `a` is outside the set the negation describes.
    #[test]
    fn folding_a_negated_class_widens_what_it_excludes() {
        let folded = Regex::compile_folded("[^a-z]").expect("compiles");
        assert!(
            !folded.matches("A"),
            "an uppercase letter escaped a negated lowercase range"
        );
        assert!(!folded.matches("z"));
        assert!(
            folded.matches("9"),
            "a digit is outside the range either way"
        );

        let folded = Regex::compile_folded("[^A-Z]+").expect("compiles");
        assert!(!folded.matches("abc"));
        assert!(folded.matches("123"));
    }

    #[test]
    fn a_folded_pattern_matches_either_case() {
        let folded = Regex::compile_folded("fn Main").expect("compiles");
        assert!(folded.matches("FN MAIN"));
        assert!(folded.matches("fn main"));
        assert!(!folded.matches("fn other"));

        // A class folds too, so `[a-z]` covers an uppercase letter without being rewritten.
        let class = Regex::compile_folded("[a-z]+").expect("compiles");
        assert!(class.matches("ABC"));

        // Case-sensitive by default, so the fold is something a caller asks for.
        assert!(
            !Regex::compile("fn Main")
                .expect("compiles")
                .matches("FN MAIN")
        );
    }

    #[test]
    fn a_pattern_matches_across_a_multibyte_character() {
        assert!(matches("é.", "éx"));
        assert!(matches("[é]", "é"));
        assert!(matches(".", "日"));
    }

    #[test]
    fn an_empty_alternative_branch_matches_nothing_at_all() {
        assert!(matches("a(b|)c", "ac"));
        assert!(matches("a(b|)c", "abc"));
    }

    /// The property the whole engine exists for. A backtracking implementation takes
    /// exponential time on this, and the test would not finish rather than fail.
    #[test]
    fn a_pattern_built_to_backtrack_catastrophically_still_returns_promptly() {
        let started = std::time::Instant::now();
        let subject = "a".repeat(60);
        assert!(!matches("(a+)+$b", &subject));
        assert!(!matches("(a|aa)*c", &subject));
        assert!(!matches("(x+x+)+y", &"x".repeat(60)));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "matching took {:?}, which is the backtracking blowup this engine rules out",
            started.elapsed()
        );
    }

    #[test]
    fn a_nested_star_terminates_rather_than_looping_on_the_empty_match() {
        assert!(matches("(a*)*b", "aaab"));
        assert!(matches("(a*)*", ""));
        assert!(!matches("(a*)*b", "ccc"));
    }

    #[test]
    fn an_unclosed_group_is_reported_rather_than_guessed_at() {
        assert_eq!(
            Regex::compile("(ab").unwrap_err(),
            PatternError::UnclosedGroup
        );
        assert_eq!(
            Regex::compile("ab)").unwrap_err(),
            PatternError::UnmatchedParen
        );
    }

    #[test]
    fn an_unclosed_class_is_reported() {
        assert_eq!(
            Regex::compile("[abc").unwrap_err(),
            PatternError::UnclosedClass
        );
        assert_eq!(
            Regex::compile("[]").unwrap_err(),
            PatternError::UnclosedClass
        );
    }

    #[test]
    fn a_repeat_with_nothing_before_it_is_reported() {
        assert_eq!(
            Regex::compile("*ab").unwrap_err(),
            PatternError::NothingToRepeat { at: 0 }
        );
        assert_eq!(
            Regex::compile("^*").unwrap_err(),
            PatternError::NothingToRepeat { at: 0 }
        );
    }

    /// A bare anchor cannot be repeated, but a group holding one is an ordinary subexpression and
    /// every other engine accepts a `?` after it. Refusing the second on the strength of the
    /// first rejects patterns that are not mistakes.
    #[test]
    fn a_group_holding_an_anchor_may_still_be_repeated() {
        assert!(matches(r"(\B)?a", "a"));
        assert!(matches("(^)?a", "a"));
        assert!(matches("(a$)?b", "b"));
        assert_eq!(
            Regex::compile(r"\B*").unwrap_err(),
            PatternError::NothingToRepeat { at: 0 }
        );
    }

    #[test]
    fn a_dangling_escape_is_reported() {
        assert_eq!(
            Regex::compile(r"ab\").unwrap_err(),
            PatternError::DanglingEscape
        );
    }

    #[test]
    fn a_backwards_range_is_reported() {
        assert_eq!(
            Regex::compile("[z-a]").unwrap_err(),
            PatternError::ReversedRange {
                start: 'z',
                end: 'a'
            }
        );
    }

    #[test]
    fn a_pattern_past_the_length_cap_is_refused() {
        let long = "a".repeat(MAX_PATTERN + 1);
        assert_eq!(
            Regex::compile(&long).unwrap_err(),
            PatternError::TooLong {
                length: MAX_PATTERN + 1
            }
        );
    }

    /// Recursive descent parses the groups, so an absurdly nested pattern has to be refused
    /// rather than allowed to exhaust the stack.
    #[test]
    fn a_pattern_nested_past_the_depth_cap_is_refused() {
        let deep = format!(
            "{}a{}",
            "(".repeat(MAX_DEPTH + 1),
            ")".repeat(MAX_DEPTH + 1)
        );
        assert_eq!(Regex::compile(&deep).unwrap_err(), PatternError::TooDeep);
    }

    #[test]
    fn a_hyphen_at_either_end_of_a_class_is_a_character() {
        assert!(matches("[a-]", "-"));
        assert!(matches("[-a]", "-"));
        assert!(matches("[a-]", "a"));
    }

    #[test]
    fn a_closing_bracket_first_in_a_class_is_a_character() {
        assert!(matches("[]]", "]"));
        assert!(!matches("[]]", "a"));
    }
}
