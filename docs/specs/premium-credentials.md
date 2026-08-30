---
id: PREM
title: Leo Premium credentials
status: normative
governs:
  - crates/skus/src/device.rs
  - crates/skus/src/profile.rs
  - crates/skus/src/store.rs
  - crates/aichat/src/lib.rs
---

## Scope

Importing a Leo Premium subscription and spending its credentials. This is an auth flow: it
carries no workspace content and no model output, so no labelled value passes through it. What it
does carry is a user's subscription, which is why the rules below are about not spending or
leaking something that is not ours to spend.

```
bravebot import-leo-creds            # from Brave (stable)
bravebot import-leo-creds nightly    # or beta, or development
bravebot import-leo-creds --forget   # discard what was imported
```

## Clauses

<a id="PREM-1"></a>
### PREM-1: this registers as a new device and never borrows the browser's credentials

Only the subscription's **order id** is read from the browser profile. The credentials themselves
are generated here and signed by Brave, exactly as a second browser on another machine would. The
browser keeps its own, and nothing it holds is spent.

`verified-by: bravebot_skus::profile::the_browsers_own_credentials_are_not_read`
`verified-by: bravebot_skus::profile::an_order_id_that_is_not_a_uuid_is_refused_rather_than_used`
`verified-by: bravebot_skus::profile::uuids_are_recognised_and_near_misses_are_not`

<a id="PREM-2"></a>
### PREM-2: a credential is never sent to the non-premium host

The premium host and the credential travel together, because a credential belongs to a
deployment. A build with no premium host stays on the free tier rather than sending one where it
does not belong.

`verified-by: bravebot_aichat::client::a_subscribed_request_goes_to_the_premium_host_with_the_credential`
`verified-by: bravebot_aichat::client::without_a_premium_host_no_credential_is_attached`

<a id="PREM-3"></a>
### PREM-3: an order is checked before anything is registered against it

An unpaid order, one asking for no credentials, one asking for an implausible number, and one
without interval metadata are all refused. The Leo item is picked out of a multi-item order rather
than assumed to be the only one.

`verified-by: bravebot_skus::device::an_unpaid_order_cannot_be_registered_against`
`verified-by: bravebot_skus::device::an_order_asking_for_no_credentials_is_refused`
`verified-by: bravebot_skus::device::an_implausible_credential_count_is_refused`
`verified-by: bravebot_skus::device::an_order_without_interval_metadata_is_refused`
`verified-by: bravebot_skus::device::the_leo_item_is_picked_out_of_a_multi_item_order`
`verified-by: bravebot_skus::device::an_order_says_how_many_credentials_to_ask_for`

<a id="PREM-4"></a>
### PREM-4: a batch is verified against the issuer's key before it is stored

A batch signed by the wrong key does not verify, one without a proof is refused, and one with no
signed credentials is an error. Tokens are matched by value, never by position.

`verified-by: bravebot_skus::device::a_batch_signed_by_the_wrong_key_does_not_verify`
`verified-by: bravebot_skus::device::a_batch_without_a_proof_is_refused`
`verified-by: bravebot_skus::device::a_response_with_no_signed_credentials_is_an_error`
`verified-by: bravebot_skus::device::tokens_are_matched_by_value_not_by_position`
`verified-by: bravebot_skus::device::a_well_formed_batch_is_decoded`

<a id="PREM-5"></a>
### PREM-5: a credential is single-use and is never offered twice

Credentials arrive in batches covering a few days and are spent one per request. A spent one is
never offered again, consecutive spends hand out different credentials, and spending past the end
of a batch is refused. A moment outside every validity window yields no credential.

`verified-by: bravebot_skus::store::a_spent_credential_is_never_offered_again`
`verified-by: bravebot_skus::store::consecutive_spends_hand_out_different_credentials`
`verified-by: bravebot_skus::store::spending_past_the_end_of_the_batch_is_refused`
`verified-by: bravebot_skus::store::the_next_usable_credential_is_the_one_valid_at_that_moment`
`verified-by: bravebot_skus::store::a_moment_outside_every_window_yields_no_credential`
`verified-by: bravebot_skus::store::a_window_does_not_include_its_own_end`

<a id="PREM-6"></a>
### PREM-6: nothing is written back unless a credential was actually spent

A session that spends nothing never writes, and a detached batch has nowhere to write.

**Why.** The store lives in the system keychain, so a write can prompt for a password. Writing
when nothing changed would ask the user for nothing.

`verified-by: bravebot_skus::store::spending_does_not_write_until_asked_to`
`verified-by: bravebot_skus::store::a_session_that_spends_nothing_never_writes`
`verified-by: bravebot_skus::store::a_detached_batch_has_nowhere_to_write`

<a id="PREM-7"></a>
### PREM-7: credentials live in the system keychain, and each channel is stored separately

Not in a file. A malformed or empty entry is reported as such rather than treated as absent
credentials, and a credential without a token is rejected on load.

`verified-by: bravebot_skus::store::each_channel_is_stored_separately`
`verified-by: bravebot_skus::store::an_empty_entry_is_reported_as_absent_rather_than_malformed`
`verified-by: bravebot_skus::store::a_batch_that_is_not_json_is_reported_as_malformed`
`verified-by: bravebot_skus::store::a_credential_without_a_token_is_rejected_on_load`
`verified-by: bravebot_skus::store::an_entry_missing_its_order_is_reported_as_malformed`
`verified-by: bravebot_skus::store::a_batch_survives_a_round_trip_through_the_stored_form`

## Requirements and limits

- **macOS and Linux.** Windows is not supported.
- The build must know the premium host. Without it premium is unavailable (PREM-2).
- A credential only works against the deployment that issued it, so import from the Brave channel
  matching the environment the binary is configured for. Mismatching them returns 401.
- Sign in to Leo in that Brave install first: a subscription that is not in the profile cannot be
  imported.
- Importing and the first request of a session may ask for the keychain password (PREM-7).
- `bravebot doctor` reports how much is left.
