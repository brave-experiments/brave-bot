---
id: FETCH
title: fetch_url
status: normative
governs:
  - crates/agent/src/tools.rs
  - crates/core/src/policy.rs
  - crates/core/src/url.rs
  - crates/net/src/lib.rs
guards:
  - symbol: Policy::before_fetch
  - symbol: Policy::before_network
---

## Scope

Fetching an http or https URL. `url` is routing; there are no content arguments. The result is a
reference. Where a rule about a host is written and how one is matched is
[permissions.md](../permissions.md); what a label means is [labels.md](../labels.md).

## Why a URL is admissible as routing

A URL is one destination, and a person can read it and say yes or no to it. That is the whole test
[TOOL-2](tool-surface.md#TOOL-2) applies, and it is what a shell string fails: there is no second
thing bundled into a URL that an approval would be silently covering. The body that comes back is
content, carried and never consulted, exactly as a quarantined file's is.

## Clauses

<a id="FETCH-1"></a>
### FETCH-1: a fetched body is untrusted and public, and no approval changes that

The label comes from `Capability::WebFetch`, so it is fixed before the request goes out and nothing
about the reply can raise it. The planner gets a reference and never the bytes: it may hand them to
a processor, or write them to a file with `contents_ref`, and it cannot read them or be told what
they say.

**Why.** This is the difference between a fetch and a run, and the one place the two could be
confused. Vouching for a command trusts what it prints, because a person can read one command and
answer for both its effect and its output. A host answers every later request however it likes, so
approving one is consent to talk to it and says nothing whatever about what it returns. If an
approval could trust a body, one yes would turn a web page into routing, which is the whole thing
the labels exist to stop.

Public rather than private, because a fetched page is not the user's own data: it may be written
without a further question about confidentiality, and there is nothing of theirs in it to release.

`verified-by: bravebot_core::policy::approving_a_fetch_never_trusts_what_comes_back`
`verified-by: bravebot_agent::turn::a_fetched_page_never_reaches_the_planner`
`verified-by: bravebot_agent::turn::a_fetched_page_can_be_processed_and_written_without_being_read`

<a id="FETCH-2"></a>
### FETCH-2: every fetch asks, unless a rule names the host

A URL nobody has ruled on is put to a person, who is shown the URL and, separately, the host it
will reach. An `allow` rule matching the host answers the prompt; a `deny` rule refuses without
asking.

```
╭ fetch this? ─────────────────────────────────────────────╮
│Fetch https://docs.example.com/api                        │
│  talking to docs.example.com                             │
│                                                          │
│  what comes back stays quarantined however you answer:   │
│  the model can pass it to a processor or write it to a    │
│  file, and cannot read it or be told what it says.       │
│                                                          │
│  y fetch it    n don't    ctrl-c stop                    │
╰──────────────────────────────────────────────────────────╯
```

The host is drawn on its own line rather than left inside the URL, and is taken from the URL by the
parser rather than from what the string looks like. `https://example.com@evil.test/` names one site
to a person skimming it and reaches another, so what they are answering about is put where it cannot
be misread.

**Why the host and not the URL.** A rule is a standing statement about who may be talked to, which
is a thing a person can hold in their head. A path is where a URL carries the particular thing being
asked for, so a rule matching one would be answering a different question on every call.

`verified-by: bravebot_core::policy::a_host_nobody_has_ruled_on_is_put_to_a_person`
`verified-by: bravebot_core::policy::a_rule_written_in_advance_answers_the_fetch_prompt`
`verified-by: bravebot_core::url::userinfo_is_not_mistaken_for_the_host`
`verified-by: bravebot_agent::turn::a_domain_rule_lets_a_fetch_through_without_asking`
`verified-by: bravebot_agent::turn::a_refused_fetch_sends_no_request`

<a id="FETCH-3"></a>
### FETCH-3: an approval is bound to the URL it was given for, and nothing is remembered

The endorsement names that exact URL and is single-use, so an answer cannot be spent on another
request. There is no "always" at this prompt: approving one fetch records nothing, and the next one
asks again. Standing permission is a rule in a settings file, which is a person writing a host down
in advance rather than answering a question about one URL.

**Why no remembering.** `run` offers it because vouching is keyed to an exact argv, which is the
thing the person read. The equivalent for a fetch would be keyed to a host, and a host is not what
was in front of them: they approved one page, and every later URL on that host is one they have not
seen. Offering it would collect a standing grant from a question about something narrower.

`verified-by: bravebot_core::policy::an_approval_for_one_url_does_not_fetch_another`
`verified-by: bravebot_core::policy::a_fetch_without_an_endorsement_is_refused`

<a id="FETCH-4"></a>
### FETCH-4: a redirect may not leave the host that was approved

Every hop passes the egress gate, and while a fetch is in flight that gate also checks the hop's
host against the one approved. A redirect within that host is ordinary and continues; a redirect
anywhere else is refused unless a rule allows that host too. A `deny` rule refuses a hop whatever
else says otherwise.

**Why.** The approval named one URL, and the host at the end of a redirect chain is one nobody was
ever shown. Without this, a page on a host somebody approved could send the request wherever it
liked, and the answer a person gave would be covering a destination they never saw.

**What this gate is not.** Reaching the configured model endpoint is egress too, and it goes through
the same gate. It is this program operating rather than something a turn asked for, so a `WebFetch`
rule does not apply to it: a rule about a website must not be able to stop the agent from talking to
its own backend. The distinction is whether a `fetch_url` call is in flight, which the policy layer
knows because it set it, and which no reply can influence.

`verified-by: bravebot_core::policy::a_fetch_cannot_be_redirected_to_a_host_nobody_approved`
`verified-by: bravebot_core::policy::a_denied_host_is_refused_at_the_egress_gate_so_a_redirect_cannot_reach_it`
`verified-by: bravebot_core::policy::a_web_fetch_rule_does_not_govern_this_programs_own_connection`
`verified-by: bravebot_core::policy::a_finished_fetch_stops_confining_the_turns_other_egress`
`verified-by: bravebot_agent::turn::a_denied_host_is_not_fetched_even_when_approved`

<a id="FETCH-5"></a>
### FETCH-5: what is not text is carried anyway, and a cap is reported

A body that is not valid UTF-8 is decoded lossily rather than refused. Responses are size-capped by
the egress layer, and a truncated one says so.

**Why.** Nothing here reads the body, so a decoding failure protects nobody: a page with one bad
byte is still the page that was asked for, and refusing it would be a fetch that failed for a reason
the planner cannot act on. That is the opposite of a file read, where
[READ-3](read-file.md#READ-3) reports a binary file as binary because the planner was going to be
shown the text.

`verified-by: bravebot_agent::turn::a_fetched_body_that_is_not_text_is_carried_anyway`
`verified-by: bravebot_net::lib::bodies_are_capped`
`verified-by: bravebot_net::lib::small_bodies_are_not_reported_as_truncated`
