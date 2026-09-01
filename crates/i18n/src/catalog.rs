// The catalog format, and the parser for it.
//
// The build script includes this file directly, so it must stay dependency-free and must not
// reach for anything the rest of the crate defines.
//
// A subset of Project Fluent, chosen because translators already have tooling for `.ftl` and
// because the subset is small enough to parse by hand. The parts left out are the parts that
// need a real expression grammar: nested selects, term references, functions, and attributes.
// Nothing in this interface has ever wanted one, and a grammar is a parser, and a parser that
// runs on every start is a liability in a process whose whole premise is that it does not
// interpret what it was given.
//
// Parsing happens at build time only. What ships is generated Rust, so no catalog text is
// read, matched, or interpolated while the agent is running.

/// A message id has to survive becoming a Rust identifier, a struct name, and a `t!` arm.
///
/// Kebab-case in the catalog because that is what `.ftl` uses and what translators will type.
fn id_char(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'
}

/// One piece of a message's text: literal characters, or a hole an argument fills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Arg(String),
}

/// Which case of a select a variant answers to.
///
/// An exact number wins over the plural category for the same value, so `[0] nothing yet` can
/// say something a category never could.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Key {
    Exact(i64),
    Category(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variant {
    pub key: Key,
    pub parts: Vec<Part>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Pattern(Vec<Part>),
    /// `{ $count -> [one] ... *[other] ... }`, with `default` indexing the starred variant.
    Select {
        arg: String,
        variants: Vec<Variant>,
        default: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    /// 1-based, for an error that names the line a person can open.
    pub line: usize,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub line: usize,
    pub problem: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "line {}: {}", self.line, self.problem)
    }
}

fn error(line: usize, problem: impl Into<String>) -> Error {
    Error {
        line,
        problem: problem.into(),
    }
}

/// Every argument the message reads, in first-appearance order, and whether it selects.
///
/// Order is the catalog's, not the call site's: the generated struct takes its fields by name,
/// so nothing downstream depends on this beyond keeping two builds of the same catalog
/// identical.
pub fn arguments(value: &Value) -> Vec<(String, bool)> {
    let mut found: Vec<(String, bool)> = Vec::new();
    let note = |name: &str, selects: bool, found: &mut Vec<(String, bool)>| match found
        .iter_mut()
        .find(|(seen, _)| seen == name)
    {
        Some((_, was)) => *was |= selects,
        None => found.push((name.to_string(), selects)),
    };
    match value {
        Value::Pattern(parts) => {
            for part in parts {
                if let Part::Arg(name) = part {
                    note(name, false, &mut found);
                }
            }
        }
        Value::Select { arg, variants, .. } => {
            note(arg, true, &mut found);
            for variant in variants {
                for part in &variant.parts {
                    if let Part::Arg(name) = part {
                        note(name, false, &mut found);
                    }
                }
            }
        }
    }
    found
}

/// Parse one catalog file.
///
/// Line-oriented rather than a grammar: a message is `id = value`, continued on any following
/// lines that are indented, and a select is the one shape allowed to span lines on its own.
pub fn parse(source: &str) -> Result<Vec<Message>, Error> {
    let lines: Vec<&str> = source.lines().collect();
    let mut messages: Vec<Message> = Vec::new();
    let mut index = 0;

    while index < lines.len() {
        let raw = lines[index];
        let number = index + 1;

        if raw.trim().is_empty() || raw.trim_start().starts_with('#') {
            index += 1;
            continue;
        }

        if raw.starts_with(char::is_whitespace) {
            return Err(error(
                number,
                "indented line does not continue a message: every message starts at column one",
            ));
        }

        let Some((id, first)) = raw.split_once('=') else {
            return Err(error(number, "expected `id = value`"));
        };
        let id = id.trim();
        if id.is_empty() || !id.chars().all(id_char) || id.starts_with('-') || id.ends_with('-') {
            return Err(error(
                number,
                format!("`{id}` is not a message id: lowercase, digits and inner hyphens only"),
            ));
        }
        if let Some(earlier) = messages.iter().find(|m| m.id == id) {
            return Err(error(
                number,
                format!("`{id}` is already defined on line {}", earlier.line),
            ));
        }

        // The value is this line's remainder plus every indented line under it, gathered before
        // anything is interpreted so a select and a wrapped sentence can be told apart.
        let mut block: Vec<&str> = Vec::new();
        if !first.trim().is_empty() {
            block.push(first.trim());
        }
        index += 1;
        while index < lines.len() {
            let next = lines[index];
            if next.trim().is_empty() {
                break;
            }
            if !next.starts_with(char::is_whitespace) {
                break;
            }
            block.push(next.trim());
            index += 1;
        }

        if block.is_empty() {
            return Err(error(number, format!("`{id}` has no value")));
        }

        let value = if block[0].starts_with('{') && block.iter().any(|l| l.contains("->")) {
            parse_select(&block, number)?
        } else {
            Value::Pattern(parse_pattern(&block.join(" "), number)?)
        };

        messages.push(Message {
            id: id.to_string(),
            line: number,
            value,
        });
    }

    Ok(messages)
}

/// `{ $count -> [one] one thing *[other] { $count } things }`, written across lines.
fn parse_select(block: &[&str], number: usize) -> Result<Value, Error> {
    let head = block[0]
        .strip_prefix('{')
        .expect("the caller checked the brace")
        .trim();
    let Some((selector, rest)) = head.split_once("->") else {
        return Err(error(number, "a select opens with `{ $arg ->`"));
    };
    if !rest.trim().is_empty() {
        return Err(error(
            number,
            "the first variant goes on its own line, under `{ $arg ->`",
        ));
    }
    let arg = selector
        .trim()
        .strip_prefix('$')
        .ok_or_else(|| error(number, "a select is on an argument, written `$name`"))?
        .trim()
        .to_string();
    if arg.is_empty() || !arg.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        return Err(error(
            number,
            format!("`${arg}` is not an argument name: lowercase and underscores only"),
        ));
    }

    let mut variants: Vec<Variant> = Vec::new();
    let mut default: Option<usize> = None;
    let mut closed = false;

    for (offset, line) in block[1..].iter().enumerate() {
        let line_number = number + offset + 1;
        if *line == "}" {
            closed = true;
            continue;
        }
        if closed {
            return Err(error(line_number, "text after the select closed"));
        }

        let (starred, body) = match line.strip_prefix('*') {
            Some(rest) => (true, rest),
            None => (false, *line),
        };
        let Some(body) = body.strip_prefix('[') else {
            return Err(error(
                line_number,
                "a variant is `[key] text` or `*[key] text`",
            ));
        };
        let Some((key, text)) = body.split_once(']') else {
            return Err(error(line_number, "a variant key is not closed with `]`"));
        };

        let key = key.trim();
        let key = match key.parse::<i64>() {
            Ok(exact) => Key::Exact(exact),
            Err(_) => {
                if key.is_empty() || !key.chars().all(|c| c.is_ascii_lowercase()) {
                    return Err(error(
                        line_number,
                        format!(
                            "`{key}` is not a variant key: a whole number or a plural category"
                        ),
                    ));
                }
                Key::Category(key.to_string())
            }
        };
        if variants.iter().any(|v| v.key == key) {
            return Err(error(line_number, "the same variant key appears twice"));
        }
        if starred {
            if default.is_some() {
                return Err(error(
                    line_number,
                    "a select has one default variant, not two",
                ));
            }
            default = Some(variants.len());
        }

        variants.push(Variant {
            key,
            parts: parse_pattern(text.trim(), line_number)?,
        });
    }

    if !closed {
        return Err(error(number, "the select is never closed with `}`"));
    }
    let Some(default) = default else {
        return Err(error(
            number,
            "a select needs a default variant, marked `*`, for the cases nothing else answers",
        ));
    };

    Ok(Value::Select {
        arg,
        variants,
        default,
    })
}

/// Literal text with `{ $name }` holes in it.
fn parse_pattern(text: &str, number: usize) -> Result<Vec<Part>, Error> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut rest = text;

    while let Some(open) = rest.find('{') {
        literal.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            return Err(error(number, "`{` is never closed"));
        };
        let name = after[..close].trim();
        let Some(name) = name.strip_prefix('$') else {
            return Err(error(
                number,
                format!("`{{{name}}}` is not an argument: write `{{ $name }}`"),
            ));
        };
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            return Err(error(
                number,
                format!("`${name}` is not an argument name: lowercase and underscores only"),
            ));
        }
        if !literal.is_empty() {
            parts.push(Part::Text(std::mem::take(&mut literal)));
        }
        parts.push(Part::Arg(name.to_string()));
        rest = &after[close + 1..];
    }

    literal.push_str(rest);
    if !literal.is_empty() {
        parts.push(Part::Text(literal));
    }
    Ok(parts)
}

/// `confirm-write-title` as a Rust identifier.
pub fn snake_case(id: &str) -> String {
    id.replace('-', "_")
}

/// `confirm-write-title` as a type name, for the struct that carries its arguments.
pub fn camel_case(id: &str) -> String {
    id.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn only(source: &str) -> Message {
        let mut parsed = parse(source).expect("the catalog parses");
        assert_eq!(parsed.len(), 1, "the fixture defines one message");
        parsed.remove(0)
    }

    fn text(value: &Value) -> String {
        match value {
            Value::Pattern(parts) => parts
                .iter()
                .map(|part| match part {
                    Part::Text(text) => text.clone(),
                    Part::Arg(name) => format!("<{name}>"),
                })
                .collect(),
            Value::Select { .. } => panic!("a select has no single text"),
        }
    }

    /// The plain case, which is most of the catalog.
    #[test]
    fn a_message_is_an_id_and_the_text_after_the_equals() {
        let message = only("write-title = approve this write?");
        assert_eq!(message.id, "write-title");
        assert_eq!(text(&message.value), "approve this write?");
    }

    /// Catalog lines are read by people, and a sentence too long for a screen has to be able to
    /// wrap without the wrap becoming part of what is displayed.
    #[test]
    fn a_wrapped_message_is_one_line_of_text() {
        let message =
            only("note =\n    untrusted: nobody has read this,\n    and the model never saw it\n");
        assert_eq!(
            text(&message.value),
            "untrusted: nobody has read this, and the model never saw it"
        );
    }

    /// The hole an argument fills is a part of its own, so the generated code interpolates a
    /// value rather than assembling a format string at run time.
    #[test]
    fn an_argument_is_a_hole_in_the_text() {
        let message = only("in-directory = in { $path }, on { $branch }");
        assert_eq!(text(&message.value), "in <path>, on <branch>");
        assert_eq!(
            arguments(&message.value),
            vec![("path".to_string(), false), ("branch".to_string(), false)]
        );
    }

    /// A translator has to be able to see which variant answers a case nothing else does.
    #[test]
    fn a_select_records_its_variants_and_which_one_is_the_default() {
        let message = only(
            "turns = { $count ->\n    [0] nothing yet\n    [one] { $count } turn\n   *[other] { $count } turns\n    }\n",
        );
        let Value::Select {
            arg,
            variants,
            default,
        } = &message.value
        else {
            panic!("a select was written");
        };
        assert_eq!(arg, "count");
        assert_eq!(variants.len(), 3);
        assert_eq!(variants[0].key, Key::Exact(0));
        assert_eq!(variants[1].key, Key::Category("one".to_string()));
        assert_eq!(*default, 2);
    }

    /// The argument a select is on is the one whose Rust type has to be a number, so the
    /// distinction has to survive parsing.
    #[test]
    fn the_argument_a_select_is_on_is_reported_as_selecting() {
        let message = only(
            "files = { $count ->\n    [one] { $count } file in { $dir }\n   *[other] { $count } files in { $dir }\n    }\n",
        );
        assert_eq!(
            arguments(&message.value),
            vec![("count".to_string(), true), ("dir".to_string(), false)]
        );
    }

    /// Without one, some count renders as nothing at all, and the language it happens in is the
    /// one nobody reviewing the change reads.
    #[test]
    fn a_select_with_no_default_variant_is_refused() {
        let refusal =
            parse("turns = { $count ->\n    [one] one turn\n    [other] some turns\n    }\n")
                .expect_err("a select without a default is not a message");
        assert!(refusal.problem.contains("default"), "{refusal}");
    }

    /// Two definitions mean one silently wins, and which one depends on the order of a file.
    #[test]
    fn a_repeated_message_id_is_refused() {
        let refusal =
            parse("greeting = hello\ngreeting = hi\n").expect_err("an id defines one message");
        assert!(refusal.problem.contains("already defined"), "{refusal}");
    }

    /// `{ name }` without the sigil is a Fluent term reference, which this subset does not have,
    /// and quietly treating it as text would put a brace on somebody's screen.
    #[test]
    fn a_placeholder_that_is_not_an_argument_is_refused() {
        let refusal =
            parse("greeting = hello { name }\n").expect_err("a placeholder names an argument");
        assert!(refusal.problem.contains("$name"), "{refusal}");
    }

    /// An error a translator cannot locate is an error they cannot fix.
    #[test]
    fn a_refusal_names_the_line_it_is_about() {
        let refusal = parse("# a comment\n\ngreeting = hello\nbroken\n")
            .expect_err("a line with no equals is not a message");
        assert_eq!(refusal.line, 4);
    }

    /// Ids are written the way `.ftl` writes them and read the way Rust reads them.
    #[test]
    fn an_id_becomes_a_rust_name_either_way_it_is_needed() {
        assert_eq!(snake_case("confirm-write-title"), "confirm_write_title");
        assert_eq!(camel_case("confirm-write-title"), "ConfirmWriteTitle");
    }
}
