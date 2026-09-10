---
id: LSP
title: lsp
status: normative
governs:
  - crates/lsp/src/lib.rs
  - crates/lsp/src/protocol.rs
  - crates/lsp/src/server.rs
  - crates/agent/src/lsp.rs
guards:
  - symbol: Operation::parse
  - symbol: locations_in
---

## Scope

Asking a language server where a symbol is defined, what refers to it, and what it is. The
operation, the file, and the position are routing; there are no content arguments. The result is a
set of locations, or a refusal.

Confinement of the server process is [sandboxing.md](../sandboxing.md). Why the built-in tools are
not MCP servers is [mcp.md](../mcp.md), and this tool is native for exactly that reason: a call
whose parts must be labelled separately cannot go behind an opaque boundary.

## Why this is admissible where a shell is not

[TOOL-2](tool-surface.md#TOOL-2) asks what a tool's routing field is, and refuses the tool if a
person could not approve that field alone. An LSP request answers easily: an operation from a fixed
set, a path, and two integers. There is no payload beside it, so unlike a shell string there is
nothing in the call that is destination and content at once.

The hard part is not the request. It is that the **answer** is a set of paths and line numbers read
out of files nobody vouched for, and a path is the one thing routing is made of. That is what
[LSP-3](#LSP-3) is about, and it is the clause to read first.

## Clauses

<a id="LSP-1"></a>
### LSP-1: the whole request is routing, and the operation is a closed set

`operation`, `path`, `line` and `character` are all `(T,pub)`. The operation is one of a fixed list
this repository knows: a definition, the references to a symbol, hover text, the symbols in a
document, the symbols in the workspace, implementations of a trait, and the two directions of a call
hierarchy. A name not on the list is refused rather than forwarded, so what the server is asked is
decided here and never by whatever a request happened to contain.

There are no content arguments at all, which makes this the only tool besides `read_file` and
`list_files` whose call carries nothing untrusted.

**Why the list is closed.** LSP is an open protocol and a server advertises methods of its own,
including ones that apply a workspace edit. Forwarding a method name would make the tool's blast
radius a property of the server rather than of this repository, and `workspace/applyEdit` is a
write nobody approved. The list holds read-only queries, and a method that changes a file is not on
it and must not be added: writing goes through `write_file` and `edit_file`, where a person sees a
diff.

`verified-by: bravebot_lsp::protocol::an_operation_outside_the_closed_set_is_refused`
`verified-by: bravebot_lsp::protocol::every_offered_operation_is_a_read`
`verified-by: bravebot_agent::lsp::the_position_is_routing_and_must_be_trusted`
`verified-by: bravebot_core::policy::routing_refuses_untrusted_values`

<a id="LSP-2"></a>
### LSP-2: a position is a position, and is not derived from content

`line` and `character` are integers the planner proposes, promoted the way a read's `offset` is
under [READ-4](read-file.md#READ-4). Nothing reads the file to work out where a symbol is, and no
part of a previous result is parsed to build the next request: where a planner wants the definition
of what a search found, it says the line the search reported, which is a number it was told.

**Why.** A position computed by scanning bytes would be a decision taken from content, and on an
untrusted file that is [LABEL-5](../labels.md#LABEL-5). Keeping the position something the planner
states, rather than something the driver derives, is what keeps this tool out of that.

An out-of-range position is answered with nothing found, not an error: a file changes under an
agent, and a stale line number is an ordinary thing rather than a fault.

`verified-by: bravebot_agent::lsp::a_position_out_of_range_finds_nothing_rather_than_failing`
`verified-by: bravebot_agent::lsp::no_position_is_computed_from_the_file`

<a id="LSP-3"></a>
### LSP-3: a location is structure; the text at it is content

A location is a path, a line and a character, plus the symbol kind for the operations that report
one. Those reach the planner whatever the trust map says about the file they name, exactly as a
line count does for a file the planner may not read and as an exit status does under
[RUN-13](run.md#RUN-13).

The **text** at a location is content and gets no such treatment. Hover text, a signature, a
docstring, a source excerpt: each is bytes the file chose, so each is labelled from the trust map
by [LABEL-2](../labels.md#LABEL-2) and quarantined when it is untrusted. One result may therefore
be a visible list of locations whose hover text is a reference.

**Why a location is structure.** It was read off the server's index, computed from the file's
syntax, and not selected by the file's own bytes in the sense that matters: an attacker who owns
`vendor/lib.js` cannot use `goToDefinition` to put a sentence in the planner's context, because the
shape of what comes back is a path and two integers and there is nowhere in it for prose to sit. A
path is already the kind of thing the planner proposes and the promote gate vouches for, and a line
number carries no more instruction than a line count does.

**What an attacker does get, stated plainly.** They choose *which* path and *which* line the planner
is told about, within their own file, by arranging their code so a symbol resolves where they like.
That is influence over the planner's attention. It is not influence over an effect: a location is
not promoted to routing by having been returned, so a write to a path that came back from here is a
path the planner named and a person approved from a diff, like any other. The residue is that a
planner may spend a step reading somewhere useless, which is the cost of a search returning a
misleading hit and is not new.

**What must not follow from this clause.** That a location may be *used* as routing without passing
the gate every other proposed path passes. Nothing here makes `crates/foo/src/lib.rs:42` trusted
because a server said it; it makes the fact of it sayable. A `read_file` of that path is promoted on
its own merits under [READ-4](read-file.md#READ-4), and its contents are labelled by the trust map
as they would be had the planner guessed the path.

`verified-by: bravebot_core::policy::a_location_from_an_untrusted_file_is_still_reportable`
`verified-by: bravebot_core::policy::a_location_is_not_routing_for_a_later_effect`
`verified-by: bravebot_lsp::protocol::a_location_carries_no_text_from_the_file`
`verified-by: bravebot_agent::lsp::hover_text_from_an_untrusted_file_is_quarantined`
`verified-by: bravebot_agent::lsp::hover_text_from_a_trusted_file_is_shown`
`verified-by: bravebot_agent::lsp::locations_are_listed_even_where_the_text_is_quarantined`

<a id="LSP-4"></a>
### LSP-4: a location outside the workspace is reported as outside it

A definition in a dependency, a toolchain source, or anywhere else beyond the working directory is
returned with its path rendered as what it is rather than as a workspace-relative path: the planner
is told this is outside the workspace, and the path is not spelled as though `read_file` would open
it.

**Why.** Most of what `goToDefinition` finds in a real Rust workspace is in `~/.cargo/registry`,
and this is the common case rather than an edge. Rendering it like a workspace path invites a read
that is refused, which costs a round and teaches the planner nothing. Saying where it is lets the
planner decide whether it needed the dependency's source at all.

Nothing outside the workspace becomes readable by having been named here. The confined-read gate is
unchanged, so a planner that asks for the file anyway is refused as it would be for any other path
outside the tree.

`verified-by: bravebot_agent::lsp::a_location_outside_the_workspace_says_so`
`verified-by: bravebot_agent::lsp::naming_an_outside_location_does_not_make_it_readable`

<a id="LSP-5"></a>
### LSP-5: a server is a program a person approved, and runs with their own access

Starting one is put to the user, and what they approve is a process that runs for the session with
the access their own shell would give it. Not confined, and the record of what was approved belongs
to the session in the way [RUN-9](run.md#RUN-9) describes.

**Why not confined, when a stdio MCP server is.** [sandboxing.md](../sandboxing.md) says what
confinement is for: code nobody vouched for. It says in the same breath that "a program the user
asked for runs with the access their own shell would give it", and [RUN-10](run.md#RUN-10) says
programs "are not enumerated and not confined". A language server is the second kind of thing. The
analogy to [MCP-3](../mcp.md#MCP-3) does not hold: an MCP server is a tool somebody found on the
internet, and a language server is part of the toolchain the user already builds with.

**Why confinement is not an option here.** A language server indexes by running its ecosystem's build
tooling: rust-analyzer builds a crate graph with `cargo metadata`, which needs a subprocess and
somewhere to write, and a Node server wants a private directory before it will start. A profile
denying those does not yield a confined server that answers; it yields one whose index never settles,
so every answer is marked partial by [LSP-7](#LSP-7). The choice is between a server with the user's
access and no working tool.

**What is being approved, said plainly at the prompt.** The server reads the whole tree and the
dependency sources, runs the build tooling of its ecosystem, and for Rust that means `build.rs` and
proc macros out of `Cargo.lock` execute. That is code from the dependency tree running with the
user's access. It is the same thing `cargo test` does and the same thing `run` does after
[RUN-7](run.md#RUN-7), and it must be asked for in those terms rather than described as a lookup.

**What does not change, and this is the important half.** The safety property here was never the
sandbox. It is the label on what comes back: [RUN-4](run.md#RUN-4)'s reasoning applies unchanged, so
a server's output is untrusted, hover text is quarantined by the trust map, and [LSP-3](#LSP-3) is
what lets a location through. None of those rest on confinement and none of them move. A server that
can read the disk is not a server that can put prose in the planner's context.

`verified-by: bravebot_lsp::server::starting_a_server_is_put_to_a_person`
`verified-by: bravebot_lsp::server::a_refused_server_does_not_start`
`verified-by: bravebot_lsp::server::a_server_is_not_asked_about_twice_in_a_session`

<a id="LSP-6"></a>
### LSP-6: no server means no answer, and says which

Where no server is configured for a file's language, where the binary is absent, and where a server
was configured but failed to start are three different sentences, and none of them is an empty
result. Nothing falls back to searching the tree.

**Why.** An absent server reported as "no references found" is a false negative that reads as
proof, which is the failure [MCP-5](../mcp.md#MCP-5) and
[SEARCH-3](search.md#SEARCH-3) each exist to prevent, and the consequence here is worse than a
wasted round: a planner that believes nothing calls a function will delete it. Falling back to a
substring search would be the same error wearing an answer, since the two questions have different
answers and only one of them was asked.

`verified-by: bravebot_lsp::server::an_unconfigured_language_is_reported_as_unconfigured`
`verified-by: bravebot_lsp::server::a_missing_binary_is_reported_as_missing`
`verified-by: bravebot_lsp::server::a_server_that_fails_to_start_is_reported_as_such`
`verified-by: bravebot_agent::lsp::nothing_found_is_not_reported_as_no_server`
`verified-by: bravebot_agent::lsp::no_server_does_not_fall_back_to_a_search`

<a id="LSP-7"></a>
### LSP-7: a server that has not finished indexing says so

An answer given while the server is still indexing is marked as partial, in the same words a
truncated search uses under [SEARCH-3](search.md#SEARCH-3), and for the same reason: a
`findReferences` run against a half-built index returns some references and looks exactly like one
that returned all of them. The notice reaches the planner whether or not the text of the result was
quarantined, since a notice inside a body nobody may read tells nobody anything.

A request may wait for indexing to finish, bounded by a limit; reaching the limit answers with what
the index has and the notice above, rather than failing.

**Why bounded.** Indexing a large workspace outlasts a person's patience, and a turn held open with
nothing to show for it is [RUN-11](run.md#RUN-11)'s problem arriving by another road.

`verified-by: bravebot_lsp::server::an_answer_during_indexing_is_marked_partial`
`verified-by: bravebot_lsp::server::a_settled_index_makes_no_partial_claim`
`verified-by: bravebot_agent::lsp::a_partial_answer_says_so_even_when_quarantined`

<a id="LSP-8"></a>
### LSP-8: one server per language per session, shut down with it

A server is long-lived: it is started on the first request for its language, kept for the session,
and shut down when the session ends. It is not started at launch, and a session that asks nothing
of a language starts nothing.

The process is killed if it does not exit on request, so a server that ignores `shutdown` does not
outlive the agent that started it.

**Why kept rather than per-call.** Indexing is the whole cost, and paying it per request would make
every call slower than the search it replaces.

**Why started lazily.** Most sessions touch one language, and indexing a workspace for a server
nobody asks about spends a person's CPU on nothing.

`verified-by: bravebot_lsp::server::a_server_is_started_once_and_reused`
`verified-by: bravebot_lsp::server::no_server_starts_until_a_request_needs_one`
`verified-by: bravebot_lsp::server::a_server_that_ignores_shutdown_is_killed`
`verified-by: bravebot_lsp::server::dropping_the_set_stops_every_server`

<a id="LSP-9"></a>
### LSP-9: the capability is separate, and a delegate does not inherit it

An `lsp` call needs its own capability, so a run that was granted file reads has not thereby been
granted a language server. A delegate gets it only where its capability set says so.

**Why separate from `FileRead`.** They are not the same act. A read opens one named file inside the
tree; a server reads the whole tree and the dependency sources beside it, and keeps a process alive
doing so. A capability set that could not tell those apart could not describe the narrower one.

`verified-by: bravebot_lsp::server::a_request_without_the_capability_is_refused`
`verified-by: bravebot_core::capability::the_lsp_capability_produces_no_routing_safe_output`
`verified-by: bravebot_agent::lsp::a_delegate_without_the_capability_is_refused`

<a id="LSP-10"></a>
### LSP-10: the index is cached under `~/.bravebot`, and that directory confers nothing on it

A server is given a cache directory of its own, keyed by the workspace it indexes, under the
directory this process already owns. Never inside the workspace, and never read by the driver.

**Why not the workspace.** A cache under `target/` would make a question about a symbol change the
tree the user is working in, and a directory that appeared as a side effect of a read is the sort of
write [write-file.md](write-file.md) exists to put in front of somebody.

An index an earlier build left under a path this one does not use is removed rather than left
where it is. Nothing reads it, so it is an index of the user's source sitting at whatever mode it
was made with, for as long as the machine lasts.

Every directory a server is pointed at is created by this process, at the modes
[state-directory.md](../state-directory.md#STATE-1) gives them, and a server whose directories
cannot be created does not start. A server handed one that is not there creates it itself, at the
umask, and fills it with an index derived from every file in the workspace. Dropping the variable
instead would put that index in the workspace, which is what this clause forbids, so the failure is
reported under [LSP-6](#LSP-6) rather than worked around.

**Why not a temporary directory.** The cost this avoids is the indexing, and an index thrown away at
the end of a session pays it again at the start of the next. `~/.bravebot` is where this process
already keeps what outlives a session, so a cache there inherits
[TRUST-11](../trust-map.md#TRUST-11) and [incognito.md](../incognito.md) rather than needing rules of
its own.

**It is not trusted, and `~/.bravebot` is exactly why somebody will think it is.**
[TRUST-11](../trust-map.md#TRUST-11) makes that directory trusted by provenance, and the provenance
it means is that the user wrote what is in it. This cache is written by us and holds bytes derived
from workspace files, so [LABEL-2](../labels.md#LABEL-2) taints it with those files' labels and
[LABEL-7](../labels.md#LABEL-7) forbids recovering a better one. Reading it back as trusted because
of where it sits would be laundering, with a directory doing the work of a constructor. Nothing here
does: the only reader is the server, the driver never opens it, and what reaches the planner is what
[LSP-3](#LSP-3) governs.

**Incognito writes none of it**, since that mode adds nothing to `~/.bravebot`. The server still runs
and still answers; it re-indexes each session and says so under [LSP-7](#LSP-7). That is the same
trade incognito already makes for the session record.

`verified-by: bravebot_lsp::server::the_cache_is_outside_the_workspace`
`verified-by: bravebot_lsp::server::the_cache_sits_directly_under_the_directory_it_is_given`
`verified-by: bravebot_lsp::server::an_index_an_earlier_build_left_too_deep_is_removed`
`verified-by: bravebot_lsp::server::a_nested_directory_that_holds_no_index_is_left_where_it_is`
`verified-by: bravebot_lsp::server::the_cache_is_keyed_by_the_workspace`
`verified-by: bravebot_lsp::server::an_incognito_session_is_given_no_cache`
`verified-by: bravebot_lsp::server::the_cache_is_never_read_by_the_driver`
`verified-by: bravebot_lsp::server::a_server_whose_index_directory_cannot_be_made_private_does_not_start`

## Known costs

- **A location is attention, and attention can be steered.** [LSP-3](#LSP-3) grants that an
  attacker who owns a file in the tree decides which paths and lines come back from a query about
  their code. Nothing here bounds how interesting they can make a location look. What is bounded is
  what a location can do: it is never routing, so the worst case is a wasted read of a file the
  planner was already allowed to read.

- **Hover text is where the value is, and hover text is content.** For an untrusted file the useful
  half of `hover` comes back as a reference. That is the same trade `read_file` makes and is not
  new, but it is worth saying that this tool is at its weakest exactly where a codebase is least
  vouched for.

- **A server runs the dependency tree's code, and that is the price of the tool working at all.**
  [LSP-5](#LSP-5) grants a server the user's own access, so for Rust `build.rs` and proc macros out of
  `Cargo.lock` execute. The alternative is not a safer tool but no tool, since a server whose index
  never settles answers only that it is unsure. What bounds this is that the user is asked in those
  words, and that nothing about the label on the output depends on their answer.

- **An index survives between sessions, and nothing prunes it.** [LSP-10](#LSP-10) keeps a cache per
  workspace under `~/.bravebot`, which is what makes the second session fast. It is not small: this
  workspace's is a few hundred megabytes, since what rust-analyzer keeps there is a build directory. A
  machine that has been in many workspaces holds one for each, and nothing removes them.

- **A server that goes quiet is indistinguishable from one that is working.** The bound in
  [LSP-7](#LSP-7) is what separates them, and it has to be enforced against a process that may send
  nothing at all rather than only between the messages it does send.

## Open questions

- Whether a server needs children of its own at all. rust-analyzer accepts a crate graph through
  `linkedProjects`, so `cargo metadata` could be run once through [`run`](run.md), approved under
  [RUN-7](run.md#RUN-7), and its output handed over. That puts the approval where this repository
  usually puts it and narrows what [LSP-5](#LSP-5) has to grant, at the cost of a second moving part
  per ecosystem.

- Whether the cache needs an eviction rule, and what it should be keyed on. [LSP-10](#LSP-10)
  accumulates one directory per workspace and nothing removes them.

- Whether a location should carry the symbol's *name* as well as its position. A name is a token
  the file chose, so it is content by [LSP-3](#LSP-3) and quarantined; but a list of positions with
  no names is hard to act on, and the planner usually knows the name already because it asked about
  it. Left out for now: adding it later is a clause, whereas taking it back is a regression.

- Whether `workspaceSymbol` belongs on the closed list at all. Its query is a string the planner
  writes, which is fine, but its answer ranges over the whole tree rather than starting from a
  position the planner already had, so it is the one operation whose result set an attacker can
  enter without being asked about.
