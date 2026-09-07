//! The grammar of a command line the planner writes.
//!
//! The planner spells a command the way a person would, and this module is the only thing that
//! ever reads that spelling. It parses the line into a syntax tree, and that tree is what the rest
//! of the compile works from. No shell is started at any point, and no part of the line is ever
//! concatenated back into a string that something else parses.
//!
//! # Why parsing here is not the trap that parsing a shell string is
//!
//! The objection to a shell string is not that it contains dangerous characters. It is that a
//! parser reading one is *predicting* what some other program will do with it, and losing that
//! race means a person approved one thing and another ran. Nothing here predicts anything: no
//! shell will ever see the line, this module is its only reader, and what comes out of it is the
//! whole of what happens.
//!
//! # The grammar is closed
//!
//! [`parse`] accepts exactly the constructs listed on it and refuses everything else, naming the
//! span that caused the refusal. A refusal returns an error and produces no tree, so there is no
//! partial plan for a caller to run: the type is the guarantee. There is no degraded mode and no
//! fallback to a real shell, because a compiler that can be made to give up and hand the string to
//! one is a shell with extra steps.
//!
//! The refusals are not a filter over text that looks dangerous. They are the constructs whose
//! meaning is not in the line. `$(date)` and `$HOME` both stand for something that has to be
//! worked out somewhere else, so a person shown the line has not been shown the plan. Quoting
//! makes every one of them ordinary text, because a quoted `$` is a dollar sign and nothing more.

use bravebot_core::command::{Joiner, Plan, Route, Step, Steps};
use std::fmt;
use std::path::{Path, PathBuf};

/// Where in the line something is, as a byte range.
///
/// Byte offsets rather than character positions, so a span indexes the line directly and a caller
/// wanting the text can slice it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// One part of a word.
///
/// A word is a sequence of these rather than a string, because quoting decides meaning and the
/// distinction has to survive parsing: `*` is a pattern and `'*'` is an asterisk, and by the time
/// either is a `String` there is no telling them apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    /// Characters standing for themselves. Anything quoted arrives here whatever it was.
    Text(String),
    /// An unquoted `*`: any run of characters within one path segment.
    Any,
    /// An unquoted `?`: one character.
    One,
    /// An unquoted `**`: any run of characters, across segment boundaries.
    Tree,
    /// An unquoted `[…]`, holding what was between the brackets.
    Class(String),
    /// An unquoted `{a,b}`, holding one piece list per alternative.
    Alternatives(Vec<Vec<Piece>>),
    /// An unquoted `{1..9}`.
    ///
    /// `width` is the number of digits to pad each number to, and is zero where the endpoints were
    /// not padded.
    Range { from: i64, to: i64, width: usize },
    /// A leading unquoted `~`.
    Home,
}

/// One word of a command: its program, an operand, or a redirection's target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Word {
    pub pieces: Vec<Piece>,
    pub span: Span,
}

impl Word {
    /// The text this word is, where it is text and nothing else.
    ///
    /// `None` for a word carrying a pattern, a brace expression or a `~`, because those stand for
    /// something the compile works out later and there is no text they already are.
    pub fn literal(&self) -> Option<String> {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Text(text) => out.push_str(text),
                _ => return None,
            }
        }
        Some(out)
    }
}

/// `NAME=literal` in front of a command's program.
///
/// The value is a `String` rather than a [`Word`] because it has to be literal: a value that
/// expanded would put something in the environment that the endorsed plan did not show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub name: String,
    pub value: String,
    pub span: Span,
}

/// Where one of a command's streams goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirection {
    /// `> file`, or `>> file` when appending.
    Stdout { target: Word, append: bool },
    /// `< file`.
    Stdin { target: Word },
    /// `2> file`, or `2>> file` when appending.
    Stderr { target: Word, append: bool },
    /// `&> file`: both streams into one file.
    Both { target: Word },
    /// `2>&1`: standard error joins standard output, touching no file.
    StderrToStdout,
}

/// One command: its environment, its program and operands, and its redirections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub assignments: Vec<Assignment>,
    /// The program first, then its operands.
    pub words: Vec<Word>,
    pub redirections: Vec<Redirection>,
    pub span: Span,
}

impl Command {
    /// The word naming the program.
    pub fn program(&self) -> &Word {
        // A command with no words is a syntax error and never reaches a caller, so the first word
        // is always there.
        &self.words[0]
    }
}

/// A parsed line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// One command on its own.
    Command(Command),
    /// Commands joined by `|`, each feeding the next. Always two or more.
    Pipeline(Vec<Command>),
    /// Two nodes joined by `&&`, `||` or `;`.
    Join {
        left: Box<Node>,
        joiner: Joiner,
        right: Box<Node>,
    },
    /// `( … )`, which groups for sequencing and starts no subshell.
    Group(Box<Node>),
}

/// Why a line will not compile.
///
/// Carries the offending text as well as its span, because the error travels without the line and
/// a reader who cannot see what was refused cannot fix it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub span: Span,
    pub text: String,
    pub reason: Reason,
}

/// What was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reason {
    /// `$(…)` or a backtick.
    CommandSubstitution,
    /// `<(…)` or `>(…)`.
    ProcessSubstitution,
    /// `$VAR` or `${…}`.
    Variable,
    /// `$((…))`.
    Arithmetic,
    /// A bare `&`.
    Background,
    /// `<<` or `<<<`.
    HereDocument,
    /// A word in command position that would reintroduce interpretation.
    Interpreter(&'static str),
    /// A reserved word that opens a control structure.
    ControlFlow(&'static str),
    /// An unquoted `!`.
    HistoryExpansion,
    /// A descriptor duplication other than `2>&1`.
    Descriptor,
    /// A redirection numbered anything but 2.
    IoNumber,
    /// `~` followed by something other than `/`.
    NamedHome,
    /// A quote or a bracket that is never closed.
    Unclosed(char),
    /// A `NAME=` whose value is not literal text.
    AssignmentValue,
    /// A pattern that matched no file.
    NoMatch,
    /// A word standing for more arguments than a person can be shown at one prompt.
    TooMany { found: usize, cap: usize },
    /// A walk that read more directories than the ceiling allows.
    TooDeep { cap: usize },
    /// A `~` where this user has no home directory.
    NoHome,
    /// A program word standing for more or less than one thing.
    NotOneProgram,
    /// A program name nothing on `$PATH` matches.
    NotFound,
    /// A redirection target that is not exactly one file.
    NotOnePath,
    /// An invocation that would want a terminal, with what to do instead.
    Interactive(&'static str),
    /// More commands in one line than a plan can put to a person at once.
    TooManySteps { cap: usize },
    /// Groups nested deeper than the grammar goes.
    TooNested { cap: usize },
    /// Anything the shape of the line gets wrong.
    Syntax(&'static str),
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandSubstitution => f.write_str(
                "a command whose text is computed is a destination nobody saw. Write the value out",
            ),
            Self::ProcessSubstitution => f.write_str(
                "process substitution names a file descriptor nobody named. Use a pipe, or a file",
            ),
            Self::Variable => f.write_str(
                "the value is not in the line, so the plan would not be in the line. Write the value out",
            ),
            Self::Arithmetic => {
                f.write_str("arithmetic is a language, and a language needs an interpreter")
            }
            Self::Background => f.write_str(
                "backgrounding is a parameter of the call rather than a token in the line",
            ),
            Self::HereDocument => f.write_str(
                "a here-document is content wearing the shape of syntax. Put the content in a file, or on standard input",
            ),
            Self::Interpreter(word) => write!(
                f,
                "`{word}` reintroduces interpretation by name. Run the program you mean instead"
            ),
            Self::ControlFlow(word) => {
                write!(f, "`{word}` opens a control structure, and control flow is a program")
            }
            Self::HistoryExpansion => f.write_str(
                "an unquoted `!` is history expansion. Quote it where you mean the character",
            ),
            Self::Descriptor => {
                f.write_str("`2>&1` is the only descriptor duplication this grammar has")
            }
            Self::IoNumber => f.write_str("only standard error may be redirected by number"),
            Self::NamedHome => {
                f.write_str("`~` is understood on its own or before a `/`, and not otherwise")
            }
            Self::Unclosed(c) => write!(f, "`{c}` is never closed"),
            Self::AssignmentValue => f.write_str(
                "an assignment's value must be literal text, so that the plan shows what the program will see",
            ),
            Self::NoMatch => f.write_str(
                "matched no file. A pattern standing for nothing is not an argument, so there is no plan to show",
            ),
            Self::TooMany { found, cap } => write!(
                f,
                "stands for {found} arguments, and at most {cap} can be put to a person at one prompt"
            ),
            Self::TooDeep { cap } => write!(
                f,
                "would read more than {cap} directories to work out what it matches. Name a narrower pattern"
            ),
            Self::NoHome => f.write_str("this user has no home directory to stand for"),
            Self::NotOneProgram => {
                f.write_str("a step's program has to be one thing, so that the plan can name it")
            }
            Self::NotFound => f.write_str(
                "is not a program that could be found. `$PATH` decides what a bare name means, and nothing on it matches",
            ),
            Self::NotOnePath => f.write_str(
                "a redirection has to name exactly one file. A destination worked out from what is on disk is a destination that moves when the tree does",
            ),
            Self::Interactive(alternative) => f.write_str(alternative),
            Self::TooManySteps { cap } => write!(
                f,
                "a line may hold at most {cap} commands. A plan longer than that is a prompt nobody reads"
            ),
            Self::TooNested { cap } => {
                write!(f, "groups may be nested at most {cap} deep")
            }
            Self::Syntax(detail) => f.write_str(detail),
        }
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "`{}` at offset {}: {}",
            self.text, self.span.start, self.reason
        )
    }
}

impl std::error::Error for Refused {}

/// How many commands one line may hold.
///
/// A reader has to take the whole plan in before answering for it, and a plan with hundreds of
/// steps in it is a prompt that grants everything. It also bounds the tree, so neither compiling
/// nor running a line can recurse further than this.
pub const MAX_STEPS: usize = 64;

/// How deeply `( … )` may nest.
pub const MAX_NESTING: usize = 16;

/// Words that would put an interpreter back in the plan.
const INTERPRETERS: [&str; 5] = ["eval", "source", ".", "exec", "trap"];

/// Reserved words that open a control structure.
const CONTROL_FLOW: [&str; 5] = ["if", "while", "for", "case", "function"];

/// Parse a command line.
///
/// Accepted, and nothing else:
///
/// - a simple command, `word...`
/// - a pipeline, `cmd | cmd`
/// - sequencing with `&&`, `||` and `;`
/// - grouping with `( … )`, for sequencing, with no subshell of its own
/// - quoting with `'…'`, `"…"` and `\`
/// - redirection with `>`, `>>`, `<`, `2>`, `2>>`, `2>&1` and `&>`, to a literal target
/// - globs, `*`, `?`, `[…]` and `**`, in operand position
/// - brace expansion, `{a,b}` and `{1..9}`
/// - a leading `~`
/// - per-command environment, `NAME=literal cmd`
///
/// Everything else is a [`Refused`], and a refusal yields no tree at all.
pub fn parse(line: &str) -> Result<Node, Refused> {
    // Trailing space is not part of anything, and trimming only the end leaves every offset in
    // the line where a caller would find it.
    let line = line.trim_end();
    if line.is_empty() {
        return Err(refuse(
            line,
            Span::new(0, 0),
            Reason::Syntax("there is nothing to run"),
        ));
    }
    let tokens = Scan::new(line).tokens()?;
    let mut parser = Parser {
        line,
        tokens,
        at: 0,
        commands: 0,
        nesting: 0,
    };
    let node = parser.sequence()?;
    if let Some(token) = parser.peek() {
        return Err(Refused {
            span: token.span(),
            text: token.text(),
            reason: Reason::Syntax("nothing this could belong to"),
        });
    }
    Ok(node)
}

/// One indivisible piece of the line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Tok {
    Word(Word),
    Pipe(Span),
    AndIf(Span),
    OrIf(Span),
    Semi(Span),
    Open(Span),
    Close(Span),
    Redirect(Op, Span),
}

/// A redirection operator, before its target has been read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Out,
    OutAppend,
    In,
    Err,
    ErrAppend,
    Both,
    ErrToOut,
}

impl Tok {
    fn span(&self) -> Span {
        match self {
            Self::Word(word) => word.span,
            Self::Pipe(span)
            | Self::AndIf(span)
            | Self::OrIf(span)
            | Self::Semi(span)
            | Self::Open(span)
            | Self::Close(span)
            | Self::Redirect(_, span) => *span,
        }
    }

    /// How the token reads in an error, which is the operator itself or the word's own spelling.
    fn text(&self) -> String {
        match self {
            Self::Word(word) => word.literal().unwrap_or_else(|| "a pattern".to_string()),
            Self::Pipe(_) => "|".to_string(),
            Self::AndIf(_) => "&&".to_string(),
            Self::OrIf(_) => "||".to_string(),
            Self::Semi(_) => ";".to_string(),
            Self::Open(_) => "(".to_string(),
            Self::Close(_) => ")".to_string(),
            Self::Redirect(op, _) => match op {
                Op::Out => ">",
                Op::OutAppend => ">>",
                Op::In => "<",
                Op::Err => "2>",
                Op::ErrAppend => "2>>",
                Op::Both => "&>",
                Op::ErrToOut => "2>&1",
            }
            .to_string(),
        }
    }
}

/// Build a refusal, taking the offending text out of the line.
fn refuse(line: &str, span: Span, reason: Reason) -> Refused {
    Refused {
        span,
        text: line
            .get(span.start..span.end)
            .unwrap_or_default()
            .to_string(),
        reason,
    }
}

/// The text a run of pieces is, where they are all text.
fn literal_of(pieces: &[Piece]) -> Option<String> {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => out.push_str(text),
            _ => return None,
        }
    }
    Some(out)
}

/// Whether an unquoted character ends the word being scanned.
fn ends_word(c: char) -> bool {
    c.is_whitespace() || matches!(c, '|' | '&' | ';' | '(' | ')' | '<' | '>')
}

/// Whether `name` may stand to the left of a `=` in a per-command assignment.
fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The name and value pieces of `word`, where it has the shape of an assignment.
///
/// The value is returned unjudged: a value that is not literal text is a refusal rather than an
/// ordinary word, so the caller reports it instead of quietly running a different command.
fn split_assignment(word: &Word) -> Option<(String, Vec<Piece>)> {
    let Some(Piece::Text(first)) = word.pieces.first() else {
        return None;
    };
    let at = first.find('=')?;
    let name = &first[..at];
    if !is_name(name) {
        return None;
    }
    let mut value = Vec::new();
    let rest = &first[at + 1..];
    if !rest.is_empty() {
        value.push(Piece::Text(rest.to_string()));
    }
    value.extend(word.pieces[1..].iter().cloned());
    Some((name.to_string(), value))
}

/// Turning the line into tokens.
///
/// Quoting is resolved here rather than left for a later pass, because it is what decides whether
/// a character is syntax or text and every judgement after this depends on the answer.
struct Scan<'a> {
    line: &'a str,
    chars: Vec<(usize, char)>,
    at: usize,
}

impl<'a> Scan<'a> {
    fn new(line: &'a str) -> Self {
        Self {
            line,
            chars: line.char_indices().collect(),
            at: 0,
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).map(|(_, c)| *c)
    }

    fn ahead(&self, n: usize) -> Option<char> {
        self.chars.get(self.at + n).map(|(_, c)| *c)
    }

    /// The byte offset of the character about to be read, or the end of the line.
    fn offset(&self) -> usize {
        self.chars
            .get(self.at)
            .map_or(self.line.len(), |(index, _)| *index)
    }

    fn bump(&mut self) -> Option<char> {
        let next = self.peek();
        if next.is_some() {
            self.at += 1;
        }
        next
    }

    fn refuse(&self, span: Span, reason: Reason) -> Refused {
        refuse(self.line, span, reason)
    }

    fn tokens(mut self) -> Result<Vec<Tok>, Refused> {
        let mut out = Vec::new();
        loop {
            while matches!(self.peek(), Some(' ' | '\t')) {
                self.bump();
            }
            let start = self.offset();
            let Some(c) = self.peek() else { break };
            let single = Span::new(start, start + c.len_utf8());
            match c {
                // A line, not a script. A newline in a shell separates commands, and admitting it
                // would mean the thing being endorsed is no longer one line long.
                '\n' | '\r' => {
                    return Err(self.refuse(single, Reason::Syntax("a command line is one line")));
                }
                '|' => {
                    self.bump();
                    if self.peek() == Some('|') {
                        self.bump();
                        out.push(Tok::OrIf(Span::new(start, self.offset())));
                    } else {
                        out.push(Tok::Pipe(single));
                    }
                }
                '&' => {
                    self.bump();
                    match self.peek() {
                        Some('&') => {
                            self.bump();
                            out.push(Tok::AndIf(Span::new(start, self.offset())));
                        }
                        Some('>') => {
                            self.bump();
                            out.push(Tok::Redirect(Op::Both, Span::new(start, self.offset())));
                        }
                        _ => return Err(self.refuse(single, Reason::Background)),
                    }
                }
                ';' => {
                    self.bump();
                    out.push(Tok::Semi(single));
                }
                '(' => {
                    self.bump();
                    out.push(Tok::Open(single));
                }
                ')' => {
                    self.bump();
                    out.push(Tok::Close(single));
                }
                '<' => {
                    self.bump();
                    match self.peek() {
                        Some('<') => {
                            self.bump();
                            if self.peek() == Some('<') {
                                self.bump();
                            }
                            return Err(
                                self.refuse(Span::new(start, self.offset()), Reason::HereDocument)
                            );
                        }
                        Some('(') => {
                            self.bump();
                            return Err(self.refuse(
                                Span::new(start, self.offset()),
                                Reason::ProcessSubstitution,
                            ));
                        }
                        _ => out.push(Tok::Redirect(Op::In, single)),
                    }
                }
                '>' => {
                    self.bump();
                    match self.peek() {
                        Some('>') => {
                            self.bump();
                            out.push(Tok::Redirect(
                                Op::OutAppend,
                                Span::new(start, self.offset()),
                            ));
                        }
                        Some('(') => {
                            self.bump();
                            return Err(self.refuse(
                                Span::new(start, self.offset()),
                                Reason::ProcessSubstitution,
                            ));
                        }
                        Some('&') => {
                            self.bump();
                            return Err(
                                self.refuse(Span::new(start, self.offset()), Reason::Descriptor)
                            );
                        }
                        _ => out.push(Tok::Redirect(Op::Out, single)),
                    }
                }
                // A digit run touching a `<` or a `>` is a redirection's descriptor rather than
                // the start of a word, which is the one place a digit is syntax.
                c if c.is_ascii_digit() && self.is_io_number() => {
                    let token = self.io_redirect(start)?;
                    out.push(token);
                }
                _ => out.push(Tok::Word(self.word()?)),
            }
        }
        Ok(out)
    }

    /// Whether the digits at the cursor are a redirection's descriptor.
    fn is_io_number(&self) -> bool {
        let mut ahead = 0;
        while self.ahead(ahead).is_some_and(|c| c.is_ascii_digit()) {
            ahead += 1;
        }
        matches!(self.ahead(ahead), Some('<' | '>'))
    }

    /// A redirection written with its descriptor, of which `2>`, `2>>` and `2>&1` are the whole set.
    fn io_redirect(&mut self, start: usize) -> Result<Tok, Refused> {
        let mut digits = String::new();
        while let Some(c) = self.peek().filter(char::is_ascii_digit) {
            digits.push(c);
            self.bump();
        }
        if digits != "2" || self.peek() != Some('>') {
            // Consumed so the span covers the operator the caller was reading, not the digits
            // alone, which on their own would read as an ordinary argument.
            self.bump();
            return Err(self.refuse(Span::new(start, self.offset()), Reason::IoNumber));
        }
        self.bump();
        match self.peek() {
            Some('>') => {
                self.bump();
                Ok(Tok::Redirect(
                    Op::ErrAppend,
                    Span::new(start, self.offset()),
                ))
            }
            Some('&') => {
                self.bump();
                if self.peek() == Some('1') {
                    self.bump();
                    return Ok(Tok::Redirect(Op::ErrToOut, Span::new(start, self.offset())));
                }
                Err(self.refuse(Span::new(start, self.offset()), Reason::Descriptor))
            }
            _ => Ok(Tok::Redirect(Op::Err, Span::new(start, self.offset()))),
        }
    }

    /// One word, with quoting resolved and patterns kept apart from text.
    fn word(&mut self) -> Result<Word, Refused> {
        let start = self.offset();
        let pieces = self.pieces(false)?;
        Ok(Word {
            pieces,
            span: Span::new(start, self.offset()),
        })
    }

    /// The pieces of a word, stopping before whatever ends it.
    ///
    /// `in_brace` adds `,` and `}` to what ends it, which is how one alternative of a brace
    /// expression is read by the same code that reads a word.
    fn pieces(&mut self, in_brace: bool) -> Result<Vec<Piece>, Refused> {
        let mut pieces: Vec<Piece> = Vec::new();
        let mut text = String::new();
        let mut quoted = false;

        macro_rules! flush {
            () => {
                if !text.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut text)));
                }
            };
        }

        while let Some(c) = self.peek() {
            let start = self.offset();
            if ends_word(c) || (in_brace && matches!(c, ',' | '}')) {
                break;
            }
            match c {
                '\'' => {
                    quoted = true;
                    self.bump();
                    loop {
                        match self.bump() {
                            // A backslash is literal inside single quotes, so there is nothing to
                            // interpret and the run ends only at the next quote.
                            Some('\'') => break,
                            Some(c) => text.push(c),
                            None => {
                                return Err(self.refuse(
                                    Span::new(start, self.offset()),
                                    Reason::Unclosed('\''),
                                ));
                            }
                        }
                    }
                }
                '"' => {
                    quoted = true;
                    self.bump();
                    loop {
                        match self.peek() {
                            Some('"') => {
                                self.bump();
                                break;
                            }
                            // Expansion inside double quotes is still expansion, so the same
                            // refusal applies as outside them.
                            Some('$') => return Err(self.dollar()),
                            Some('`') => return Err(self.backtick()),
                            Some('\\') => {
                                self.bump();
                                match self.bump() {
                                    Some(c @ ('"' | '\\' | '$' | '`')) => text.push(c),
                                    // A backslash before anything else is a backslash, which is
                                    // what a shell does inside double quotes.
                                    Some(c) => {
                                        text.push('\\');
                                        text.push(c);
                                    }
                                    None => {
                                        return Err(self.refuse(
                                            Span::new(start, self.offset()),
                                            Reason::Unclosed('"'),
                                        ));
                                    }
                                }
                            }
                            Some(c) => {
                                text.push(c);
                                self.bump();
                            }
                            None => {
                                return Err(self.refuse(
                                    Span::new(start, self.offset()),
                                    Reason::Unclosed('"'),
                                ));
                            }
                        }
                    }
                }
                '\\' => {
                    self.bump();
                    match self.bump() {
                        Some(c) => text.push(c),
                        None => {
                            return Err(self.refuse(
                                Span::new(start, self.offset()),
                                Reason::Syntax("a `\\` with nothing after it"),
                            ));
                        }
                    }
                }
                '$' => return Err(self.dollar()),
                '`' => return Err(self.backtick()),
                '!' => {
                    self.bump();
                    return Err(
                        self.refuse(Span::new(start, self.offset()), Reason::HistoryExpansion)
                    );
                }
                '*' => {
                    flush!();
                    let mut stars = 0;
                    while self.peek() == Some('*') {
                        self.bump();
                        stars += 1;
                    }
                    pieces.push(if stars > 1 { Piece::Tree } else { Piece::Any });
                }
                '?' => {
                    flush!();
                    self.bump();
                    pieces.push(Piece::One);
                }
                '[' => match self.class() {
                    Some(class) => {
                        flush!();
                        pieces.push(class);
                    }
                    // A bracket that opens nothing is a bracket, which is what a shell makes of
                    // it. There is one reading, so this is not a guess.
                    None => {
                        text.push('[');
                        self.bump();
                    }
                },
                '{' => match self.braces()? {
                    Some(braces) => {
                        flush!();
                        pieces.push(braces);
                    }
                    None => {
                        text.push('{');
                        self.bump();
                    }
                },
                '~' if pieces.is_empty() && text.is_empty() => {
                    self.bump();
                    match self.peek() {
                        None | Some('/') => pieces.push(Piece::Home),
                        Some(c) if ends_word(c) => pieces.push(Piece::Home),
                        Some(c) => {
                            let end = self.offset() + c.len_utf8();
                            return Err(self.refuse(Span::new(start, end), Reason::NamedHome));
                        }
                    }
                }
                c => {
                    text.push(c);
                    self.bump();
                }
            }
        }

        flush!();
        // `''` is a real, empty argument, and it must not vanish: a command given one and a
        // command given none are different commands.
        if pieces.is_empty() && quoted {
            pieces.push(Piece::Text(String::new()));
        }
        Ok(pieces)
    }

    /// A refusal for the `$` at the cursor, naming what it opened.
    fn dollar(&mut self) -> Refused {
        let start = self.offset();
        let reason = match (self.ahead(1), self.ahead(2)) {
            (Some('('), Some('(')) => Reason::Arithmetic,
            (Some('('), _) => Reason::CommandSubstitution,
            _ => Reason::Variable,
        };
        self.bump();
        // The span covers the whole construct where it can be found, so the message points at
        // `${HOME}` rather than at a dollar sign the reader has to go looking for.
        match self.peek() {
            Some('(') => self.take_until(')'),
            Some('{') => self.take_until('}'),
            _ => {
                while self.peek().is_some_and(|c| c.is_alphanumeric() || c == '_') {
                    self.bump();
                }
            }
        }
        self.refuse(Span::new(start, self.offset()), reason)
    }

    /// A refusal for the backtick at the cursor.
    fn backtick(&mut self) -> Refused {
        let start = self.offset();
        self.bump();
        self.take_until('`');
        self.refuse(Span::new(start, self.offset()), Reason::CommandSubstitution)
    }

    /// Consume up to and including `close`, or to the end of the line.
    ///
    /// Only ever used to widen a span that is already a refusal, so an unbalanced construct
    /// running to the end of the line is the right answer rather than a further error.
    fn take_until(&mut self, close: char) {
        while let Some(c) = self.bump() {
            if c == close {
                break;
            }
        }
    }

    /// A `[…]` class, or nothing where the bracket opens no class.
    fn class(&mut self) -> Option<Piece> {
        let save = self.at;
        self.bump();
        let mut body = String::new();
        // A leading `!` or `^` negates, and a `]` straight after either is the character itself.
        if matches!(self.peek(), Some('!' | '^')) {
            body.push(self.bump()?);
        }
        if self.peek() == Some(']') {
            body.push(self.bump()?);
        }
        loop {
            match self.peek() {
                Some(']') => {
                    self.bump();
                    return Some(Piece::Class(body));
                }
                Some('\\') => {
                    self.bump();
                    match self.bump() {
                        Some(c) => body.push(c),
                        None => break,
                    }
                }
                // Nothing that would have ended the word can be inside the brackets, and neither
                // can anything that would have been refused: falling back to a literal bracket
                // leaves the refusal to the ordinary path.
                Some(c) if ends_word(c) || matches!(c, '$' | '`') => break,
                Some(c) => {
                    body.push(c);
                    self.bump();
                }
                None => break,
            }
        }
        self.at = save;
        None
    }

    /// A `{a,b}` or `{1..9}`, or nothing where the brace opens neither.
    ///
    /// A refusal from inside the braces stands rather than becoming a fallback, because the
    /// literal reading would reach the same construct and refuse it there.
    fn braces(&mut self) -> Result<Option<Piece>, Refused> {
        let save = self.at;
        self.bump();
        let mut alternatives = Vec::new();
        loop {
            alternatives.push(self.pieces(true)?);
            match self.peek() {
                Some(',') => {
                    self.bump();
                }
                Some('}') => {
                    self.bump();
                    break;
                }
                _ => {
                    self.at = save;
                    return Ok(None);
                }
            }
        }

        if alternatives.len() == 1 {
            let range = literal_of(&alternatives[0]).and_then(|body| parse_range(&body));
            if let Some(piece) = range {
                return Ok(Some(piece));
            }
            // `{}` and `{x}` stand for themselves in a shell, and a single alternative is not a
            // choice, so there is nothing to expand.
            self.at = save;
            return Ok(None);
        }
        Ok(Some(Piece::Alternatives(alternatives)))
    }
}

/// `1..9` as a range, where that is what it is.
fn parse_range(body: &str) -> Option<Piece> {
    let (from, to) = body.split_once("..")?;
    let low: i64 = from.parse().ok()?;
    let high: i64 = to.parse().ok()?;
    // Padded endpoints mean padded output, which is what a person writing `{01..12}` is asking
    // for. Anything unpadded is plain.
    let padded = |text: &str| text.trim_start_matches('-').starts_with('0') && text.len() > 1;
    let width = if padded(from) || padded(to) {
        from.len().max(to.len())
    } else {
        0
    };
    Some(Piece::Range {
        from: low,
        to: high,
        width,
    })
}

/// Turning tokens into a tree.
struct Parser<'a> {
    line: &'a str,
    tokens: Vec<Tok>,
    at: usize,
    /// How many commands have been read, so that a line cannot grow past what a plan may hold.
    commands: usize,
    /// How deep inside `( … )` the parser is, which is also how deep the tree can get.
    nesting: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at)
    }

    fn refuse(&self, span: Span, reason: Reason) -> Refused {
        refuse(self.line, span, reason)
    }

    /// The end of the line, for a span pointing at something that is not there.
    fn tail(&self) -> Span {
        Span::new(self.line.len(), self.line.len())
    }

    /// `and_or (';' and_or)*`, with a trailing `;` allowed.
    fn sequence(&mut self) -> Result<Node, Refused> {
        let mut left = self.and_or()?;
        while matches!(self.peek(), Some(Tok::Semi(_))) {
            self.at += 1;
            if matches!(self.peek(), None | Some(Tok::Close(_))) {
                break;
            }
            let right = self.and_or()?;
            left = Node::Join {
                left: Box::new(left),
                joiner: Joiner::Then,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// `element (('&&' | '||') element)*`, binding tighter than `;` and to the left.
    fn and_or(&mut self) -> Result<Node, Refused> {
        let mut left = self.element()?;
        loop {
            let joiner = match self.peek() {
                Some(Tok::AndIf(_)) => Joiner::And,
                Some(Tok::OrIf(_)) => Joiner::Or,
                _ => break,
            };
            self.at += 1;
            let right = self.element()?;
            left = Node::Join {
                left: Box::new(left),
                joiner,
                right: Box::new(right),
            };
        }
        Ok(left)
    }

    /// A group or a pipeline.
    ///
    /// A group holds a sequence and cannot itself be a pipeline stage, which is what "for
    /// sequencing only" means: there is no list whose output has to be piped somewhere, so
    /// nothing here needs a subshell to hold one.
    fn element(&mut self) -> Result<Node, Refused> {
        if let Some(Tok::Open(span)) = self.peek() {
            let span = *span;
            self.at += 1;
            self.nesting += 1;
            if self.nesting > MAX_NESTING {
                return Err(self.refuse(span, Reason::TooNested { cap: MAX_NESTING }));
            }
            let inner = self.sequence()?;
            self.nesting -= 1;
            if !matches!(self.peek(), Some(Tok::Close(_))) {
                return Err(self.refuse(span, Reason::Unclosed('(')));
            }
            self.at += 1;
            return Ok(Node::Group(Box::new(inner)));
        }
        self.pipeline()
    }

    /// `command ('|' command)*`.
    fn pipeline(&mut self) -> Result<Node, Refused> {
        let mut stages = vec![self.command()?];
        while matches!(self.peek(), Some(Tok::Pipe(_))) {
            self.at += 1;
            stages.push(self.command()?);
        }
        if stages.len() == 1 {
            return Ok(Node::Command(stages.swap_remove(0)));
        }
        Ok(Node::Pipeline(stages))
    }

    /// `assignment* (word | redirection)+`, with at least one word among them.
    fn command(&mut self) -> Result<Command, Refused> {
        let start = self.peek().map_or(self.line.len(), |tok| tok.span().start);
        self.commands += 1;
        if self.commands > MAX_STEPS {
            return Err(self.refuse(
                Span::new(start, start),
                Reason::TooManySteps { cap: MAX_STEPS },
            ));
        }
        let mut assignments = Vec::new();
        let mut words: Vec<Word> = Vec::new();
        let mut redirections = Vec::new();
        let mut end = start;

        loop {
            match self.peek() {
                Some(Tok::Word(word)) => {
                    let word = word.clone();
                    self.at += 1;
                    end = word.span.end;
                    // Only in front of the program. `env FOO=bar cmd` passes an argument to `env`
                    // and is not this.
                    if words.is_empty()
                        && let Some((name, value)) = split_assignment(&word)
                    {
                        let value = literal_of(&value)
                            .ok_or_else(|| self.refuse(word.span, Reason::AssignmentValue))?;
                        assignments.push(Assignment {
                            name,
                            value,
                            span: word.span,
                        });
                        continue;
                    }
                    words.push(word);
                }
                Some(Tok::Redirect(op, span)) => {
                    let (op, span) = (*op, *span);
                    self.at += 1;
                    end = span.end;
                    if op == Op::ErrToOut {
                        redirections.push(Redirection::StderrToStdout);
                        continue;
                    }
                    let Some(Tok::Word(target)) = self.peek() else {
                        return Err(
                            self.refuse(span, Reason::Syntax("a redirection with no target"))
                        );
                    };
                    let target = target.clone();
                    self.at += 1;
                    end = target.span.end;
                    redirections.push(match op {
                        Op::Out => Redirection::Stdout {
                            target,
                            append: false,
                        },
                        Op::OutAppend => Redirection::Stdout {
                            target,
                            append: true,
                        },
                        Op::In => Redirection::Stdin { target },
                        Op::Err => Redirection::Stderr {
                            target,
                            append: false,
                        },
                        Op::ErrAppend => Redirection::Stderr {
                            target,
                            append: true,
                        },
                        Op::Both | Op::ErrToOut => Redirection::Both { target },
                    });
                }
                _ => break,
            }
        }

        if words.is_empty() {
            let span = if end > start {
                Span::new(start, end)
            } else {
                self.tail()
            };
            let reason = if assignments.is_empty() {
                Reason::Syntax("a command with no program")
            } else {
                // Nothing carries over between calls, so an assignment on its own could only ever
                // have set something for a command that is not there.
                Reason::Syntax("an assignment with nothing to run")
            };
            return Err(self.refuse(span, reason));
        }

        let program = &words[0];
        if let Some(name) = program.literal() {
            if let Some(found) = INTERPRETERS.iter().find(|word| **word == name) {
                return Err(self.refuse(program.span, Reason::Interpreter(found)));
            }
            if let Some(found) = CONTROL_FLOW.iter().find(|word| **word == name) {
                return Err(self.refuse(program.span, Reason::ControlFlow(found)));
            }
        }

        Ok(Command {
            assignments,
            words,
            redirections,
            span: Span::new(start, end),
        })
    }
}

/// How many arguments one word may stand for.
///
/// A prompt long enough that nobody reads it is a prompt that grants everything and asks nothing,
/// so the bound is what a person can still take in rather than what a machine can produce.
pub const MAX_ARGUMENTS: usize = 100;

/// How many directories working out one pattern may read.
///
/// `**` over a large tree is a walk, and a walk with no ceiling holds the turn open while nothing
/// is shown. Reaching this is a refusal naming the ceiling, never a partial answer: a plan built
/// from half a walk is a plan that does not match the line.
pub const MAX_DIRECTORIES: usize = 4096;

/// What one word stands for.
///
/// Braces multiply, `~` becomes a path, and a pattern becomes the files it matches, sorted. A word
/// with no pattern in it stands for itself and touches no filesystem, so a target that does not
/// exist yet is still an argument.
///
/// `directory` is where the command will run, which is what a relative pattern is relative to.
pub fn expand(word: &Word, directory: &Path, home: Option<&Path>) -> Result<Vec<String>, Refused> {
    let refused = |reason| Refused {
        span: word.span,
        text: render(&word.pieces),
        reason,
    };

    // Counted before anything is built, so the bound limits the work rather than only the answer.
    let count = alternative_count(&word.pieces);
    if count > MAX_ARGUMENTS {
        return Err(refused(Reason::TooMany {
            found: count,
            cap: MAX_ARGUMENTS,
        }));
    }

    let mut out = Vec::new();
    for candidate in alternatives(&word.pieces) {
        let candidate = resolve_home(&candidate, home).ok_or_else(|| refused(Reason::NoHome))?;
        if !candidate.iter().any(is_pattern) {
            // Nothing to work out, so nothing is read. `> out.txt` names a file that does not
            // exist yet, and a pattern-free word must not be judged against what is on disk.
            out.push(render(&candidate));
            continue;
        }
        let matched = walk(directory, &candidate).map_err(refused)?;
        if matched.is_empty() {
            // Never the pattern itself. A shell passes an unmatched pattern through as an
            // argument, which is the one thing nobody ever means by writing it.
            return Err(refused(Reason::NoMatch));
        }
        out.extend(matched);
    }

    if out.len() > MAX_ARGUMENTS {
        return Err(refused(Reason::TooMany {
            found: out.len(),
            cap: MAX_ARGUMENTS,
        }));
    }
    Ok(out)
}

/// Whether a piece stands for something that has to be looked up on disk.
fn is_pattern(piece: &Piece) -> bool {
    matches!(
        piece,
        Piece::Any | Piece::One | Piece::Tree | Piece::Class(_)
    )
}

/// A word's pieces as the pattern they were written as.
///
/// The spelling a refusal quotes back, and the text a pattern-free word simply is.
fn render(pieces: &[Piece]) -> String {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Any => out.push('*'),
            Piece::One => out.push('?'),
            Piece::Tree => out.push_str("**"),
            Piece::Class(body) => {
                out.push('[');
                out.push_str(body);
                out.push(']');
            }
            Piece::Alternatives(options) => {
                out.push('{');
                for (index, option) in options.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(&render(option));
                }
                out.push('}');
            }
            Piece::Range { from, to, width } => {
                out.push('{');
                out.push_str(&pad(*from, *width));
                out.push_str("..");
                out.push_str(&pad(*to, *width));
                out.push('}');
            }
            Piece::Home => out.push('~'),
        }
    }
    out
}

/// `n` as text, zero-padded to `width` where a padded range asked for it.
fn pad(n: i64, width: usize) -> String {
    if width == 0 {
        return n.to_string();
    }
    let digits = n.unsigned_abs().to_string();
    let sign = if n < 0 { 1 } else { 0 };
    let zeros = width.saturating_sub(digits.len() + sign);
    format!(
        "{}{}{}",
        if n < 0 { "-" } else { "" },
        "0".repeat(zeros),
        digits
    )
}

/// How many words the braces in `pieces` multiply out to.
///
/// Saturating, because the answer to an absurd range is only ever used to refuse it.
fn alternative_count(pieces: &[Piece]) -> usize {
    let mut total = 1usize;
    for piece in pieces {
        let factor = match piece {
            Piece::Alternatives(options) => options
                .iter()
                .map(|option| alternative_count(option))
                .fold(0usize, usize::saturating_add),
            Piece::Range { from, to, .. } => usize::try_from(from.abs_diff(*to))
                .unwrap_or(usize::MAX)
                .saturating_add(1),
            _ => 1,
        };
        total = total.saturating_mul(factor);
    }
    total
}

/// The words the braces in `pieces` stand for, in the order a reader would write them.
fn alternatives(pieces: &[Piece]) -> Vec<Vec<Piece>> {
    let mut out: Vec<Vec<Piece>> = vec![Vec::new()];
    for piece in pieces {
        match piece {
            Piece::Alternatives(options) => {
                let mut next = Vec::new();
                for base in &out {
                    for option in options {
                        for tail in alternatives(option) {
                            let mut one = base.clone();
                            one.extend(tail);
                            next.push(one);
                        }
                    }
                }
                out = next;
            }
            Piece::Range { from, to, width } => {
                // A descending range counts down, which is what somebody writing `{9..1}` asked
                // for.
                let numbers: Vec<i64> = if from <= to {
                    (*from..=*to).collect()
                } else {
                    (*to..=*from).rev().collect()
                };
                let mut next = Vec::new();
                for base in &out {
                    for n in &numbers {
                        let mut one = base.clone();
                        one.push(Piece::Text(pad(*n, *width)));
                        next.push(one);
                    }
                }
                out = next;
            }
            other => {
                for one in &mut out {
                    one.push(other.clone());
                }
            }
        }
    }
    out
}

/// `pieces` with a leading `~` replaced by the home directory it stands for.
fn resolve_home(pieces: &[Piece], home: Option<&Path>) -> Option<Vec<Piece>> {
    if !pieces.first().is_some_and(|piece| *piece == Piece::Home) {
        return Some(pieces.to_vec());
    }
    let home = home?.to_string_lossy().into_owned();
    let mut out = vec![Piece::Text(home)];
    out.extend(pieces[1..].iter().cloned());
    Some(out)
}

/// One `/`-separated part of a pattern.
enum Part {
    /// Text, matching one name exactly and needing no directory read.
    Name(String),
    /// A pattern, matched against the names in one directory.
    Pattern(Vec<Piece>),
    /// `**`, standing for any number of directories.
    Tree,
}

/// Split a pattern into the parts a walk takes one at a time.
///
/// Empty parts are dropped, so `src/` and `a//b` mean what a reader means by them. Whether the
/// pattern was absolute is the caller's to notice, since a leading `/` is one of those empties.
fn parts(pieces: &[Piece]) -> Vec<Part> {
    let mut out: Vec<Vec<Piece>> = Vec::new();
    let mut current: Vec<Piece> = Vec::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => {
                for (index, segment) in text.split('/').enumerate() {
                    if index > 0 {
                        out.push(std::mem::take(&mut current));
                    }
                    if !segment.is_empty() {
                        current.push(Piece::Text(segment.to_string()));
                    }
                }
            }
            other => current.push(other.clone()),
        }
    }
    out.push(current);

    out.into_iter()
        .filter(|part| !part.is_empty())
        .map(|part| {
            if part.len() == 1 && part[0] == Piece::Tree {
                return Part::Tree;
            }
            match literal_of(&part) {
                Some(name) => Part::Name(name),
                None => Part::Pattern(part),
            }
        })
        .collect()
}

/// The paths `pieces` matches, sorted.
///
/// Iterative rather than recursive, so a `**` over a deep tree cannot exhaust the stack. Symlinks
/// are stepped over the way the tree walk steps over them, which also settles what a cycle does.
fn walk(directory: &Path, pieces: &[Piece]) -> Result<Vec<String>, Reason> {
    let absolute = matches!(pieces.first(), Some(Piece::Text(text)) if text.starts_with('/'));
    let parts = parts(pieces);
    let base: PathBuf = if absolute {
        PathBuf::from("/")
    } else {
        directory.to_path_buf()
    };

    let mut found: Vec<String> = Vec::new();
    let mut read = 0usize;
    let mut pending: Vec<(String, usize)> = vec![(String::new(), 0)];

    while let Some((relative, index)) = pending.pop() {
        if index == parts.len() {
            found.push(if absolute {
                format!("/{relative}")
            } else {
                relative
            });
            continue;
        }
        match &parts[index] {
            // A name is either there or it is not, so nothing is enumerated to find out.
            Part::Name(name) => {
                let next = extend(&relative, name);
                if base.join(&next).symlink_metadata().is_ok() {
                    pending.push((next, index + 1));
                }
            }
            Part::Tree => {
                read += 1;
                if read > MAX_DIRECTORIES {
                    return Err(Reason::TooDeep {
                        cap: MAX_DIRECTORIES,
                    });
                }
                // `**` stands for no directories as readily as for several, so the rest of the
                // pattern is tried here as well as below.
                pending.push((relative.clone(), index + 1));
                for (name, is_dir) in entries(&base.join(&relative)) {
                    if !is_dir
                        || name.starts_with('.')
                        || crate::workspace::is_ignored_directory(&name)
                    {
                        continue;
                    }
                    pending.push((extend(&relative, &name), index));
                }
            }
            Part::Pattern(part) => {
                read += 1;
                if read > MAX_DIRECTORIES {
                    return Err(Reason::TooDeep {
                        cap: MAX_DIRECTORIES,
                    });
                }
                let last = index + 1 == parts.len();
                for (name, is_dir) in entries(&base.join(&relative)) {
                    if !matches_name(part, &name) || (!last && !is_dir) {
                        continue;
                    }
                    pending.push((extend(&relative, &name), index + 1));
                }
            }
        }
    }

    // Sorted so that the same line compiles to the same plan every time. An endorsement is bound
    // to the plan, and a plan whose argument order came out of a directory read would not be.
    found.sort();
    found.dedup();
    Ok(found)
}

/// `relative` with `name` on the end.
fn extend(relative: &str, name: &str) -> String {
    if relative.is_empty() {
        name.to_string()
    } else {
        format!("{relative}/{name}")
    }
}

/// The names in `directory`, each with whether it is a directory.
///
/// A symlink is neither, and is left out: following one would reach outside the tree being
/// walked, and a link to an ancestor would make the walk endless.
fn entries(directory: &Path) -> Vec<(String, bool)> {
    let Ok(reading) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in reading.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        out.push((
            entry.file_name().to_string_lossy().into_owned(),
            kind.is_dir(),
        ));
    }
    out
}

/// Whether one directory entry's name matches one part of a pattern.
fn matches_name(part: &[Piece], name: &str) -> bool {
    // A name beginning with a dot is matched only by a pattern that writes the dot, which is the
    // rule a shell uses and the reason `*` does not sweep up `.git`.
    if name.starts_with('.')
        && !matches!(part.first(), Some(Piece::Text(text)) if text.starts_with('.'))
    {
        return false;
    }
    matches_atoms(&atoms(part), name)
}

/// One character's worth of a pattern.
enum Atom<'a> {
    Char(char),
    /// `?`
    One,
    /// `[…]`
    Class(&'a str),
    /// `*`, and `**` where one appears inside a name rather than as a whole part.
    Star,
}

fn atoms(part: &[Piece]) -> Vec<Atom<'_>> {
    let mut out = Vec::new();
    for piece in part {
        match piece {
            Piece::Text(text) => out.extend(text.chars().map(Atom::Char)),
            Piece::One => out.push(Atom::One),
            Piece::Any | Piece::Tree => out.push(Atom::Star),
            Piece::Class(body) => out.push(Atom::Class(body)),
            // Braces are gone before a pattern is walked, and a `~` is a path by then.
            Piece::Alternatives(_) | Piece::Range { .. } | Piece::Home => {}
        }
    }
    out
}

/// Whether `name` matches `atoms`.
///
/// One pass forward with a single remembered `*`, so the work is the name's length times the
/// pattern's and a hostile pattern costs nothing unusual. Patterns arrive through a turn, and a
/// backtracking matcher would turn one into a way to hold the turn open.
fn matches_atoms(atoms: &[Atom<'_>], name: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let mut at = 0;
    let mut into = 0;
    let mut star: Option<(usize, usize)> = None;

    while into < name.len() {
        let matched = match atoms.get(at) {
            Some(Atom::Star) => {
                star = Some((at + 1, into));
                at += 1;
                continue;
            }
            Some(Atom::Char(c)) => *c == name[into],
            Some(Atom::One) => true,
            Some(Atom::Class(body)) => class_matches(body, name[into]),
            None => false,
        };
        if matched {
            at += 1;
            into += 1;
            continue;
        }
        // Give the remembered `*` one more character and carry on from there.
        match star {
            Some((resume, from)) => {
                at = resume;
                into = from + 1;
                star = Some((resume, from + 1));
            }
            None => return false,
        }
    }

    while matches!(atoms.get(at), Some(Atom::Star)) {
        at += 1;
    }
    at == atoms.len()
}

/// Whether `c` is in the bracket expression `body`.
fn class_matches(body: &str, c: char) -> bool {
    let (negated, body) = match body.strip_prefix(['!', '^']) {
        Some(rest) => (true, rest),
        None => (false, body),
    };
    let chars: Vec<char> = body.chars().collect();
    let mut at = 0;
    let mut hit = false;
    while at < chars.len() {
        if at + 2 < chars.len() && chars[at + 1] == '-' {
            if chars[at] <= c && c <= chars[at + 2] {
                hit = true;
            }
            at += 3;
        } else {
            if chars[at] == c {
                hit = true;
            }
            at += 1;
        }
    }
    hit != negated
}

/// Compile `line` into the plan that would run it in `directory`.
///
/// Every branch is compiled, whether or not it would be reached, so that a person answering one
/// question has been shown everything the line could do. Anything the compiler cannot fully
/// resolve is a refusal, and a refusal yields no plan.
pub fn compile(line: &str, directory: &Path, home: Option<&Path>) -> Result<Plan, Refused> {
    let node = parse(line)?;
    let mut compiler = Compiler {
        directory,
        home,
        writes: Vec::new(),
        reads: Vec::new(),
    };
    let steps = compiler.node(&node)?;
    Ok(Plan {
        line: line.trim_end().to_string(),
        directory: directory.to_path_buf(),
        steps,
        writes: compiler.writes,
        reads: compiler.reads,
        stdin: None,
    })
}

/// One compile in progress: where it runs, and what the plan has named so far.
struct Compiler<'a> {
    directory: &'a Path,
    home: Option<&'a Path>,
    writes: Vec<PathBuf>,
    reads: Vec<PathBuf>,
}

impl Compiler<'_> {
    fn node(&mut self, node: &Node) -> Result<Steps, Refused> {
        match node {
            Node::Command(command) => Ok(Steps::Pipeline(vec![self.step(command)?])),
            Node::Pipeline(commands) => {
                let mut steps = Vec::with_capacity(commands.len());
                for command in commands {
                    steps.push(self.step(command)?);
                }
                Ok(Steps::Pipeline(steps))
            }
            Node::Join {
                left,
                joiner,
                right,
            } => {
                let left = self.node(left)?;
                let right = self.node(right)?;
                Ok(Steps::Join {
                    left: Box::new(left),
                    joiner: *joiner,
                    right: Box::new(right),
                })
            }
            Node::Group(inner) => Ok(Steps::Group(Box::new(self.node(inner)?))),
        }
    }

    fn step(&mut self, command: &Command) -> Result<Step, Refused> {
        let word = command.program();
        let expanded = expand(word, self.directory, self.home)?;
        let [program] = expanded.as_slice() else {
            return Err(Refused {
                span: word.span,
                text: render(&word.pieces),
                reason: Reason::NotOneProgram,
            });
        };

        // Before the name is looked up, so that the answer does not depend on whether the editor
        // in question happens to be installed on this machine.
        if let Some(alternative) = wants_a_terminal(program, command) {
            return Err(Refused {
                span: command.span,
                text: program.clone(),
                reason: Reason::Interactive(alternative),
            });
        }

        let resolved =
            crate::programs::resolve(program, self.directory).ok_or_else(|| Refused {
                span: word.span,
                text: program.clone(),
                reason: Reason::NotFound,
            })?;

        let mut args = Vec::new();
        for word in &command.words[1..] {
            args.extend(expand(word, self.directory, self.home)?);
        }

        let mut routes = Vec::new();
        for redirection in &command.redirections {
            routes.push(self.route(redirection)?);
        }

        Ok(Step {
            program: program.clone(),
            resolved,
            args,
            environment: command
                .assignments
                .iter()
                .map(|assignment| (assignment.name.clone(), assignment.value.clone()))
                .collect(),
            routes,
        })
    }

    /// One redirection, with its target recorded in the write set or the read set.
    fn route(&mut self, redirection: &Redirection) -> Result<Route, Refused> {
        Ok(match redirection {
            Redirection::Stdout { target, append } => Route::Stdout {
                path: self.writing(target)?,
                append: *append,
            },
            Redirection::Stderr { target, append } => Route::Stderr {
                path: self.writing(target)?,
                append: *append,
            },
            Redirection::Both { target } => Route::Both {
                path: self.writing(target)?,
            },
            Redirection::Stdin { target } => {
                let path = self.target(target)?;
                self.reads.push(path.clone());
                Route::Stdin { path }
            }
            // A descriptor renamed touches no file, so there is nothing to put in either set and
            // nothing for a person to endorse beyond the step itself.
            Redirection::StderrToStdout => Route::StderrToStdout,
        })
    }

    fn writing(&mut self, target: &Word) -> Result<PathBuf, Refused> {
        let path = self.target(target)?;
        self.writes.push(path.clone());
        Ok(path)
    }

    /// The one file a redirection names.
    ///
    /// A pattern is refused even where it happens to match a single file today. The plan has to
    /// say where the bytes go, and a destination worked out from what is on disk is a destination
    /// that changes when the tree does.
    fn target(&self, word: &Word) -> Result<PathBuf, Refused> {
        let refused = || Refused {
            span: word.span,
            text: render(&word.pieces),
            reason: Reason::NotOnePath,
        };
        if word.pieces.iter().any(is_pattern) {
            return Err(refused());
        }
        let expanded = expand(word, self.directory, self.home)?;
        let [one] = expanded.as_slice() else {
            return Err(refused());
        };
        let path = Path::new(one);
        Ok(if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.directory.join(path)
        })
    }
}

/// What to do instead, for an invocation that would want a terminal.
///
/// A convenience rather than a guarantee. Something interactive that is not named here reaches
/// the deadline and comes back with what it printed, which is the same outcome by a slower road,
/// so this list is allowed to be short and is not a safety property.
fn wants_a_terminal(program: &str, command: &Command) -> Option<&'static str> {
    let name = Path::new(program)
        .file_name()?
        .to_string_lossy()
        .into_owned();
    let args: Vec<String> = command.words[1..]
        .iter()
        .filter_map(|word| word.literal())
        .collect();
    match name.as_str() {
        "vi" | "vim" | "nvim" | "emacs" | "nano" | "pico" | "ed" | "micro" | "helix" | "hx"
        | "kak" => Some(
            "an editor needs a terminal. Use `edit_file`, or a program that takes what it needs on the command line",
        ),
        "less" | "more" | "most" => Some(
            "a pager needs a terminal. Narrow the output in the line instead, with `head`, `tail` or `sed -n`",
        ),
        "top" | "htop" | "btop" => {
            Some("this draws over a terminal. Ask for one reading instead, such as `ps`")
        }
        "git" => git_wants_a_terminal(&args),
        _ => None,
    }
}

fn git_wants_a_terminal(args: &[String]) -> Option<&'static str> {
    let subcommand = args.iter().find(|arg| !arg.starts_with('-'))?;
    let has = |flags: &[&str]| args.iter().any(|arg| flags.contains(&arg.as_str()));
    match subcommand.as_str() {
        "rebase" if has(&["-i", "--interactive"]) => Some(
            "`git rebase -i` opens an editor. Name the rewrite outright, with `--onto` or an explicit sequence",
        ),
        "add" if has(&["-i", "--interactive", "-p", "--patch"]) => {
            Some("`git add -i` and `git add -p` need a terminal. Name the paths to stage instead")
        }
        "commit" if !carries_a_message(args) => {
            Some("`git commit` with no message opens an editor. Pass `-m`")
        }
        _ => None,
    }
}

/// Whether a `git commit` was given its message rather than left to ask for one.
fn carries_a_message(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg == "--no-edit"
            || arg.starts_with("--message")
            || arg.starts_with("--file")
            || arg.starts_with("--reuse-message")
            // A bundled short flag carries its letters, so `-am` is a message as surely as `-m`.
            || (arg.starts_with('-')
                && !arg.starts_with("--")
                && arg.contains(['m', 'F', 'C']))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(line: &str) -> Node {
        parse(line).unwrap_or_else(|e| panic!("`{line}` should parse, and was refused: {e}"))
    }

    fn refused(line: &str) -> Refused {
        parse(line).expect_err(&format!("`{line}` should have been refused"))
    }

    /// The words of a single command, as the text they are.
    fn argv(line: &str) -> Vec<String> {
        match parsed(line) {
            Node::Command(command) => command
                .words
                .iter()
                .map(|word| word.literal().expect("a literal word"))
                .collect(),
            other => panic!("`{line}` is not one command: {other:?}"),
        }
    }

    fn command(line: &str) -> Command {
        match parsed(line) {
            Node::Command(command) => command,
            other => panic!("`{line}` is not one command: {other:?}"),
        }
    }

    /// A command is its program and its operands, in the order they were written, because that
    /// order is what a person reading the plan is endorsing.
    #[test]
    fn a_simple_command_is_its_program_and_its_operands() {
        assert_eq!(
            argv("git log --oneline -50"),
            ["git", "log", "--oneline", "-50"]
        );
    }

    /// Filtering at the source is the whole point of the notation, so a pipe has to survive as
    /// the ordered chain it is.
    #[test]
    fn a_pipeline_keeps_its_stages_in_order() {
        match parsed("grep -rn symbol components/ | head -30") {
            Node::Pipeline(stages) => {
                assert_eq!(stages.len(), 2);
                assert_eq!(stages[0].program().literal().as_deref(), Some("grep"));
                assert_eq!(stages[1].program().literal().as_deref(), Some("head"));
            }
            other => panic!("not a pipeline: {other:?}"),
        }
    }

    /// Precedence decides which branches can run, and every branch that can run has to be in the
    /// plan a person is shown. Reading `a && b ; c` as `a && (b ; c)` would show a plan in which
    /// `c` depends on `a`, which is not what was written.
    #[test]
    fn and_binds_tighter_than_a_semicolon() {
        match parsed("a && b ; c") {
            Node::Join {
                left,
                joiner: Joiner::Then,
                right,
            } => {
                assert!(matches!(
                    *left,
                    Node::Join {
                        joiner: Joiner::And,
                        ..
                    }
                ));
                assert!(matches!(*right, Node::Command(_)));
            }
            other => panic!("not sequenced at the top: {other:?}"),
        }
    }

    /// Grouping exists so a sequence can be one side of a branch. It starts no subshell, so a
    /// group is a shape in the plan and never a process of its own.
    #[test]
    fn parentheses_group_a_sequence() {
        match parsed("(a ; b) && c") {
            Node::Join {
                left,
                joiner: Joiner::And,
                ..
            } => assert!(matches!(*left, Node::Group(_))),
            other => panic!("not a group on the left: {other:?}"),
        }
    }

    /// The property the whole surface rests on: the compiler is the only thing that ever splits
    /// the line, and by the time an argument exists the splitting has already happened.
    #[test]
    fn a_quoted_metacharacter_is_one_argument() {
        assert_eq!(
            argv("git commit -m '; rm -rf /'"),
            ["git", "commit", "-m", "; rm -rf /"]
        );
    }

    #[test]
    fn a_backslash_makes_the_next_character_ordinary() {
        assert_eq!(argv(r"echo a\;b"), ["echo", "a;b"]);
    }

    /// A command given an empty argument and a command given none are different commands, so an
    /// empty argument must not vanish between the line and the plan.
    #[test]
    fn an_empty_argument_survives_being_quoted() {
        let command = command("prog ''");
        assert_eq!(command.words.len(), 2);
        assert_eq!(command.words[1].pieces, vec![Piece::Text(String::new())]);
    }

    /// Each form names a different stream and a different mode, and a redirection read as the
    /// wrong one writes a file nobody endorsed.
    #[test]
    fn each_redirection_form_is_read_as_the_stream_it_names() {
        let command = command("cmd > a >> b < c 2> d 2>> e &> f");
        let target = |word: &Word| word.literal().expect("a literal target");
        let forms: Vec<String> = command
            .redirections
            .iter()
            .map(|redirection| match redirection {
                Redirection::Stdout { target: t, append } => {
                    format!("out{}{}", if *append { "+" } else { "" }, target(t))
                }
                Redirection::Stdin { target: t } => format!("in{}", target(t)),
                Redirection::Stderr { target: t, append } => {
                    format!("err{}{}", if *append { "+" } else { "" }, target(t))
                }
                Redirection::Both { target: t } => format!("both{}", target(t)),
                Redirection::StderrToStdout => "join".to_string(),
            })
            .collect();
        assert_eq!(forms, ["outa", "out+b", "inc", "errd", "err+e", "bothf"]);
    }

    /// Joining the streams renames a descriptor. Treating it as a write would put a file called
    /// `1` in the plan's write set and ask a person to endorse it.
    #[test]
    fn stderr_joining_stdout_names_no_file() {
        let command = command("cmd 2>&1");
        assert_eq!(command.redirections, vec![Redirection::StderrToStdout]);
    }

    /// Quoting is what separates a pattern from the characters it is made of, and the distinction
    /// has to survive parsing: after this there is nothing left that could tell them apart.
    #[test]
    fn a_glob_survives_as_a_pattern_and_a_quoted_one_does_not() {
        let command = command("ls *.rs '*.rs' a?b src/**/main.rs");
        assert_eq!(
            command.words[1].pieces,
            vec![Piece::Any, Piece::Text(".rs".into())]
        );
        assert_eq!(command.words[2].pieces, vec![Piece::Text("*.rs".into())]);
        assert_eq!(
            command.words[3].pieces,
            vec![Piece::Text("a".into()), Piece::One, Piece::Text("b".into())]
        );
        assert_eq!(
            command.words[4].pieces,
            vec![
                Piece::Text("src/".into()),
                Piece::Tree,
                Piece::Text("/main.rs".into())
            ]
        );
    }

    #[test]
    fn braces_are_alternatives_or_a_range() {
        let command = command("ls {a,b}.rs {1..3}");
        assert_eq!(
            command.words[1].pieces,
            vec![
                Piece::Alternatives(vec![
                    vec![Piece::Text("a".into())],
                    vec![Piece::Text("b".into())]
                ]),
                Piece::Text(".rs".into())
            ]
        );
        assert_eq!(
            command.words[2].pieces,
            vec![Piece::Range {
                from: 1,
                to: 3,
                width: 0
            }]
        );
    }

    /// `find -exec ls {} \;` is an ordinary request, and `{}` there is two characters rather than
    /// a choice between nothing and nothing. A brace that stands for one thing stands for itself,
    /// which is the only reading it has.
    #[test]
    fn a_brace_that_offers_no_choice_is_literal_text() {
        assert_eq!(
            argv(r"find . -exec ls {} \;"),
            ["find", ".", "-exec", "ls", "{}", ";"]
        );
        assert_eq!(argv("echo {x}"), ["echo", "{x}"]);
    }

    #[test]
    fn a_leading_tilde_stands_for_a_home_and_a_later_one_does_not() {
        let command = command("ls ~/src a~b");
        assert_eq!(
            command.words[1].pieces,
            vec![Piece::Home, Piece::Text("/src".into())]
        );
        assert_eq!(command.words[2].pieces, vec![Piece::Text("a~b".into())]);
    }

    /// A `~` that would stand for somebody else's home is refused rather than left as text: a
    /// plan showing a literal `~alice` reads as a path that was going to be expanded.
    #[test]
    fn a_tilde_naming_a_user_is_refused() {
        assert_eq!(refused("ls ~alice/src").reason, Reason::NamedHome);
    }

    #[test]
    fn an_assignment_in_front_of_the_program_is_environment() {
        let command = command("RUST_LOG=debug cargo test");
        assert_eq!(command.assignments.len(), 1);
        assert_eq!(command.assignments[0].name, "RUST_LOG");
        assert_eq!(command.assignments[0].value, "debug");
        assert_eq!(
            command
                .words
                .iter()
                .map(|word| word.literal().unwrap())
                .collect::<Vec<_>>(),
            ["cargo", "test"]
        );
    }

    /// `env FOO=bar printenv` passes an argument to `env`. Reading it as an assignment would run
    /// a different program from the one that was written.
    #[test]
    fn an_assignment_after_the_program_is_an_ordinary_argument() {
        let command = command("env FOO=bar printenv");
        assert!(command.assignments.is_empty());
        assert_eq!(argv("env FOO=bar printenv"), ["env", "FOO=bar", "printenv"]);
    }

    /// An assignment whose value expands would put something in a program's environment that the
    /// endorsed plan did not show.
    #[test]
    fn an_assignment_whose_value_is_not_literal_is_refused() {
        assert_eq!(refused("FOO=*.rs cmd").reason, Reason::AssignmentValue);
    }

    /// Nothing carries over between calls, so an assignment on its own could only have set
    /// something for a command that is not in the line.
    #[test]
    fn an_assignment_with_nothing_to_run_is_refused() {
        assert!(matches!(refused("FOO=bar").reason, Reason::Syntax(_)));
    }

    /// The refusals, one per row of the grammar's closed table. Each is a construct whose meaning
    /// is not in the line, so a person shown the line has not been shown the plan.
    #[test]
    fn a_command_whose_text_would_be_computed_is_refused() {
        assert_eq!(refused("echo $(date)").reason, Reason::CommandSubstitution);
        assert_eq!(refused("echo `date`").reason, Reason::CommandSubstitution);
    }

    #[test]
    fn process_substitution_is_refused() {
        assert_eq!(
            refused("diff <(a) <(b)").reason,
            Reason::ProcessSubstitution
        );
        assert_eq!(refused("tee >(cat)").reason, Reason::ProcessSubstitution);
    }

    #[test]
    fn a_variable_is_refused() {
        assert_eq!(refused("ls $HOME").reason, Reason::Variable);
        assert_eq!(refused("ls ${HOME}").reason, Reason::Variable);
    }

    #[test]
    fn arithmetic_is_refused() {
        assert_eq!(refused("echo $((1+1))").reason, Reason::Arithmetic);
    }

    /// Backgrounding is a parameter of the call rather than a token in the line, so that it is
    /// visible in the prompt and cannot hide inside an argument.
    #[test]
    fn backgrounding_is_refused() {
        assert_eq!(refused("sleep 5 &").reason, Reason::Background);
    }

    #[test]
    fn a_here_document_is_refused() {
        assert_eq!(refused("cat << EOF").reason, Reason::HereDocument);
        assert_eq!(refused("cat <<< text").reason, Reason::HereDocument);
    }

    #[test]
    fn a_program_that_reintroduces_interpretation_is_refused() {
        for line in [
            "eval ls",
            "source setup.sh",
            ". setup.sh",
            "exec ls",
            "trap x",
        ] {
            assert!(
                matches!(refused(line).reason, Reason::Interpreter(_)),
                "`{line}` was not refused as an interpreter"
            );
        }
    }

    #[test]
    fn a_word_that_opens_control_flow_is_refused() {
        for line in ["if true", "while true", "for x", "case x", "function f"] {
            assert!(
                matches!(refused(line).reason, Reason::ControlFlow(_)),
                "`{line}` was not refused as control flow"
            );
        }
    }

    #[test]
    fn an_unquoted_exclamation_mark_is_refused() {
        assert_eq!(refused("echo !!").reason, Reason::HistoryExpansion);
    }

    #[test]
    fn a_descriptor_duplication_other_than_stderr_to_stdout_is_refused() {
        assert_eq!(refused("cmd >&2").reason, Reason::Descriptor);
        assert_eq!(refused("cmd 2>&3").reason, Reason::Descriptor);
    }

    #[test]
    fn a_numbered_redirection_other_than_standard_error_is_refused() {
        assert_eq!(refused("cmd 1> out").reason, Reason::IoNumber);
        assert_eq!(refused("cmd 2< in").reason, Reason::IoNumber);
    }

    #[test]
    fn a_redirection_with_no_target_is_refused() {
        assert!(matches!(refused("cmd >").reason, Reason::Syntax(_)));
        assert!(matches!(refused("cmd > | wc").reason, Reason::Syntax(_)));
    }

    #[test]
    fn an_unclosed_quote_is_refused() {
        assert_eq!(refused("echo 'a").reason, Reason::Unclosed('\''));
        assert_eq!(refused("echo \"a").reason, Reason::Unclosed('"'));
        assert_eq!(refused("(a").reason, Reason::Unclosed('('));
    }

    /// A line, not a script. A newline separates commands in a shell, and admitting one would
    /// mean the thing being endorsed is no longer one line long.
    #[test]
    fn a_newline_in_the_line_is_refused() {
        assert!(matches!(
            refused("echo a\necho b").reason,
            Reason::Syntax(_)
        ));
    }

    /// Expansion inside double quotes is still expansion, so the quoting that makes a `$`
    /// ordinary is the single-quoted kind.
    #[test]
    fn an_expansion_inside_double_quotes_is_still_refused() {
        assert_eq!(refused(r#"echo "$HOME""#).reason, Reason::Variable);
        assert_eq!(
            refused(r#"echo "`date`""#).reason,
            Reason::CommandSubstitution
        );
    }

    /// None of the refusals is about text that looks dangerous. Quoted, every one of them is an
    /// ordinary argument, and the planner can pass any of these characters to a program.
    #[test]
    fn quoting_makes_a_refused_construct_ordinary_text() {
        assert_eq!(
            argv(r#"echo '$HOME' '`date`' '!' '&' '<<' '$(x)'"#),
            ["echo", "$HOME", "`date`", "!", "&", "<<", "$(x)"]
        );
        assert_eq!(argv(r#"echo "\$HOME""#), ["echo", "$HOME"]);
    }

    /// A refusal has to say where, or the planner cannot tell which part of a long line it has to
    /// rewrite.
    #[test]
    fn a_refusal_names_the_span_that_caused_it() {
        let refusal = refused("ls $HOME");
        assert_eq!(refusal.text, "$HOME");
        assert_eq!(refusal.span, Span { start: 3, end: 8 });
        assert!(refusal.to_string().contains("$HOME"));
    }

    /// There is no degraded mode: no falling back to a shell, and no running the prefix that did
    /// compile. A refusal produces no tree at all, so there is nothing for a caller to run.
    #[test]
    fn a_line_that_half_compiles_yields_nothing() {
        assert!(parse("echo ok && $(date)").is_err());
        assert!(parse("echo ok ; ls $HOME").is_err());
    }

    #[test]
    fn an_empty_line_is_refused() {
        assert!(matches!(refused("   ").reason, Reason::Syntax(_)));
    }

    /// A plan a reader cannot take in is a prompt that grants everything, and the same bound keeps
    /// the tree shallow enough that compiling or running a line cannot recurse away.
    #[test]
    fn a_line_with_more_commands_than_a_plan_may_hold_is_refused() {
        let long = vec!["true"; MAX_STEPS + 1].join(" ; ");
        assert_eq!(
            refused(&long).reason,
            Reason::TooManySteps { cap: MAX_STEPS }
        );
        let allowed = vec!["true"; MAX_STEPS].join(" ; ");
        assert!(parse(&allowed).is_ok());
    }

    #[test]
    fn groups_nested_past_the_bound_are_refused() {
        let deep = format!(
            "{}true{}",
            "(".repeat(MAX_NESTING + 1),
            ")".repeat(MAX_NESTING + 1)
        );
        assert_eq!(
            refused(&deep).reason,
            Reason::TooNested { cap: MAX_NESTING }
        );
    }

    /// A scratch tree to expand patterns against.
    struct Tree {
        root: PathBuf,
    }

    impl Tree {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir().join(format!("bravebot-cmdline-{name}"));
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).expect("a scratch directory");
            Self { root }
        }

        fn file(&self, relative: &str) -> &Self {
            let path = self.root.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a scratch directory");
            }
            std::fs::write(&path, b"x").expect("a scratch file");
            self
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn nth_word(line: &str, index: usize) -> Word {
        command(line).words.swap_remove(index)
    }

    fn expanded(line: &str, index: usize, at: &Path) -> Vec<String> {
        expand(&nth_word(line, index), at, None)
            .unwrap_or_else(|e| panic!("`{line}` should expand, and was refused: {e}"))
    }

    fn expansion_refused(line: &str, index: usize, at: &Path) -> Refused {
        expand(&nth_word(line, index), at, None).expect_err("should have been refused")
    }

    /// The person is shown the file list rather than the pattern, so the pattern has to be gone
    /// by the time there is a plan. Sorted, because an endorsement is bound to the plan and a
    /// plan whose order came out of a directory read would not be stable.
    #[test]
    fn a_pattern_becomes_the_files_it_matches() {
        let tree = Tree::new("matches");
        tree.file("b.rs").file("a.rs").file("c.txt");
        assert_eq!(expanded("ls *.rs", 1, &tree.root), ["a.rs", "b.rs"]);
    }

    #[test]
    fn a_pattern_matches_within_one_segment_and_a_tree_across_them() {
        let tree = Tree::new("segments");
        tree.file("a.rs").file("sub/d.rs").file("sub/deep/e.rs");
        assert_eq!(expanded("ls sub/*.rs", 1, &tree.root), ["sub/d.rs"]);
        assert_eq!(
            expanded("ls **/*.rs", 1, &tree.root),
            ["a.rs", "sub/d.rs", "sub/deep/e.rs"]
        );
    }

    /// A pattern standing for nothing is not an argument. A shell hands the pattern through as
    /// text, which is never what anybody writing one meant, and would put a plan in front of a
    /// person that reads as a list of files and is not one.
    #[test]
    fn a_pattern_matching_nothing_is_refused_rather_than_passed_through() {
        let tree = Tree::new("nothing");
        tree.file("a.rs");
        let refusal = expansion_refused("ls *.zzz", 1, &tree.root);
        assert_eq!(refusal.reason, Reason::NoMatch);
        assert_eq!(refusal.text, "*.zzz");
    }

    /// An approval prompt long enough that nobody reads it is a prompt that grants everything and
    /// asks nothing. The count is in the refusal so the planner knows how much narrower to be.
    #[test]
    fn expansion_is_bounded_and_the_refusal_says_the_count() {
        let tree = Tree::new("bounded");
        let refusal = expansion_refused("echo {1..500}", 1, &tree.root);
        assert_eq!(
            refusal.reason,
            Reason::TooMany {
                found: 500,
                cap: MAX_ARGUMENTS
            }
        );
        assert!(refusal.to_string().contains("500"));
    }

    /// The same names a listing steps over, so that one idea of what the tree contains serves
    /// both. Otherwise `**` would sweep a vendored dependency into a plan a person then endorses.
    #[test]
    fn a_tree_pattern_does_not_descend_into_an_ignored_directory() {
        let tree = Tree::new("ignored");
        tree.file("keep/x.rs")
            .file("node_modules/y.rs")
            .file(".git/z.rs")
            .file("target/w.rs");
        assert_eq!(expanded("ls **/*.rs", 1, &tree.root), ["keep/x.rs"]);
    }

    /// A word with no pattern in it is not judged against what is on disk. `> out.txt` names a
    /// file that does not exist yet, and refusing it would make redirection useless.
    #[test]
    fn a_word_with_no_pattern_names_a_file_that_need_not_exist() {
        let tree = Tree::new("absent");
        assert_eq!(expanded("echo nope.txt", 1, &tree.root), ["nope.txt"]);
    }

    #[test]
    fn braces_multiply_a_word() {
        let tree = Tree::new("braces");
        assert_eq!(
            expanded("echo {a,b}{1,2}", 1, &tree.root),
            ["a1", "a2", "b1", "b2"]
        );
    }

    #[test]
    fn a_range_counts_and_keeps_the_padding_it_was_written_with() {
        let tree = Tree::new("range");
        assert_eq!(expanded("echo {1..3}", 1, &tree.root), ["1", "2", "3"]);
        assert_eq!(expanded("echo {08..10}", 1, &tree.root), ["08", "09", "10"]);
        assert_eq!(expanded("echo {3..1}", 1, &tree.root), ["3", "2", "1"]);
    }

    /// The rule a shell uses, and the reason `*` does not sweep up `.git` even where the walk
    /// would have descended into it.
    #[test]
    fn a_dot_file_is_matched_only_by_a_pattern_that_writes_the_dot() {
        let tree = Tree::new("dotfiles");
        tree.file(".hidden").file("visible");
        assert_eq!(expanded("ls *", 1, &tree.root), ["visible"]);
        assert_eq!(expanded("ls .*", 1, &tree.root), [".hidden"]);
    }

    #[test]
    fn a_class_matches_the_characters_it_names() {
        let tree = Tree::new("class");
        tree.file("a1.txt").file("a2.txt").file("ab.txt");
        assert_eq!(
            expanded("ls a[0-9].txt", 1, &tree.root),
            ["a1.txt", "a2.txt"]
        );
        assert_eq!(expanded("ls a[!0-9].txt", 1, &tree.root), ["ab.txt"]);
    }

    /// A `~` stands for a path, so what reaches the plan is the path and not the character. The
    /// home directory is passed in rather than read here, so the answer does not depend on whose
    /// machine the test runs on.
    #[test]
    fn a_leading_tilde_becomes_the_home_directory() {
        let word = nth_word("ls ~/src", 1);
        let expanded = expand(&word, Path::new("/"), Some(Path::new("/home/someone")))
            .expect("a tilde with a home to stand for");
        assert_eq!(expanded, ["/home/someone/src"]);
    }

    /// Without a home there is no path to put in the plan, and inventing one would show a person
    /// a destination that is not where the bytes would go.
    #[test]
    fn a_tilde_with_no_home_to_stand_for_is_refused() {
        let word = nth_word("ls ~/src", 1);
        assert_eq!(
            expand(&word, Path::new("/"), None)
                .expect_err("no home")
                .reason,
            Reason::NoHome
        );
    }

    /// Quoting decides this as it decides everything else here: a quoted pattern is the
    /// characters it is made of, and nothing is read to work out what it stands for.
    #[test]
    fn a_quoted_pattern_is_not_expanded() {
        let tree = Tree::new("quoted");
        tree.file("a.rs");
        assert_eq!(expanded("ls '*.rs'", 1, &tree.root), ["*.rs"]);
    }

    fn compiled(line: &str, at: &Path) -> Plan {
        compile(line, at, None)
            .unwrap_or_else(|e| panic!("`{line}` should compile, and was refused: {e}"))
    }

    fn compile_refused(line: &str, at: &Path) -> Refused {
        compile(line, at, None).expect_err("should have been refused")
    }

    /// The plan is the routing field a raw string did not have, so a step has to carry the file
    /// that will run rather than the name that was written. Resolving once, here, is what keeps
    /// what a person endorsed and what executes the same value.
    #[test]
    fn a_step_carries_the_file_its_name_resolved_to() {
        let tree = Tree::new("resolved");
        let plan = compiled("cat", &tree.root);
        let steps = plan.steps.steps();
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].program, "cat");
        assert!(steps[0].resolved.is_absolute());
        assert!(steps[0].resolved.ends_with("cat"));
    }

    /// A branch that may run is an effect a person is answering for, so all of them are compiled
    /// and none is left to be worked out part-way through a line.
    #[test]
    fn every_branch_that_could_run_is_in_the_plan() {
        let tree = Tree::new("branches");
        let plan = compiled("cat a && head b || wc c", &tree.root);
        let programs: Vec<&str> = plan
            .steps
            .steps()
            .iter()
            .map(|step| step.program.as_str())
            .collect();
        assert_eq!(programs, ["cat", "head", "wc"]);
    }

    #[test]
    fn a_group_and_a_pipeline_both_keep_their_steps() {
        let tree = Tree::new("shapes");
        let plan = compiled("(cat a ; head b) && wc c | cat", &tree.root);
        assert_eq!(plan.steps.steps().len(), 4);
        assert!(matches!(plan.steps, Steps::Join { .. }));
    }

    /// By the time an argument is in a plan the only thing that ever split the line has already
    /// run, so there is nothing left that could split it again.
    #[test]
    fn an_argument_is_final_by_the_time_it_is_a_step() {
        let tree = Tree::new("final");
        let plan = compiled("cat '; rm -rf /'", &tree.root);
        assert_eq!(plan.steps.steps()[0].args, ["; rm -rf /"]);
        assert!(plan.writes.is_empty());
    }

    /// A redirection is where bytes land, so it belongs in the set of files a person is being
    /// asked to let this line write.
    #[test]
    fn a_redirection_joins_the_write_set() {
        let tree = Tree::new("writes");
        let plan = compiled("cat > out.txt", &tree.root);
        assert_eq!(plan.writes, [tree.root.join("out.txt")]);
        assert!(plan.reads.is_empty());
    }

    #[test]
    fn an_append_writes_and_an_input_reads() {
        let tree = Tree::new("streams");
        tree.file("in.txt");
        let plan = compiled("cat < in.txt >> out.txt 2> err.txt", &tree.root);
        assert_eq!(
            plan.writes,
            [tree.root.join("out.txt"), tree.root.join("err.txt")]
        );
        assert_eq!(plan.reads, [tree.root.join("in.txt")]);
    }

    /// Renaming a descriptor touches no file. Recording it as a write would put a file called `1`
    /// in front of a person and ask them to endorse it.
    #[test]
    fn joining_the_streams_writes_no_file() {
        let tree = Tree::new("join");
        let plan = compiled("cat 2>&1", &tree.root);
        assert!(plan.writes.is_empty());
        assert_eq!(plan.steps.steps()[0].routes, [Route::StderrToStdout]);
    }

    /// A destination worked out from what is on disk is a destination that moves when the tree
    /// does, so the plan would stop saying where the bytes go.
    #[test]
    fn a_redirection_target_that_is_not_one_path_is_refused() {
        let tree = Tree::new("target");
        tree.file("only.txt");
        assert_eq!(
            compile_refused("cat > *.txt", &tree.root).reason,
            Reason::NotOnePath
        );
        assert_eq!(
            compile_refused("cat > {a,b}.txt", &tree.root).reason,
            Reason::NotOnePath
        );
    }

    #[test]
    fn a_program_that_cannot_be_found_is_refused() {
        let tree = Tree::new("missing");
        assert_eq!(
            compile_refused("bravebot-no-such-program-anywhere x", &tree.root).reason,
            Reason::NotFound
        );
    }

    /// A program that would sit waiting for a terminal holds the turn open until the deadline and
    /// prints nothing useful. Refusing it up front costs a message and saves the wait.
    #[test]
    fn a_program_that_wants_a_terminal_is_refused_before_it_starts() {
        let tree = Tree::new("interactive");
        for line in [
            "vim notes.txt",
            "less notes.txt",
            "git rebase -i HEAD~3",
            "git add -p",
            "git commit",
        ] {
            assert!(
                matches!(
                    compile_refused(line, &tree.root).reason,
                    Reason::Interactive(_)
                ),
                "`{line}` was not refused as interactive"
            );
        }
    }

    /// The list is a convenience, not a ban on the program: the same program doing something that
    /// needs no terminal is an ordinary request.
    #[test]
    fn the_same_program_without_the_interactive_part_is_not_refused() {
        let tree = Tree::new("noninteractive");
        for line in ["git rebase --continue", "git add .", "git commit -m done"] {
            let refusal = compile(line, &tree.root, None);
            let interactive = matches!(
                refusal.as_ref().err().map(|e| &e.reason),
                Some(Reason::Interactive(_))
            );
            assert!(!interactive, "`{line}` was refused as interactive");
        }
    }

    /// An environment written in front of a step belongs to that step, so a plan shows what each
    /// program will see rather than something accumulated across the line.
    #[test]
    fn an_assignment_belongs_to_the_step_it_was_written_in_front_of() {
        let tree = Tree::new("environment");
        let plan = compiled("cat a | RUST_LOG=debug head -1", &tree.root);
        let steps = plan.steps.steps();
        assert!(steps[0].environment.is_empty());
        assert_eq!(
            steps[1].environment,
            [("RUST_LOG".to_string(), "debug".to_string())]
        );
    }

    /// The line is kept for a reader, and it is context rather than the thing endorsed: two
    /// spellings that compile alike are the same plan.
    #[test]
    fn the_line_is_kept_beside_the_plan_it_compiled_to() {
        let tree = Tree::new("spelling");
        let one = compiled("cat  'a b'", &tree.root);
        let other = compiled("cat \"a b\"", &tree.root);
        assert_eq!(one.steps, other.steps);
        assert_ne!(one.line, other.line);
    }
}
