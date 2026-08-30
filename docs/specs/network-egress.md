---
id: NET
title: Network egress
status: normative
governs:
  - crates/net/src/lib.rs
---

## Scope

Every request this process makes to the network: what has to be true before one leaves, and what
comes back. What the returned bytes are labelled, and what may then be done with them, is
[labels.md](labels.md).

## Clauses

<a id="NET-1"></a>
### NET-1: there is one way out, and it is not optional

Every outbound request goes through a single call. The HTTP client is private to that module and
no other crate depends on it, so there is no second path that could skip the gate.

**Why.** This is the whole property, and it is structural rather than a matter of discipline. In
the design this replaces, using the hardened helper was optional and two of three fetchers
bypassed the redirect check.

`verified-by: bravebot_core::policy::network_egress_requires_the_fetch_capability`
`verified-by: bravebot_net::egress::a_successful_fetch_returns_a_labelled_body`
`verified-by: bravebot_net::egress::a_fetch_without_the_capability_is_refused`

<a id="NET-2"></a>
### NET-2: a redirect is revalidated on every hop

Redirects are followed by hand and each new URL is put to the gate before it is fetched. The chain
is bounded, so a loop ends rather than running forever.

**Why.** Following redirects automatically would mean the gate only ever saw the first URL, and a
permitted host could hand off to a denied one.

`verified-by: bravebot_net::egress::every_redirect_hop_is_revalidated`
`verified-by: bravebot_net::egress::a_redirect_loop_is_bounded`
`verified-by: bravebot_net::lib::redirect_status_codes_are_recognised`
`verified-by: bravebot_net::lib::absolute_redirects_are_used_as_given`
`verified-by: bravebot_net::lib::path_absolute_redirects_keep_the_authority`
`verified-by: bravebot_net::lib::relative_redirects_resolve_against_the_parent_path`
`verified-by: bravebot_net::lib::scheme_relative_redirects_keep_the_scheme`

<a id="NET-3"></a>
### NET-3: only http and https ever reach the network

Any other scheme is refused before a connection is attempted, rather than handed to a library to
interpret.

**Why.** A URL is routing, and a scheme this code does not understand is a destination nobody
decided on.

`verified-by: bravebot_net::lib::only_http_schemes_are_permitted`
`verified-by: bravebot_net::egress::non_http_schemes_never_reach_the_network`

<a id="NET-4"></a>
### NET-4: a body is capped, and a truncated one says so

Past the cap the body is cut, and the result reports that it was. A body that stops partway is a
failure rather than a short success, and a read that fails is not reported as a short body.

**Why.** Silence would let a caller treat half an answer as the whole one. This is resource
hygiene, not content inspection: the bytes are never parsed to decide anything.

`verified-by: bravebot_net::lib::bodies_are_capped`
`verified-by: bravebot_net::lib::small_bodies_are_not_reported_as_truncated`
`verified-by: bravebot_net::lib::a_failed_read_is_not_a_short_body`
`verified-by: bravebot_net::egress::a_body_that_stops_partway_is_a_failure_rather_than_a_short_body`

<a id="NET-5"></a>
### NET-5: each phase of a request is bounded separately

Connecting, starting to reply, and continuing to reply are timed apart. A reply that is still
arriving is not cut off for having taken a while to start, and one that stops arriving is given
up on rather than waited for indefinitely.

**Why.** One timeout over the whole call cannot tell a slow answer from a dead connection, and
choosing a single number makes one of those two cases wrong.

`verified-by: bravebot_net::egress::a_reply_still_arriving_is_not_cut_off_for_taking_longer_than_it_took_to_start`
`verified-by: bravebot_net::egress::a_reply_that_never_comes_gives_up`
`verified-by: bravebot_net::egress::a_reply_that_stops_arriving_is_given_up_on`

<a id="NET-6"></a>
### NET-6: a failure that means "not now" may be retried, and nothing else may

A connection that gave out is worth another attempt. A refusal is not, and only the statuses that
mean the server is temporarily unable are treated as retryable.

**Why.** Retrying a refusal turns one denied request into several.

`verified-by: bravebot_net::lib::a_connection_that_gave_out_is_worth_another_attempt_and_a_refusal_is_not`
`verified-by: bravebot_net::lib::only_the_statuses_that_mean_not_now_are_worth_another_attempt`
`verified-by: bravebot_net::egress::a_non_success_status_is_an_error`

## Known costs

- **`bravebot-net` is not the only crate that opens a socket.** `bravebot-skus` builds its own
  HTTP client for the subscription service. That traffic carries credentials and an order id,
  never workspace content or model output, so no labelled value escapes the gate. NET-1 is about
  everything carrying labelled content. A second egress that ever carried content would be a
  violation.
