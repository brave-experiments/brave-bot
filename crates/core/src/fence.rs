//! Taking a document out of the markdown fence a model wrapped it in.
//!
//! A model asked to return a file hands back ```` ```rust ```` and the file and ```` ``` ````,
//! because that is what returning code looks like in a chat. When the answer is going into a file
//! rather than onto a screen, the fence is not part of the document: it is packaging, and it
//! makes the file it lands in invalid.
//!
//! Only a fence around the whole answer is packaging. A document with fenced blocks inside it,
//! which is what a markdown file is, must come back untouched, so the test is deliberately
//! strict: the first line opens a fence, the last line closes one, and nothing between them
//! closes it early.

/// `text` without the fence that wraps the whole of it, and `text` itself where no fence wraps
/// it.
///
/// Total on purpose. Whether an answer arrived fenced is a fact about untrusted bytes, and a
/// caller handed an `Option` would have to ask: the document goes into the same file either way,
/// so there is nothing there for anyone to decide and nothing here says which it was.
pub fn unwrapped(text: String) -> String {
    inside(&text).unwrap_or(text)
}

/// The document inside a fence that wraps the whole of `text`, or `None` if there is no such
/// fence.
///
/// Private, so the only thing that can be done with the absence of a fence is to leave the text
/// alone.
fn inside(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let mut lines = trimmed.lines();

    let opening = lines.next()?.trim_end();
    let marker = opening.chars().take_while(|c| *c == '`').count();
    // Three or more, as markdown requires. The rest of the opening line is the language, which
    // may be anything but must not contain a backtick.
    if marker < 3 || opening[marker..].contains('`') {
        return None;
    }

    let mut body: Vec<&str> = lines.collect();
    let closing = body.pop()?.trim();
    if closing.chars().take_while(|c| *c == '`').count() < marker
        || !closing.trim_matches('`').is_empty()
    {
        return None;
    }

    // A fence that closes in the middle means the answer is a document containing blocks rather
    // than one block, and taking the ends off it would corrupt it.
    if body
        .iter()
        .any(|line| line.trim().starts_with(&"`".repeat(marker)))
    {
        return None;
    }

    Some(format!("{}\n", body.join("\n")))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The case that put ```` ```python ```` at the top of somebody's server.py.
    #[test]
    fn a_fence_around_the_whole_answer_is_packaging() {
        assert_eq!(
            unwrapped("```python\nprint(1)\nprint(2)\n```".to_string()),
            "print(1)\nprint(2)\n"
        );
    }

    #[test]
    fn a_fence_with_no_language_is_still_a_fence() {
        assert_eq!(unwrapped("```\nplain\n```".to_string()), "plain\n");
    }

    /// Whitespace around the answer is the model's, not the document's.
    #[test]
    fn surrounding_blank_lines_do_not_hide_the_fence() {
        assert_eq!(
            unwrapped("\n\n```js\nlet a = 1;\n```\n\n".to_string()),
            "let a = 1;\n"
        );
    }

    /// A markdown file is a document with fences in it. Taking the ends off one would corrupt
    /// the file this exists to protect.
    #[test]
    fn a_document_containing_fences_is_left_alone() {
        let markdown = "# Title\n\n```rust\nfn main() {}\n```\n\nMore prose.\n";
        assert_eq!(unwrapped(markdown.to_string()), markdown);
    }

    /// Two blocks and nothing else is still not one block.
    #[test]
    fn two_fenced_blocks_are_not_one_wrapper() {
        let two = "```\none\n```\n```\ntwo\n```";
        assert_eq!(unwrapped(two.to_string()), two);
    }

    #[test]
    fn an_ordinary_file_is_left_alone() {
        assert_eq!(unwrapped("fn main() {}\n".to_string()), "fn main() {}\n");
        assert_eq!(unwrapped("".to_string()), "");
        assert_eq!(
            unwrapped("```not closed\nbody\n".to_string()),
            "```not closed\nbody\n"
        );
    }
}
