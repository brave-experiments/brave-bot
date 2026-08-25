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

/// The document inside a fence that wraps the whole of `text`, or `None` if there is no such
/// fence.
pub fn strip(text: &str) -> Option<String> {
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
        let stripped = strip("```python\nprint(1)\nprint(2)\n```").expect("a wrapped document");
        assert_eq!(stripped, "print(1)\nprint(2)\n");
    }

    #[test]
    fn a_fence_with_no_language_is_still_a_fence() {
        assert_eq!(strip("```\nplain\n```"), Some("plain\n".to_string()));
    }

    /// Whitespace around the answer is the model's, not the document's.
    #[test]
    fn surrounding_blank_lines_do_not_hide_the_fence() {
        assert_eq!(
            strip("\n\n```js\nlet a = 1;\n```\n\n"),
            Some("let a = 1;\n".to_string())
        );
    }

    /// A markdown file is a document with fences in it. Taking the ends off one would corrupt
    /// the file this exists to protect.
    #[test]
    fn a_document_containing_fences_is_left_alone() {
        let markdown = "# Title\n\n```rust\nfn main() {}\n```\n\nMore prose.\n";
        assert_eq!(strip(markdown), None);
    }

    /// Two blocks and nothing else is still not one block.
    #[test]
    fn two_fenced_blocks_are_not_one_wrapper() {
        assert_eq!(strip("```\none\n```\n```\ntwo\n```"), None);
    }

    #[test]
    fn an_ordinary_file_is_left_alone() {
        assert_eq!(strip("fn main() {}\n"), None);
        assert_eq!(strip(""), None);
        assert_eq!(strip("```not closed\nbody\n"), None);
    }
}
