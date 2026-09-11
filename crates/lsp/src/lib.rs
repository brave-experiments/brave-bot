//! Language server client: positions in, locations out.
//!
//! What makes this admissible where a shell is not is the shape of a request. An operation from a
//! closed set, a path, and two integers: all routing, no payload, nothing a person could not
//! approve on its own. [tools/lsp.md] is the spec, and LSP-3 is the clause to read first, because
//! the interesting part is not the request but the answer.
//!
//! An answer is split in two, and the split is visible in the types here. A [`protocol::Location`]
//! is structure: a path, a line, a character, a symbol kind, read off the server's index. It has
//! nowhere for prose to sit, deliberately, and that is what lets it reach the planner whatever the
//! trust map says about the file it names. Hover text is the other half: bytes the file chose, so it
//! is labelled from the trust map and quarantined like anything else nobody vouched for.
//!
//! This is native rather than an MCP server for the reason [mcp.md] gives: a call whose parts must
//! be labelled separately cannot go behind a boundary that labels the whole thing at once.
//!
//! [tools/lsp.md]: ../../../docs/specs/tools/lsp.md
//! [mcp.md]: ../../../docs/specs/mcp.md

#![forbid(unsafe_code)]

pub mod protocol;
pub mod server;
#[cfg(test)]
mod testutil;

pub use protocol::{Location, Operation, SymbolKind};
pub use server::{Language, Question, Server, Servers, Starting};

use bravebot_core::policy::Denial;
use std::fmt;

pub type LspResult<T> = Result<T, LspError>;

/// What one question produced.
///
/// Two fields for two footings, which is LSP-3 in the shape of a struct. `locations` is structure
/// and travels; `text` is content and is labelled by whoever holds the trust map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answer {
    /// Where the server says the thing is. Never carries a byte the file wrote.
    pub locations: Vec<Location>,
    /// The text at the position, for the operations that report any.
    pub text: Option<String>,
    /// Whether the index was still building, so this answer may be short of the truth.
    ///
    /// LSP-7. A fact about the answer rather than a sentence inside it, so it survives quarantine:
    /// a notice written into a body nobody may read tells nobody anything.
    pub partial: bool,
}

#[derive(Debug)]
pub enum LspError {
    /// A person was asked whether to start the server and said no.
    Refused { language: server::Language },
    /// The policy refused the call.
    Denied(Denial),
    /// This file's language has no server in the table.
    NoServerFor { path: String },
    /// A whole-tree query was asked before any server was running, so there is nothing to ask.
    NoServerForQuery,
    /// The server for this language is not installed.
    NoBinary {
        language: server::Language,
        program: &'static str,
    },
    /// The server was there and did not start.
    Start {
        language: server::Language,
        detail: String,
    },
    /// The server exited while we were talking to it.
    Exited { language: server::Language },
    /// The transport failed, or the server sent something unusable.
    Transport {
        language: server::Language,
        detail: String,
    },
    /// The server did not answer inside the budget.
    TimedOut {
        language: server::Language,
        method: String,
    },
    /// The server returned a JSON-RPC error.
    Server {
        language: server::Language,
        code: i64,
        message: String,
    },
    /// The operation named is not one of ours.
    UnknownOperation { named: String },
}

/// Each variant says which of LSP-6's three failures it is, in words that cannot be mistaken for an
/// answer about the code. "No references found" and "no server ran" must never render alike.
impl fmt::Display for LspError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Refused { language } => write!(
                f,
                "the {} language server was not started, because the user declined; nothing here \
                 says anything about the code, and asking again will not change it",
                language.as_str()
            ),
            Self::Denied(denial) => write!(f, "{denial}"),
            Self::NoServerForQuery => write!(
                f,
                "workspaceSymbol searches a language server's index and no server is running yet, \
                 so this question was not asked of one; ask about a symbol in a file first, which \
                 starts the server for that file's language, then this will search it"
            ),
            Self::NoServerFor { path } => write!(
                f,
                "no language server is configured for {path}, so this question was not asked of \
                 one; nothing here says anything about the code"
            ),
            Self::NoBinary { language, program } => write!(
                f,
                "the {} language server is not installed: {program} was not found on PATH, so \
                 this question was not asked of one",
                language.as_str()
            ),
            Self::Start { language, detail } => write!(
                f,
                "the {} language server did not start: {detail}",
                language.as_str()
            ),
            Self::Exited { language } => write!(
                f,
                "the {} language server exited before replying",
                language.as_str()
            ),
            Self::Transport { language, detail } => write!(
                f,
                "the {} language server sent something unusable: {detail}",
                language.as_str()
            ),
            Self::TimedOut { language, method } => write!(
                f,
                "the {} language server did not answer {method} in time",
                language.as_str()
            ),
            Self::Server {
                language,
                code,
                message,
            } => write!(
                f,
                "the {} language server returned error {code}: {message}",
                language.as_str()
            ),
            Self::UnknownOperation { named } => write!(
                f,
                "'{named}' is not an operation this tool offers; the operations are \
                 goToDefinition, findReferences, hover, documentSymbol, workspaceSymbol, \
                 goToImplementation, incomingCalls and outgoingCalls"
            ),
        }
    }
}

impl std::error::Error for LspError {}

impl LspError {
    /// Whether no server answered, as opposed to one that answered and came back empty.
    ///
    /// The distinction LSP-6 exists for: a planner that reads "no references" as proof will delete a
    /// function that is called from a file no server ever indexed. A refusal belongs here too, since
    /// a server nobody allowed to start is a server that said nothing.
    pub fn is_absence_of_a_server(&self) -> bool {
        matches!(
            self,
            Self::NoServerFor { .. }
                | Self::NoServerForQuery
                | Self::NoBinary { .. }
                | Self::Start { .. }
                | Self::Refused { .. }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An answer with nothing in it is still an answer, and must be distinguishable from never
    /// having asked. LSP-6.
    #[test]
    fn an_empty_answer_is_not_an_absent_server() {
        let empty = Answer::default();
        assert!(empty.locations.is_empty());
        assert!(!empty.partial);

        assert!(
            LspError::NoServerFor {
                path: "a.txt".into()
            }
            .is_absence_of_a_server()
        );
        assert!(
            LspError::NoBinary {
                language: server::Language::Rust,
                program: "rust-analyzer"
            }
            .is_absence_of_a_server()
        );
        assert!(
            LspError::Start {
                language: server::Language::Rust,
                detail: "boom".into()
            }
            .is_absence_of_a_server()
        );

        // A server that answered and failed is not an absent one.
        assert!(
            !LspError::Server {
                language: server::Language::Rust,
                code: -32601,
                message: "unsupported".into()
            }
            .is_absence_of_a_server()
        );
        assert!(
            !LspError::TimedOut {
                language: server::Language::Rust,
                method: "textDocument/references".into()
            }
            .is_absence_of_a_server()
        );
    }

    /// LSP-1: a name off the closed set is refused, and the refusal says what the set is so the
    /// planner does not spend a round guessing.
    #[test]
    fn an_unknown_operation_names_the_ones_that_exist() {
        let said = LspError::UnknownOperation {
            named: "workspace/applyEdit".into(),
        }
        .to_string();
        assert!(said.contains("not an operation"), "{said}");
        assert!(said.contains("goToDefinition"), "{said}");
        assert!(said.contains("findReferences"), "{said}");
    }

    /// No failure may read as a statement about the code. This is the whole of LSP-6's why.
    #[test]
    fn no_failure_reads_as_an_answer_about_the_code() {
        let failures = [
            LspError::NoServerFor {
                path: "a.txt".into(),
            },
            LspError::NoBinary {
                language: server::Language::Rust,
                program: "rust-analyzer",
            },
            LspError::Start {
                language: server::Language::Rust,
                detail: "boom".into(),
            },
            LspError::Exited {
                language: server::Language::Rust,
            },
            LspError::TimedOut {
                language: server::Language::Rust,
                method: "textDocument/references".into(),
            },
        ];
        for failure in failures {
            let said = failure.to_string();
            for forbidden in ["no references", "no matches", "not used", "nothing found"] {
                assert!(
                    !said.contains(forbidden),
                    "{said} reads as an answer about the code"
                );
            }
        }
    }
}
