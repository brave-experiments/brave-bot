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
    /// Wait for one more key, which the instruction needs before it means anything.
    Wait(Pending),
    /// The key means nothing in this mode, and nothing at all should happen.
    ///
    /// Not "fall through to the ordinary bindings": a letter that vi does not use is a letter that
    /// does nothing, and typing it into the line would be the box acting on an instruction it did
    /// not understand.
    Nothing,
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
        }
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
}
