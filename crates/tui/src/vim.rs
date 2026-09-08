//! Editing the line the way vi does, for somebody who has the habit.
//!
//! Two modes over the same box. INSERT is what the box has always been: a character typed lands at
//! the caret. NORMAL takes the letters as instructions instead, so `i` opens INSERT before the
//! caret and `A` opens it at the end of the line.
//!
//! Nothing labelled passes through here. This decides where a caret goes and which of the person's
//! own characters move, on a line they are typing; no workspace content and no model output reaches
//! it.
//!
//! # Why the mode is not a flag on the box
//!
//! Shell mode is a `bool` because there are two states and one of them is the absence of the other.
//! Here there is a third thing to say: whether the person asked for vi editing at all. Somebody who
//! did not is in neither mode, and their `i` is the letter i. So the state is an `Option`, absent
//! for the box everybody else has, and the two modes exist only inside it.

/// Which mode the box is in, for somebody editing the way vi does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Typing text, which is what the box does for everybody.
    #[default]
    Insert,
    /// Taking the letters as instructions.
    Normal,
}

impl Mode {
    /// The word this mode is drawn as.
    ///
    /// Untranslated and upper case, which is what every vi draws in the corner: it is the word
    /// somebody already knows, and `NORMAL` is not a sentence to be read but a state to be
    /// recognised.
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Insert => "INSERT",
            Mode::Normal => "NORMAL",
        }
    }
}

/// Which style of editing the box does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Editing {
    /// The box everybody has: arrows and the readline chords, and no modes at all.
    #[default]
    Ordinary,
    /// The box for somebody who edits the way vi does.
    Vi,
}

impl Editing {
    /// Every style, the ordinary box first, which is the order a picker offers them in.
    pub const ALL: [Editing; 2] = [Editing::Ordinary, Editing::Vi];

    /// The word this style is stored and configured as.
    ///
    /// `vim` rather than `vi`, because that is the word the settings files of the tools people
    /// already configure use, and a word somebody copies from one of those has to work here.
    pub fn as_str(self) -> &'static str {
        match self {
            Editing::Ordinary => "emacs",
            Editing::Vi => "vim",
        }
    }

    /// The style a word names, or `None` for a word that names none of them.
    ///
    /// Case-insensitive, because this reads a word from a file somebody may have edited by hand.
    /// An unrecognised word is no choice at all rather than a choice of something, so a mistyped
    /// setting leaves the box everybody has rather than a box whose keys do something unexplained.
    pub fn named(word: &str) -> Option<Editing> {
        let word = word.trim();
        Editing::ALL
            .into_iter()
            .find(|style| style.as_str().eq_ignore_ascii_case(word))
    }
}

/// What a key press in NORMAL mode asks the box to do.
///
/// Returned rather than applied, because the line lives on the session and this module holds no
/// reference to it. One place decides, and one place mutates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Open INSERT mode, having first moved the caret where the key says.
    Insert(Opening),
    /// Move the caret, and nothing else.
    Move(Motion),
    /// Change the line, over the stretch of it the second half names.
    Change(Operator, Extent),
    /// Wait for one more key, which the instruction needs before it means anything.
    Wait(Pending),
    /// Put back what the last change took, which is `u`.
    Undo,
    /// Do the last change again, at the caret, which is `.`.
    Again,
    /// Put the register into the line, before or after the caret: `P` and `p`.
    Paste { before: bool },
    /// Join this line and the one below into one, which is `J`.
    Join,
    /// The key means nothing in this mode, and nothing at all should happen.
    ///
    /// Not "fall through to the ordinary bindings": a letter that vi does not use is a letter that
    /// does nothing, and typing it into the line would be the box acting on an instruction it did
    /// not understand.
    Nothing,
}

/// What is done to a stretch of the line.
///
/// Three operators over one set of extents, which is what makes `dw`, `cw` and `yw` one idea rather
/// than three bindings: the letter says what happens and the rest says where.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    /// `d`: take it out, keeping it in the register.
    Delete,
    /// `c`: take it out and open INSERT mode where it was.
    Change,
    /// `y`: keep it in the register and leave the line alone.
    Yank,
    /// `>`: move the line a step further from the margin.
    Indent,
    /// `<`: move the line a step back towards the margin.
    Dedent,
}

impl Operator {
    /// Whether the line is left as it was.
    ///
    /// The one operator that reads without writing, which is why undo has nothing to record for it.
    pub fn reads_only(self) -> bool {
        matches!(self, Operator::Yank)
    }
}

/// Which stretch of the line an operator acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Extent {
    /// From the caret to wherever a motion would take it: `dw`, `d$`, `df,`.
    To(Motion),
    /// The whole line, newline and all: `dd`, `cc`, `yy`.
    Line,
    /// From the caret to the end of the line: `D`, `C`, `Y`.
    ToLineEnd,
    /// The character under the caret: `x`, and `s` with a change.
    Character,
    /// A thing the line is made of rather than a distance: `diw`, `da"`, `ci(`.
    Object(Object),
}

/// A stretch named by what it is rather than by how far away its end is.
///
/// The reason these exist: `ci(` is what somebody means when they want the arguments replaced, and
/// the alternative is counting characters to a closing bracket they can see perfectly well.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Object {
    /// What kind of thing.
    pub kind: Kind,
    /// Whether to take what surrounds it too: the blank after a word, or the brackets themselves.
    /// `a` against `i`.
    pub around: bool,
}

/// Which kind of thing a text object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `w`: a run of word characters, or a run of blanks where the caret is on one.
    Word,
    /// `W`: a run of anything that is not a blank, so a path or a flag is one thing.
    Bigword,
    /// A pair of delimiters and what lies between them, named by either of the pair.
    Pair(char, char),
}

impl Kind {
    /// The kind a character names as a text object, or `None` for one that names none.
    ///
    /// Either half of a pair names it, since `di(` and `di)` are the same request and nobody wants to
    /// think about which one they typed.
    pub fn named(c: char) -> Option<Kind> {
        match c {
            'w' => Some(Kind::Word),
            'W' => Some(Kind::Bigword),
            '"' => Some(Kind::Pair('"', '"')),
            '\'' => Some(Kind::Pair('\'', '\'')),
            '`' => Some(Kind::Pair('`', '`')),
            '(' | ')' => Some(Kind::Pair('(', ')')),
            '[' | ']' => Some(Kind::Pair('[', ']')),
            '{' | '}' => Some(Kind::Pair('{', '}')),
            '<' | '>' => Some(Kind::Pair('<', '>')),
            _ => None,
        }
    }
}

/// Where a motion takes the caret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    /// `h`: one character left.
    Left,
    /// `l` and Space: one character right.
    Right,
    /// `w`: the start of the next word.
    WordRight,
    /// `e`: the end of this word, or of the next one.
    WordEnd,
    /// `b`: the start of this word, or of the one before.
    WordLeft,
    /// `0`: the first column of the line.
    LineStart,
    /// `$`: the last character of the line.
    LineEnd,
    /// `^`: the first character of the line that is not a blank.
    FirstNonBlank,
    /// `gg`: the first line of the input.
    InputStart,
    /// `G`: the last line of the input.
    InputEnd,
    /// `f`, `F`, `t`, `T` once their character has arrived, and `;` and `,` repeating one.
    ToChar(Find),
}

impl Motion {
    /// Whether an operator over this motion takes the character it landed on.
    ///
    /// Vi's distinction, and it is not decoration: `dw` from the start of a word takes the word and the
    /// blank after it, stopping before the next word's first letter, while `de` takes the word and
    /// stops having taken its last. Both are what the keys mean, and the difference is exactly this.
    ///
    /// The forward-looking motions that land *on* something are inclusive. The ones that land where the
    /// next thing begins are not, since that character is the start of what was not asked for.
    pub fn takes_what_it_lands_on(self) -> bool {
        match self {
            Motion::WordEnd | Motion::LineEnd => true,
            // `f` lands on the character and takes it; `t` stops one short and takes that one.
            Motion::ToChar(find) => find.forwards,
            Motion::Left
            | Motion::Right
            | Motion::WordRight
            | Motion::WordLeft
            | Motion::LineStart
            | Motion::FirstNonBlank
            | Motion::InputStart
            | Motion::InputEnd => false,
        }
    }
}

/// A jump to a character on the line, which is the shape `f`, `F`, `t` and `T` share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Find {
    /// The character to look for.
    pub target: char,
    /// Whether to look towards the end of the line rather than the start.
    pub forwards: bool,
    /// Whether to stop short of the character rather than landing on it: `t` and `T` against `f`
    /// and `F`.
    pub short: bool,
}

impl Find {
    /// The same jump the other way, which is what `,` asks for.
    pub fn reversed(self) -> Self {
        Self {
            forwards: !self.forwards,
            ..self
        }
    }
}

/// An instruction that has arrived without everything it needs.
///
/// Held rather than acted on, because `f` alone says to jump to a character nobody has named yet.
/// The next key press names it, and until then nothing has happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    /// `f`, `F`, `t` or `T`, waiting for the character to jump to.
    Find { forwards: bool, short: bool },
    /// `g`, which means nothing alone and `gg` with the second press.
    G,
    /// An operator waiting for the stretch to act on: the motion in `dw`, or the doubled letter in
    /// `dd`.
    Operate(Operator),
    /// An operator waiting for the character in `df,` or `ct)`, having already taken the `f` or `t`.
    OperateToChar {
        operator: Operator,
        forwards: bool,
        short: bool,
    },
    /// An operator waiting for the kind of thing in `diw` or `ca"`, having already taken the `i` or
    /// `a`.
    OperateObject { operator: Operator, around: bool },
}

impl Pending {
    /// What the key that arrived after this one means, or `Nothing` where the pair is not an
    /// instruction.
    ///
    /// A pair that means nothing abandons the wait rather than holding it open for a third key. The
    /// alternative is a box where one stray press swallows every letter after it until something
    /// happens to match.
    pub fn then(self, c: char) -> Command {
        match self {
            Pending::Find { forwards, short } => Command::Move(Motion::ToChar(Find {
                target: c,
                forwards,
                short,
            })),
            Pending::G if c == 'g' => Command::Move(Motion::InputStart),
            Pending::G => Command::Nothing,
            Pending::OperateToChar {
                operator,
                forwards,
                short,
            } => Command::Change(
                operator,
                Extent::To(Motion::ToChar(Find {
                    target: c,
                    forwards,
                    short,
                })),
            ),
            Pending::OperateObject { operator, around } => match Kind::named(c) {
                Some(kind) => Command::Change(operator, Extent::Object(Object { kind, around })),
                None => Command::Nothing,
            },
            Pending::Operate(operator) => operated(operator, c),
        }
    }
}

/// What an operator does to the stretch the next key names.
///
/// The doubled letter is the whole line, which is why `dd` and `cc` are spelled that way and why the
/// letter has to be compared against the operator that is waiting: `dy` is not a line.
///
/// The jump keys wait again rather than resolving here, since `df` still needs the character. That is
/// the only place two keys stack up before anything happens.
fn operated(operator: Operator, c: char) -> Command {
    let doubled = match operator {
        Operator::Delete => 'd',
        Operator::Change => 'c',
        Operator::Yank => 'y',
        Operator::Indent => '>',
        Operator::Dedent => '<',
    };
    if c == doubled {
        return Command::Change(operator, Extent::Line);
    }
    match c {
        // `i` and `a` here are not the keys that open INSERT mode: after an operator they say the
        // stretch is a thing rather than a distance, and the next press says which thing.
        'i' => Command::Wait(Pending::OperateObject {
            operator,
            around: false,
        }),
        'a' => Command::Wait(Pending::OperateObject {
            operator,
            around: true,
        }),
        'f' => Command::Wait(Pending::OperateToChar {
            operator,
            forwards: true,
            short: false,
        }),
        'F' => Command::Wait(Pending::OperateToChar {
            operator,
            forwards: false,
            short: false,
        }),
        't' => Command::Wait(Pending::OperateToChar {
            operator,
            forwards: true,
            short: true,
        }),
        'T' => Command::Wait(Pending::OperateToChar {
            operator,
            forwards: false,
            short: true,
        }),
        // Any motion at all names a stretch, so `d$` and `dG` work for the reason `dw` does rather
        // than because they were listed. A key that is not a motion is not a stretch, and the pair
        // means nothing.
        _ => match command(c) {
            Command::Move(motion) => Command::Change(operator, Extent::To(motion)),
            _ => Command::Nothing,
        },
    }
}

/// Where the caret goes as INSERT mode opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    /// `i`: at the caret, before the character it is on.
    Here,
    /// `I`: at the first character of the line.
    LineStart,
    /// `a`: after the character the caret is on.
    After,
    /// `A`: at the end of the line.
    LineEnd,
    /// `o`: on a new line below this one.
    LineBelow,
    /// `O`: on a new line above this one.
    LineAbove,
}

/// What a key press in NORMAL mode means.
///
/// `None` for a key this module does not claim, which leaves it to whatever answered it before vi
/// editing existed: the arrows still move the caret, Enter still sends, and Ctrl-C still stops.
/// Those are not vi's keys, and taking them would make the mode a place where the rest of the
/// interface stops working.
pub fn command(c: char) -> Command {
    match c {
        'i' => Command::Insert(Opening::Here),
        'I' => Command::Insert(Opening::LineStart),
        'a' => Command::Insert(Opening::After),
        'A' => Command::Insert(Opening::LineEnd),
        'o' => Command::Insert(Opening::LineBelow),
        'O' => Command::Insert(Opening::LineAbove),
        'h' => Command::Move(Motion::Left),
        // Space among them, which is what vi does with it: a key wider than any other, for the
        // motion people press most.
        'l' | ' ' => Command::Move(Motion::Right),
        'w' => Command::Move(Motion::WordRight),
        'e' => Command::Move(Motion::WordEnd),
        'b' => Command::Move(Motion::WordLeft),
        '0' => Command::Move(Motion::LineStart),
        '$' => Command::Move(Motion::LineEnd),
        '^' => Command::Move(Motion::FirstNonBlank),
        'G' => Command::Move(Motion::InputEnd),
        'g' => Command::Wait(Pending::G),
        'f' => Command::Wait(Pending::Find {
            forwards: true,
            short: false,
        }),
        'F' => Command::Wait(Pending::Find {
            forwards: false,
            short: false,
        }),
        't' => Command::Wait(Pending::Find {
            forwards: true,
            short: true,
        }),
        'T' => Command::Wait(Pending::Find {
            forwards: false,
            short: true,
        }),
        // The operators, each waiting for the stretch to act on.
        'd' => Command::Wait(Pending::Operate(Operator::Delete)),
        'c' => Command::Wait(Pending::Operate(Operator::Change)),
        'y' => Command::Wait(Pending::Operate(Operator::Yank)),
        '>' => Command::Wait(Pending::Operate(Operator::Indent)),
        '<' => Command::Wait(Pending::Operate(Operator::Dedent)),
        // The capitals are the same operators to the end of the line, which is the one stretch common
        // enough to have a key of its own.
        'D' => Command::Change(Operator::Delete, Extent::ToLineEnd),
        'C' => Command::Change(Operator::Change, Extent::ToLineEnd),
        // `Y` is the line rather than the rest of it, which is vi's own inconsistency and the one
        // people's hands expect: `yy` and `Y` are the same key twice.
        'Y' => Command::Change(Operator::Yank, Extent::Line),
        // The character under the caret. `x` takes it and stays, `s` takes it and starts typing.
        'x' => Command::Change(Operator::Delete, Extent::Character),
        's' => Command::Change(Operator::Change, Extent::Character),
        'S' => Command::Change(Operator::Change, Extent::Line),
        'p' => Command::Paste { before: false },
        'P' => Command::Paste { before: true },
        'J' => Command::Join,
        'u' => Command::Undo,
        '.' => Command::Again,
        _ => Command::Nothing,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The word in a settings file is the one the tools people already configure use, so a line
    /// copied from one of those works here rather than being read as a name this program does not
    /// know.
    #[test]
    fn the_configured_word_for_vi_editing_is_the_one_other_tools_use() {
        assert_eq!(Editing::named("vim"), Some(Editing::Vi));
        assert_eq!(Editing::named("emacs"), Some(Editing::Ordinary));
    }

    /// A file edited by hand is where this word comes from, and `Vim` is the same request as `vim`.
    #[test]
    fn the_configured_word_is_read_whatever_its_case() {
        assert_eq!(Editing::named("VIM"), Some(Editing::Vi));
        assert_eq!(Editing::named("  Vim  "), Some(Editing::Vi));
    }

    /// A mistyped setting must leave the box everybody has. Reading an unknown word as vi editing
    /// would give somebody a box whose letters do things they did not ask for, and the only clue
    /// would be the typo they cannot see.
    #[test]
    fn a_word_naming_no_style_is_no_choice_at_all() {
        assert_eq!(Editing::named("vi"), None);
        assert_eq!(Editing::named(""), None);
        assert_eq!(Editing::named("nano"), None);
    }

    /// The six keys that open INSERT mode, each with the place the caret goes. They differ only in
    /// that, which is why one enum covers them.
    #[test]
    fn the_keys_that_open_insert_mode_say_where_the_caret_lands() {
        assert_eq!(command('i'), Command::Insert(Opening::Here));
        assert_eq!(command('I'), Command::Insert(Opening::LineStart));
        assert_eq!(command('a'), Command::Insert(Opening::After));
        assert_eq!(command('A'), Command::Insert(Opening::LineEnd));
        assert_eq!(command('o'), Command::Insert(Opening::LineBelow));
        assert_eq!(command('O'), Command::Insert(Opening::LineAbove));
    }

    /// A letter vi does not use does nothing, rather than being typed. Falling through to the line
    /// would make NORMAL mode a place where half the alphabet quietly edits the prompt.
    #[test]
    fn a_letter_that_means_nothing_in_normal_mode_types_nothing() {
        assert_eq!(command('z'), Command::Nothing);
        assert_eq!(command('q'), Command::Nothing);
    }

    /// Space is a motion rather than a character, which is what vi does with it: the widest key on the
    /// board, for the motion pressed most.
    #[test]
    fn space_moves_right_like_the_letter_does() {
        assert_eq!(command(' '), Command::Move(Motion::Right));
        assert_eq!(command('l'), Command::Move(Motion::Right));
    }

    /// The four jumps differ in two ways and nothing else: which direction they look, and whether they
    /// land on the character or stop short of it. One shape covers all four.
    #[test]
    fn the_four_jumps_to_a_character_differ_only_in_direction_and_where_they_stop() {
        let waiting = |c: char| match command(c) {
            Command::Wait(Pending::Find { forwards, short }) => (forwards, short),
            other => panic!("{c} was {other:?} rather than a jump waiting for its character"),
        };
        assert_eq!(waiting('f'), (true, false));
        assert_eq!(waiting('F'), (false, false));
        assert_eq!(waiting('t'), (true, true));
        assert_eq!(waiting('T'), (false, true));
    }

    /// The character that arrives after one of those keys is the target, whatever it is: `f$` jumps to
    /// a dollar rather than being read as the key that ends a line.
    #[test]
    fn the_press_after_a_jump_key_is_the_character_to_jump_to() {
        let pending = Pending::Find {
            forwards: true,
            short: false,
        };
        assert_eq!(
            pending.then('$'),
            Command::Move(Motion::ToChar(Find {
                target: '$',
                forwards: true,
                short: false,
            }))
        );
    }

    /// `,` is the last jump the other way, which is the whole of what it means. Reversing the direction
    /// and nothing else is what makes `f` then `,` land back where the caret came from.
    #[test]
    fn reversing_a_jump_changes_its_direction_and_nothing_else() {
        let find = Find {
            target: 'x',
            forwards: true,
            short: true,
        };
        assert_eq!(
            find.reversed(),
            Find {
                target: 'x',
                forwards: false,
                short: true,
            }
        );
    }

    /// `g` means nothing alone and `gg` is the first line. A pair that means nothing abandons the wait
    /// rather than holding it open for a third key, which would let one stray press swallow every
    /// letter after it until something happened to match.
    #[test]
    fn a_pair_beginning_with_g_is_the_start_of_the_input_or_nothing() {
        assert_eq!(command('g'), Command::Wait(Pending::G));
        assert_eq!(Pending::G.then('g'), Command::Move(Motion::InputStart));
        assert_eq!(Pending::G.then('x'), Command::Nothing);
    }

    /// Any motion at all names a stretch, which is what makes `dw`, `d$` and `dG` one idea rather than
    /// three bindings. A key that is not a motion names no stretch, and the pair means nothing.
    #[test]
    fn an_operator_takes_any_motion_as_its_stretch() {
        let after = |c: char| Pending::Operate(Operator::Delete).then(c);
        assert_eq!(
            after('w'),
            Command::Change(Operator::Delete, Extent::To(Motion::WordRight))
        );
        assert_eq!(
            after('$'),
            Command::Change(Operator::Delete, Extent::To(Motion::LineEnd))
        );
        assert_eq!(after('z'), Command::Nothing);
    }

    /// The doubled letter is the whole line, and it is compared against the operator that is waiting:
    /// `dy` is not a line, and reading any second letter as one would make every mistyped pair take a
    /// line out.
    #[test]
    fn the_doubled_letter_is_the_whole_line_and_only_its_own() {
        assert_eq!(
            Pending::Operate(Operator::Delete).then('d'),
            Command::Change(Operator::Delete, Extent::Line)
        );
        assert_eq!(
            Pending::Operate(Operator::Yank).then('y'),
            Command::Change(Operator::Yank, Extent::Line)
        );
        assert_eq!(
            Pending::Operate(Operator::Delete).then('y'),
            Command::Nothing
        );
    }

    /// `df,` is two keys before anything can happen, which is the only place a wait stacks on a wait:
    /// the operator has its motion and the motion still needs its character.
    #[test]
    fn an_operator_over_a_jump_waits_again_for_the_character() {
        let pending = match Pending::Operate(Operator::Delete).then('f') {
            Command::Wait(pending) => pending,
            other => panic!("df was {other:?} rather than a wait"),
        };
        assert_eq!(
            pending.then(','),
            Command::Change(
                Operator::Delete,
                Extent::To(Motion::ToChar(Find {
                    target: ',',
                    forwards: true,
                    short: false,
                }))
            )
        );
    }

    /// `de` takes the word's last letter and `dw` stops before the next word's first, which is the whole
    /// of the difference between an inclusive motion and an exclusive one.
    #[test]
    fn a_motion_says_whether_an_operator_takes_the_character_it_landed_on() {
        assert!(Motion::WordEnd.takes_what_it_lands_on());
        assert!(Motion::LineEnd.takes_what_it_lands_on());
        assert!(!Motion::WordRight.takes_what_it_lands_on());
        assert!(!Motion::WordLeft.takes_what_it_lands_on());
    }

    /// `i` and `a` after an operator are not the keys that open INSERT mode: they say the stretch is a
    /// thing rather than a distance, and the next press says which thing.
    #[test]
    fn i_and_a_after_an_operator_name_a_text_object() {
        let pending = |c: char| match Pending::Operate(Operator::Delete).then(c) {
            Command::Wait(pending) => pending,
            other => panic!("d{c} was {other:?} rather than a wait"),
        };
        assert_eq!(
            pending('i').then('w'),
            Command::Change(
                Operator::Delete,
                Extent::Object(Object {
                    kind: Kind::Word,
                    around: false
                })
            )
        );
        assert_eq!(
            pending('a').then('w'),
            Command::Change(
                Operator::Delete,
                Extent::Object(Object {
                    kind: Kind::Word,
                    around: true
                })
            )
        );
    }

    /// Either half of a pair names it, since `di(` and `di)` are the same request and nobody wants to
    /// have to think about which one they typed.
    #[test]
    fn either_half_of_a_pair_names_the_same_object() {
        assert_eq!(Kind::named('('), Some(Kind::Pair('(', ')')));
        assert_eq!(Kind::named(')'), Some(Kind::Pair('(', ')')));
        assert_eq!(Kind::named('{'), Kind::named('}'));
        assert_eq!(Kind::named('['), Kind::named(']'));
    }

    /// A quote is its own closing mark, which is why it is one kind with the same character twice
    /// rather than a case of its own.
    #[test]
    fn a_quote_closes_itself() {
        assert_eq!(Kind::named('"'), Some(Kind::Pair('"', '"')));
        assert_eq!(Kind::named('\''), Some(Kind::Pair('\'', '\'')));
    }

    /// A key naming no kind of thing ends the wait rather than holding it open for a third press.
    #[test]
    fn a_key_naming_no_kind_of_object_means_nothing() {
        assert_eq!(Kind::named('z'), None);
        assert_eq!(
            Pending::OperateObject {
                operator: Operator::Delete,
                around: false
            }
            .then('z'),
            Command::Nothing
        );
    }

    /// A yank reads without writing, which is why there is nothing for undo to put back after one and
    /// why it is the operator that records no change.
    #[test]
    fn the_yank_is_the_operator_that_only_reads() {
        assert!(Operator::Yank.reads_only());
        assert!(!Operator::Delete.reads_only());
        assert!(!Operator::Change.reads_only());
        assert!(!Operator::Indent.reads_only());
    }
}
