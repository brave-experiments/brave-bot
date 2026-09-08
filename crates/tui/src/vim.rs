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
    /// The key means nothing in this mode, and nothing at all should happen.
    ///
    /// Not "fall through to the ordinary bindings": a letter that vi does not use is a letter that
    /// does nothing, and typing it into the line would be the box acting on an instruction it did
    /// not understand.
    Nothing,
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
}
