# Credentials: the design

What Brave builds, in what order, and what the user has to do. The normative spec that states what
must be true follows separately.

**We adopt [Brave Vault](https://github.com/brave-experiments/brave-vault)'s model and its grant
machinery, harden it, and make bravebot one of its agents. We do not build a second broker.**

Brave Vault's *design* is our design, and its plan already states our principles: the agent never
holds the vault secret, capabilities rather than credentials, short-lived and single-use, least
privilege per item and action, prefer never-reveal where the broker can perform the action, and
every grant auditable and revocable. It is Rust and runs on all three platforms.

## What credentials this design covers

Two conditions have to hold: **the broker can hold the credential**, and **it can either perform
the action itself or intercept the request on the way out**. Everything in or out of scope follows
from those.

**In scope.** OAuth tokens and API keys the broker obtains, short-lived tokens it mints or fetches,
and bearer tokens carried in HTTP headers. Concretely: email, chat,
issue trackers, most SaaS APIs, and after phase 4, `git push` over HTTPS and arbitrary HTTP.

## What Brave Vault already gives us

| our requirement | what exists |
| --- | --- |
| an action performed so the agent never sees the secret | `use` fills a login through the native-messaging host and extension. The pattern is proven; that one action is all there is |
| a grant is a capability, not a mood | a handle grants nothing on its own and only lets the agent ask; the grant it exchanges for names one item and one action |
| short-lived, least privilege | per item and per action. Ad-hoc grants default to 60 seconds and one use; pre-created grants take a user-configurable `max_uses` and TTL, which is the standing-grant row below rather than this one |
| every grant auditable and revocable | logged, and revocable per agent or per grant from the Agents panel |
| approval outside the agent's own process | the app draws the prompt in its own window. Out of process, and **not** out of account: the helper drawing it shares a uid with the agent, so a same-uid attacker forges the answer |
| standing grants, deliberately given | pre-authorisation in the Agents panel |
| honesty about the residual risk | the README states the same-user caveat and names the same fix we did, a separate operating-system user |

## What we do not want from Brave Vault

Three of the vault's actions hand credential bytes to the caller. bravebot uses none of them.

| we decline | why |
| --- | --- |
| `reveal`, which copies the password to the clipboard by default and prints it to stdout with `--print` | the README flags the clipboard as readable by other local applications, so the default exposes the secret to every process on the machine, and `--print` puts it in the caller's output |
| `grab-session`, which prints live session cookies | those are a bearer token for the origins they cover, until they expire. bravebot drives no browser |
| `use --via clipboard`, the opt-in alternative to `--via inject` | the same clipboard exposure as `reveal`, reached through an action we otherwise want. bravebot's ceiling permits `perform` and `placeholder`, and no variant of `use` |

Excluding all three at enrolment costs nothing in scope and buys the central property outright: not
"no call returns credential bytes, except these", but **no call returns credential bytes**.

Interactive password prompts are not a fourth path. `git push` over HTTPS and HTTP authenticate by
substitution in phase 4, and everything else in scope is a delegated `perform`. There is no action
that writes a secret to a child's stdin, stdout, or an askpass helper. A child bravebot started
runs under the same uid and holds whatever it is given, so handing it plaintext is rung 3 wearing a
different hat, not rung 1.

## What is missing in Brave Vault

Its *implementation* is a prototype. The vault file
is encrypted with `const VAULT_PASSWORD: &str = "testing"`, so encryption at rest is decorative
today. It also reads `BRAVE_SERVICES_KEY` from an `.envrc` in the person's home, and `.envrc` is
direnv, so the key is exported into the environment of every shell entering that directory and
everything those shells spawn. A broker that stores secrets carefully while holding its own
service key in an inherited environment variable has a rung-4 credential underneath a rung-1
design.
The action paths do not carry over either: filling a login form through a browser extension shares
nothing with sending mail or signing a request beyond the word "delegated".

| gap | why it matters for bravebot |
| --- | --- |
| item type: service token | the vault holds site logins and sessions. Email, GitHub and cloud need OAuth tokens and API keys |
| hardening before it holds anything real | a key-derivation password held in the platform store instead of the `testing` constant, and `BRAVE_SERVICES_KEY` out of the environment and into that store |
| action: `perform` beyond form fill | sending mail, signing a request, calling an API. Their plan names signing as intended, nothing implements it, and form-fill code does not generalise |
| destination scoping for calls that are not browser navigations | today's control is the browser requesting host permissions when cookies are grabbed, which bounds which origin a session comes from. Nothing constrains which endpoint a service token may reach |
| placeholders and egress substitution | `git push` over HTTPS and arbitrary HTTP cannot be delegated whole, so something must substitute at the boundary |
| caps | rate, volume and distinct destinations. Grants expire but are not throttled |
| separate operating-system user | the README names this as the gap itself. It is the largest single improvement available, because it is what makes the store unreadable by anything running as the person |
| headless operation | the approval channel is a window and the never-reveal path runs through a browser extension. Over SSH there is neither. Out of scope rather than scheduled: a standing grant is redeemable only while a verified helper session for that uid is connected, so a box with no helper has no credential path |
| Windows peer verification | the vault ships an NSIS installer, so the platform is supported, but nothing verifies either end of the channel there. Phase 1 requires Authenticode in both directions |
| a trail the agent cannot write | grants are logged; the log is not yet a policy input with integrity requirements |

## Architecture

The broker cannot draw its own window. A macOS `LaunchDaemon` runs outside any GUI session,
Windows session 0 isolation forbids a service drawing UI at all, and a Linux system service has no
display connection. So the app splits in two.

```
┌─ the person ─────────────────────────────────────────────────────────┐
│  sees a prompt and answers it                                        │
│  no out-of-account confirmation exists on any platform               │
└────▲─────────────────────────────────────────────────────────────────┘
     │ the prompt is drawn and answered inside the account below,
     │ so a same-uid attacker can forge the answer
┌────┴─ agent domain: the person's uid ────────────────────────────────┐
│  bravebot        planner → gates → tools → spawned programs          │
│                  holds a handle, grant ids, placeholders             │
│  UI helper       Agents panel, prompt windows                        │
│                  same uid as bravebot, so not a second domain        │
│  spawned child   proxy configured to the egress socket, sends        │
│                  requests that still carry a placeholder             │
└────┬─────────────────────────────────────────────────────────────────┘
     │ two channels, in a place the person cannot claim
     │   control  Mach in the system domain on macOS; a socket in a
     │            root-owned directory on Linux and Windows
     │   egress   a socket in a root-owned directory on all three,
     │            including macOS: Mach does not carry HTTP CONNECT
     │   each end verifies the other's account; neither is a read path
┌────▼─ broker domain: uid _bravevault ────────────────────────────────┐
│  bravevault-brokerd, no UI, no session                               │
│    store     every credential at rest, unlocked without a session    │
│    grants    per item, action, destination, expiry, caps             │
│    perform   acts whole and returns an outcome, never bytes          │
│    egress    substitutes at the approved position, re-signs          │
│    trail     every grant and every use, append-only                  │
└────┬─────────────────────────────────────────────────────────────────┘
     ▼ mail server, GitHub, cloud API
```

**The UI helper lives in the person's account, and that costs us a claim.** A same-uid attacker can
inject into the helper or synthesise the click, so an approval drawn by the helper is forgeable by
exactly the adversary this design is built against. "Approval on a channel the agent cannot draw"
is therefore **out of process but not out of account**.

**There is no local out-of-account confirmation, on any of the three platforms.** Having the
broker invoke a system-drawn prompt does not escape the split, because those prompts need the very
session a daemon does not have. `UserConsentVerifier` is a WinRT API needing a foreground app
context and
does not work from a session 0 service. On macOS the Touch ID prompt is drawn in the console user's
session, so a `LaunchDaemon` calling `LAContext` is in the same position as one calling `NSWindow`.
Authorization Services through `SecurityAgent` is the closest thing that historically worked from a
daemon, and it is the narrowest of the options.

What is actually achievable is a session-resident helper that triggers a prompt and reports the
result to the broker. There is no attestation on that report, so a same-uid attacker forges the
report rather than the click. The forgery moves one hop and does not go away.

**So the design states the limit rather than dressing it.** The credential boundary holds: no
approval, forged or genuine, causes the broker to hand over bytes, and `perform` is unaffected. The
approval boundary does not hold. On every platform, an attacker running as the person can obtain
any grant the enrolled binary's ceiling permits.

Three things follow, and they shape everything downstream.

- **Scope carries the weight that approval cannot.** What bounds a compromised agent is the item,
  the action, the destination, the expiry and the caps, not the prompt in front of them. Caps stop
  being a nicety.
- **No action is treated as unforgeable.** We do not maintain a category of actions requiring
  stronger confirmation, because no local mechanism delivers one, and a category we cannot enforce
  is a claim rather than a control.
- **A second device is the only real answer**, and it is out of scope here. When it is built, it
  is the first thing that makes an approval mean something against this adversary.

## The store

Brave Vault owns this and we do not add a second one. Its own storage, moved into the
`_bravevault` account, is the answer on all three platforms.

**Where the store choice is actually interesting is unlock, and that is the phase 1 risk.**
`_bravevault` has no login session, so the login keychain on macOS and gnome-keyring on Linux are
both unavailable: neither unlocks without one. The workable mechanisms are narrower than "the
platform store", they differ per platform, and they need a spike before phase 1 rather than a line
in a table.

| platform | mechanism | property |
| --- | --- | --- |
| macOS | an item in the System Keychain, `/Library/Keychains/System.keychain` | readable by the daemon, not by the person's login keychain tooling. This is the older file-based keychain rather than the data protection keychain, and it is the one a session-less daemon can actually use |
| Linux | systemd `LoadCredential` or `SetCredentialEncrypted`, sealed to the TPM where one exists | injected at start into `$CREDENTIALS_DIRECTORY`, readable by the service for its lifetime and not by the person. `SetCredentialEncrypted` binds decryption to the machine. Children the broker spawns inherit neither that directory nor the key |
| Windows | DPAPI at machine scope, under the service account | the equivalent constraint and the equivalent answer |

**Our own encryption stops sooner than it looks.** A key that unlocks without a session is a key
nobody types, so it is machine-held and reachable by root. Encryption at rest therefore covers a
stolen disk, and nothing else. **After phase 1 the account boundary is still the control**, exactly
as it is before. That is the right trade for a daemon that must start unattended, and it is a
smaller claim than the `testing` constant invites.

**Phase 1 also gives a property up, and it should be said rather than discovered.** Today's broker
lives inside the unlocked desktop app and goes away when the vault locks, so while it is locked
nothing can ask at all. A daemon holding a machine-unlocked key is reachable whenever the machine
is on. That is the price of a daemon that starts without anyone logging in, and the account
boundary is what stands in its place.

The obvious alternative is a trap. **A keychain used from the person's own account protects
nothing here.** Claude Code CLI creates its
keychain item with `security add-generic-password` and no access-control arguments, so the item
trusts `/usr/bin/security`, which any process can invoke, and one command returns its access and
refresh tokens with no prompt. Binding the item to a binary instead does not help.
`SecTrustedApplicationCreateFromPath` has been deprecated since macOS 10.15, and the modern
mechanism, access groups on the data protection keychain, needs a code-signing entitlement and a
login session, which a daemon does not have.
**The account boundary is the control, not the store.** A keychain item belonging to another user is
unreadable by tooling run as the person, with no access list and no deprecated API involved.

For fleets, the vault gains a backend for HashiCorp Vault or the cloud provider's secret manager,
because dynamic secrets with leases are genuinely a rung we cannot otherwise reach. That is not the
default and it is not phase one.

## Peer verification

Verification is defence in depth. The boundary is that no call available to bravebot returns a
secret.

**macOS.** A Mach service in the system domain, registered by a `LaunchDaemon`. Connections carry
an audit token, resolved to a code signature with `SecCodeCopyGuestWithAttributes`, and the broker
requires our team identifier rather than merely a valid signature. The reverse direction is the
bootstrap namespace itself: a user process cannot register a name in the system domain, so the
client does not need a peer check to know it reached the real service. 1Password CLI does the
forward half the same way, verifying the connecting executable's signature and naming that process
in its prompt.

**Linux.** There is no code signature to check, so this is a path and ownership check and should
be called one. `SO_PEERCRED` for uid and pid, then a `pidfd` pinned on that pid before reading
`/proc/<pid>/exe`, so the binary cannot be swapped between check and use, and the resolved binary must be root-owned and not writable by the caller.

That rules out a `$HOME/.local/bin` install, which our own installer offers, so the rule is:
**credential capability on Linux requires a system-path install.** A home-directory install runs
bravebot normally and refuses every credential-backed capability, saying why. Narrowing the check
instead would leave peer verification comparing a binary the caller can rewrite. A setgid helper
would also satisfy the check and is rejected for the same reason: it authenticates a group, not a
binary the caller cannot modify.

**Windows.** A named pipe created with `FILE_FLAG_FIRST_PIPE_INSTANCE` and a restrictive
descriptor, `GetNamedPipeClientProcessId` for the caller, then an Authenticode check on that
process's image pinned to our publisher. Both directions are required, and neither exists in the
vault today.

**Verification runs both ways.** bravebot authenticates the broker too, or a process running as the person binds the socket path first, or replaces
it, and becomes the broker as far as the agent is concerned. It still cannot obtain a credential,
so the store boundary survives, but it harvests every `perform` argument, returns forged outcomes
so bravebot reports a mail sent that never was, and hands out its own placeholders together with a
proxy configuration the child then trusts, which puts a certificate authority in the attacker's
hands. The refuse-when-absent rule sharpens this: the moment the real broker is down is exactly
when a fake one is most useful.

The fix is the same asymmetry read backwards, and it is small on all three platforms because the
broker runs under a different account.

- **The socket cannot be squatted.** It lives in a directory owned by root and not writable by the
  person, so the path cannot be replaced. On macOS the broker registers a Mach service in the
  system domain, which a user process cannot claim at all.
- **The client checks the server.** On Linux, `SO_PEERCRED` on the connected socket: bravebot
  refuses any peer whose uid is not `_bravevault`, and a same-uid attacker cannot present that uid.
  On Windows, `GetNamedPipeServerProcessId` and an Authenticode check pinned to our publisher. On
  macOS the system Mach namespace already answers it, as above.

A hostile program running as the person can still present itself and ask. It cannot obtain a
credential. Asking is the residual attack, and what bounds it is the grant's scope, its expiry and
its caps. Not the prompt: the prompt is forgeable by the same attacker.

**The helper is a second verified client, not a role any process of the person can claim.** Both
client binaries, bravebot and the helper, are pinned at install, not enrolled by a prompt. The
broker accepts prompt replies and PKCE relays only from the helper binary, checked the same way as
bravebot: team identifier on macOS, Authenticode publisher on Windows, root-owned system path on
Linux. A same-uid process that is not that binary cannot open a helper session. The helper's
ceiling is drawing a prompt and relaying a code; it cannot `perform` or `placeholder`. A helper
session is per caller uid, matching the grant namespace, so a session for one person does not
redeem another's grants. Injecting the live helper is the same forgery as synthesising a click, and
is already counted in the residual: it requires the helper to be running.

## The agent-facing API

Brave Vault's existing shape, with two additions and one restriction.

```
enroll  --label bravebot                                 # once, confirmed in the app
request --item <id> --action <perform|placeholder> --to <destination> [--position <where>]
perform --grant <id> --action send_mail --args <json>    # new
placeholder --grant <id>                                 # new
```

**bravebot's ceiling is `perform` and `placeholder`. It can never request `reveal`,
`grab-session`, or either variant of `use`.**
There is deliberately no flag for this. An `--actions` argument would invite the reading that the
caller chooses its own ceiling, which is the thing the next paragraph exists to prevent.

**The ceiling binds to the verified binary, not to the label the caller supplies.** Enrolment is
initiated by the agent and confirmed by a prompt, so a compromised bravebot could otherwise enrol a
second agent under a plausible label asking for `reveal`, `grab-session`, or `use`, and the person would
see a prompt resembling the one they already approved. Instead the broker resolves the enrolling
binary through peer verification and applies that binary's ceiling to whatever it asks for. Any
enrolment from the bravebot binary gets bravebot's ceiling, and re-enrolling widens nothing. Three of the vault's actions return credential bytes, and `use --via inject` is excluded for a
different reason: it fills a login form through the browser extension, and bravebot drives no
browser. None of them is needed by anything
bravebot does, and excluding them at enrolment is what makes "no call returns a credential" true
of our agent rather than aspirational.

**`request` carries the destination, for the same reason it carries the position.** A recipient,
a host, an endpoint: whatever decides where the effect lands is named before approval, so the
person approves it and the broker can refuse a `perform` that names a different one. A destination
chosen after approval is a destination the agent chose.

`perform` and `placeholder` return no bytes. Adding a third call that does, or widening bravebot's
enrolment, discards the design. The tests below are what carry that rule.

## Grants

Brave Vault's grants gain two properties.

**Lifetimes** are `once`, `task`, `session` and `never`, the same four bravebot's prompting rules
use. `never` means the question is put every time rather than cached. `session` is a span of time and
has nothing to do with `grab-session`. `task` is the addition, running to the end of the turn
series a person's typed prompt started.

**Two deliberate divergences from those rules, named rather than glossed.** They allow no second
standing grant over actions, on the grounds that the trust map is already the standing one and it
covers paths. Credential pre-authorisation in the Agents panel is a standing grant over actions,
and calling that alignment would be untrue: it is deliberate, visible, and the only place a life
longer than `session` is permitted. The second is caps. Those rules re-ask without revoking; the
broker hard-refuses. Stronger, and enforced where a prompt cannot be forged.

**Caps**, enforced in the broker: rate, volume in bytes, and distinct destinations. Three rules
make them well defined.

Counted when the action is requested, before it runs, so concurrent calls cannot pass a cap
together. A failed action still counts, because refunding failures makes a cap something an
attacker stays under by failing. And a cap attaches to a **capability**, meaning one
agent, one item and one action, rather than to a single grant, so a `once` grant is not exempt: ten
single-use grants for the same capability in a minute trip a rate cap of five. That is the only
reading under which caps mean anything. Caps do not aggregate across agents, because nothing here
reasons about a sum.

**What is a control and what is a courtesy.** Once approvals are forgeable by a same-uid attacker,
the two have to be separated and labelled, or the document reads as though the release rules
survived a collapse that took everything else.

| controls, enforced by the broker whatever the answer was | courtesies, which help an honest person and stop no attacker |
| --- | --- |
| item, action and destination scoping | per-request rather than per-application approval |
| expiry, and no renewal by use | the content of a prompt |
| caps on rate, bytes and distinct destinations | a per-call lowering of a mail body |
| the ceiling bound to the verified binary, so no read path is reachable | the choice of lifetime offered |
| a verified helper session, for that uid, to redeem a standing grant | a locked screen, which does not drop the session |

**Approval is still per request, not per application.** Against a same-uid attacker the two are
identical, because that attacker forges either answer. The
reason to keep it is the other two cases: a person who over-grants by accident, and software that
is buggy rather than hostile. It is a courtesy, and worth having as one.

## How a credential arrives

A service-token item type says what the broker holds and not how it got there, and the difference
matters because the acquisition path is the one moment a secret could pass through bravebot.

**The default is an authorisation-code flow with PKCE, run by the UI helper.** The loopback
redirect lands on the helper, in the person's session, which is where the browser already is. The
broker never listens for HTTP: a daemon accepting an authorisation code on loopback is a port any
local process can post to, and PKCE narrows that without removing it.

The split that makes this safe is where the verifier lives. **The broker generates the PKCE
verifier and never releases it**, the helper relays only the returned code over the control socket
it already uses, and the broker performs the exchange. A code intercepted in the person's account
is worth nothing without the verifier. bravebot is on none of these legs.

**Pasting a token in is a lowering**, asked for separately, said to be weaker, and recorded as
such. It is the path for services offering no flow at all, and it is never the default, because a
pasted token arrives through whatever typed it.

**Where a service can mint a bounded credential, that is the one we ask for.** An installation
token for one repository rather than a personal access token, an assumed role with a session
policy rather than a static key. Holding the broader form instead is the same recorded lowering.

## The two paths

**Delegated.** The broker performs the action whole and reports the outcome. Sending mail, signing
a request, calling an API. No placeholder, no proxy, no certificate work. This is the strongest
rung at a fraction of the cost and it covers most of what new capabilities need.

**Substituted.** For actions only the agent can perform: `git push`, arbitrary HTTP, a tool that
authenticates for itself. The agent gets `bvb-<32 hex>`, unique per session and credential.

**The position is part of the request, so the person approves it.** A grant names where the
placeholder may appear, as a specific header or field, and a request carrying it anywhere else
fails rather than being substituted. The position is chosen at `request` and not at `placeholder`,
because a position picked after approval is a position the agent picked. Substituting anywhere in
headers and body would make the placeholder a token the agent can aim at any part of a request to
an allowed destination, which turns an approved call into a way of writing the credential into a
body of its choosing.

The broker's egress socket sits in the child's proxy configuration. What follows governs what it
does there.

**A destination is the origin of the request being forwarded**, and the fields that are present
have to agree on it: the `Host` or `:authority` of the inner request, always; the `CONNECT`
target, when the request is tunneled; the TLS server name, when the connection is TLS. Each
present field is matched against the grant, and a mismatch between any two fails the request.
Cleartext HTTP has no `CONNECT` and no server name, and is matched on `Host` alone. Checking only
the `CONNECT` target leaves the inner `Host` free, which is server-side request forgery carrying a
real credential; checking only `Host` on a tunneled TLS request lets the connection go somewhere
else entirely. Matching happens before any name resolution, and the resolved address is then checked so a public name cannot be
pointed at internal infrastructure. That set is default-deny and explicitly includes loopback, the
RFC 1918 ranges, carrier-grade NAT, IPv6 unique-local and link-local, and `169.254.0.0/16`, which
is where the cloud metadata endpoint lives and is the single most valuable target on the list. An
internal destination is reachable only when the grant named that exact host. A grant for one host never fires for a
literal address, and matching is on the exact name rather than a suffix. Rebinding between the
check and the connection is closed by resolving once and connecting to that address.

**A redirect is a new destination and is checked as one.** The proxy does not follow redirects on
the client's behalf. It returns them, so the client's next request arrives as its own request and
is matched against the grant like any other. Where a hop is ever followed internally, the
destination is revalidated first, which is the rule bravebot's own egress already applies to every
hop. Without it, one `302` after substitution sends a real credential somewhere the grant never
named.

**Proxy configuration is advice, so it cannot be the boundary.** A child that ignores
`HTTP_PROXY` reaches the network directly, and with a placeholder in hand that is a failed
authentication rather than a leak, which is the property to test. Making it a boundary means a
network namespace or a syscall filter, and that is the confinement work rather than this.

**Re-signing is per scheme and fails closed.** For AWS the grant carries the access key and secret
together, with the session token where one exists. An unsigned payload, a missing date, or a
scheme the boundary does not implement is refused rather than forwarded with a signature that will
not verify.

**Substitution means reading requests, so the boundary terminates TLS** with a local certificate
authority installed in the trust store the child actually consults, which differs per runtime, and
never in the person's login keychain. Two failures follow and they are reported distinctly: a
runtime whose trust store we cannot configure, and a server that pins its certificate. Pinning is
a property of a connection rather than of a configuration, so neither can be announced at startup,
and what we promise is a connection-time failure naming which of the two it was.

## Email

**Send is delegated**, as a new `perform` action. The recipient decides where the effect lands, so
it is routing and must be releasable and trusted. Mail the agent read can therefore never name the
recipient of mail it sends, which closes reply-to-injection with no rule of its own.

Two things about where that check lives. Only the trusted half fires today, since the releasable
half passes by construction until confidentiality propagates through the planner. And it is
bravebot's gate that knows the provenance: the broker only checks the envelope against the grant,
so a compromised bravebot that skips its own gate is bounded by the grant's recipient rather than
by provenance.

**The envelope is the grant, not just the `To:` field.** `Cc`, `Bcc` and `Reply-To` decide where
the effect lands exactly as `To:` does, so the broker requires every envelope address to equal the
granted one or be absent. `From:` is the mailbox the grant is for, or absent: a different identity
is a different principal. Unknown fields in `perform` args are refused. Without that, one approved
recipient is a channel to anyone.

**The recipient is bounded and the body is not**, which is the part that is easy to miss. One
address says nothing about what may be written to it, so an approved `send_mail` is a channel of
unlimited capacity. The body is content carried to an effect and takes the release rule, but that
half is a courtesy twice over: a lowering is forgeable like every other approval, and the planner's
output is labelled releasable today whatever its context held, so the check passes by
construction until that changes.

**The byte cap is the control.** 64 KiB per capability per task, counted in bytes rather than
messages, measured on the encoded bytes as they go on the wire so transfer encoding counts at its
transferred size, and refused at the broker whatever anyone approved. That number is a security
parameter and changing it is a change to this design. Attachments are out of scope for `send_mail`
in phase 3, which is what makes it defensible: one screenshot exceeds 64 KiB and ordinary prose
does not. If attachments are ever wanted they arrive with their own cap and their own argument.

**Read is scoped at the provider.** The grant covers one label or folder that the person's own
server-side filter populates, so a passcode or a reset link is never in scope. We never inspect a
message to decide what it is. Incoming mail is untrusted content and quarantines like anything
else from outside.

**Where a provider cannot filter server side, we do not offer read for it.** A dedicated address
is the supported alternative, and the setup command says so rather than degrading quietly.

## Sending one email, end to end

The worked example, in the terms above. Everything here is phase 3, and it needs phases 1 and 2
first.

**Setup, once.** The UI helper begins an authorisation-code flow with PKCE. The broker generates
the verifier and keeps it, handing out only the challenge. The helper opens the browser and
listens on loopback, because it is the component in the person's session and the broker never
accepts HTTP. The person consents at the provider, scoped to sending and not reading. The
redirect returns a code to the helper, which relays it over the control socket, and the broker
exchanges code and verifier for the tokens, which land in its store under `_bravevault`. bravebot
is on none of these legs, and a code intercepted in the person's account is worthless without the
verifier.

**Per message.**

1. The planner proposes `send_mail`. It can see labels and the tool's rule so it can plan around
   them; nothing it says decides anything.
2. bravebot's gates run before the broker is involved. The capability must be granted. The
   recipient is routing, so it must be releasable and trusted, which is what stops mail the agent
   read from naming the recipient of mail it sends. The body is content carried to an effect and
   takes the release rule.
3. Unless a grant is in force, the action lands on `asked`. `request --item mail --action perform
   --to <recipient>` either matches a pre-authorised grant or raises a prompt through the helper,
   never through the conversation. The person sees the recipient, the subject, the body, the item's
   label, and the lifetimes on offer. Never the credential itself: a helper that rendered bytes
   would be a read path.
4. The answer mints a grant: bravebot's handle, the mail item, `send_mail`, that recipient, a
   lifetime, caps.
5. `perform --grant <id> --action send_mail --args {...}`. Both ends verify: the broker checks
   bravebot's signature or binary path; bravebot checks the peer is `_bravevault` on Linux, the
   publisher on Windows, and the system Mach namespace on macOS.
6. The broker checks the grant is unexpired and unspent, a verified helper session for this uid
   is connected if the grant is standing, every envelope address matches the one approved or is
   absent, unknown args fields are absent, the encoded body is inside the byte cap for this
   capability and task, and the rate and spread caps hold. All counted at request, before anything
   runs.
7. The broker reads the token from its store, refreshes it if needed, and calls the provider
   itself.
8. It returns an outcome, sent or not sent with a message id, and never bytes. It appends to the
   trail, which bravebot cannot write.

**What bravebot never held**: the access token, the refresh token, the PKCE verifier, or any call
that would return them.

**Where it fails on purpose.** Broker down: refuse, never degrade. Recipient drawn from untrusted
content: refused before the broker is called. Body over the cap: refused even with an approval.
A compromised bravebot can forge the approval and gets one grant, for one recipient, bounded by
caps and expiry, and still cannot obtain the token. With no grant and no helper connection the
send refuses. A standing grant with no helper connection refuses at the broker, not in bravebot.

## Build order

The ordering rule: **nothing holds a real credential until the account boundary and the encryption
are real.**

| phase | lands | buys |
| --- | --- | --- |
| 1 | vault hardening: key derivation from a real password held in a store that unlocks without a session, `BRAVE_SERVICES_KEY` out of the environment and into that store, scrubbed from every child the broker spawns, and no release-build path that reads a secret from the environment at all; broker moved to `_bravevault`; UI helper split into the person's session; peer verification on three platforms | a broker that can hold a secret at all. Nothing above is optional and nothing below is safe without it |
| 2 | bravebot enrols with the restricted action set, bound to the verified binary; `Grant` type; rung reported everywhere | a working handle, whose ceiling is `perform` and `placeholder` and nothing else |
| 3 | service-token item type; `perform`; `send_mail`; PKCE with the verifier on the broker; envelope equality; a verified helper session required to redeem a standing grant; **and its bounds in the same phase**: lifetimes, caps in bytes and requests, trail integrity | email send at the strongest rung, bounded on the day it ships |
| 4 | `placeholder` and egress substitution, local certificate authority, position binding, SigV4 re-signing | `git push` over HTTPS and arbitrary HTTP without holding tokens |
| 5 | HashiCorp Vault and cloud secret manager backends | a store for interactive machines, and real dynamic secrets. Not unattended operation |
| 6 | provider-side scoped mailbox read | inbox access that is not account recovery |

Phase 1 has no user-visible feature and is not negotiable. Phase 3 is the first thing anybody
notices, and it carries its own bounds because a channel of unlimited capacity cannot ship a phase
before the thing that limits it. Phase 4 holds most of the cost. Reading email
is last on purpose.

## Decisions

**Promote Vault out of `brave-experiments` and staff it, before phase 1.** Both sides are ours:
bravebot and the vault. Phase 1 is work on that code, and today it is still an experiment: no
releases, no tags, a hardcoded vault password. The decision is whether this is a product we will
maintain, not whether to take a dependency on someone else. Phase 1 does not start until that
is yes.

**When the broker is not running, refuse.** Loudly, naming the vault. Degrading to bravebot holding
a secret would defeat the design at the moment nobody is watching.

**No same-user mode.** On Linux a same-uid process can attach a debugger to the broker and read its
memory. The installer takes administrator rights once; if the person declines, bravebot runs with
no credential capability rather than holding secrets itself.

**One service account, `_bravevault`, with per-caller namespaces** keyed by the verified peer uid.
The same isolation as an account per person, with a simpler installer.

**Send before read.** Read is phase 6 and also waits for server-side filtering proven at two
providers.
Sending wrongly costs one message. Reading wrongly hands over every account that mails the address.

**No permission mode answers a credential approval, `--dangerously-skip-permissions` included.**
bravebot has a bypass mode that answers every permission question, including the ones deciding
trust, and a delegate inherits it. A credential grant is not on that list and cannot be added to
it. The mode's justification is that the blast radius is bounded by something other than the
prompts, meaning a container with no network and nothing in it worth losing. A credential grant's
blast radius is outside the container: a mail account, a repository, a cloud tenant. The premise
does not hold, so the exemption does. This sits on the same footing as the two prompts no rule can
answer today, and it needs saying, because otherwise one flag mints credential grants with nobody
watching.

**A standing grant is redeemable only while a verified helper session for that uid is connected.**
This is the control that makes a box with no helper have no credential path, and it has to be a
broker rule rather than a sentence about bravebot's behaviour. bravebot already refuses effects and
declines questions when nobody can be asked, but that binds the honest client. Grants live in an
always-on daemon, so a standing grant is otherwise redeemable by any same-uid `perform`, including
a one-shot nobody is watching.

The helper connection is not a flag bravebot sets. It is an authenticated session from the helper
binary, checked as in Peer verification, scoped to that uid. A same-uid process that is not the
helper cannot open one.

A locked screen is not a disconnected helper. Logout and helper exit drop the session; lock does
not. Standing grants stay redeemable on a logged-in machine whose owner has walked away. That is
the residual of a session, named rather than implied away.

What follows: with no standing grant and no helper, every credential-backed capability refuses.
With a standing grant, it is redeemable while that uid's helper is connected, and not otherwise.
A `session` grant dies when the helper session ends. `once` and `task` grants belong to the run
that minted them and are not waiting in the daemon for the next one-shot.

Provisioned pre-authorisation, a standing grant that works with no session at all, is deliberately
not built. It serves a case nobody has asked for, on the configuration with the weakest peer
verification of the three platforms. If it is ever wanted it arrives as its own design, with the
signed-configuration format and provisioning path that implies.

**HashiCorp Vault and cloud backends are a store for interactive machines**, not a way to revive
unattended operation. Phase 5 does not undo the helper-session rule.

## What this does not cover

Each row fails one of the two conditions in **What credentials this design covers**: the broker cannot hold the
credential, or there is nowhere to perform or intercept the action.

| not covered | why |
| --- | --- |
| ambient authority: a logged-in `gh`, `aws sso`, `docker.sock`, a metadata endpoint | there is no credential to hand the broker. It never sees anything, and only confinement helps |
| credentials already on disk: `~/.aws`, `~/.ssh`, tool caches | the broker did not obtain them. Migrating one in helps only if the original copy can be deleted, and an SSO cache regenerates itself |
| key material such as an SSH key | the secret never travels, so there is nothing to substitute. Covering it means the broker becomes a signing oracle, which is `ssh-agent`, and this design does not build one |
| passcodes, magic links, reset links | they arrive through the capability somebody granted, so the bound is provider-side scoping, which is phase 6 |
| clients that pin certificates | no interception point. This fails at connection time and says why |
| non-HTTP protocols: SMTP and IMAP directly, SSH, database wire protocols | the egress boundary is HTTP-shaped. Either the broker performs the whole action or there is no path |
| a tool reading its own configuration file | `terraform` reading `~/.aws/credentials` bypasses both paths. Masking listed config files at the boundary is a later intercept path, not phase 4 |
| tokens held by a connected protocol server | that server holds its own downstream credentials. Nothing here sees or bounds them |
| pushes, biometrics, hardware-key touches | not bytes, so nothing can hold them |

**The honest summary: this makes new capabilities safe and retrofits nothing.** Email and GitHub
work because the broker obtains those tokens itself. A developer's existing AWS profile and SSH
keys do not, and those are what an agent on a laptop reaches for first. Bounding those is the
confinement problem, and it is a different piece of work.

## What this does not fix

**A compromised agent can still ask, and can forge the approval.** The broker will not leak a
token, and it will send a mail if a grant covers it. No local mechanism makes an approval
unforgeable by a same-uid attacker, so what bounds this is the grant's scope, expiry and caps, not
the prompt. Describe it as a bounded blast radius and a legible trail, never as secrecy, and never
imply the prompt is doing work it cannot do.

**A grabbed session is a bearer token.** Brave Vault says this plainly about `grab-session`, and it
stays true: origin scoping controls which session the agent gets, not what it does with it.
bravebot's enrolment excludes the action, so this bounds the vault rather than us, and the
boundary is per agent rather than per machine: another of the person's agents may hold that
ceiling, and a session it grabbed is a credential in the same store on the same machine.

## Tests, and the phase each one gates

Each test gates the phase that introduces the thing it checks, so no phase is held to a property
that does not exist yet.

**Phase 1, the hardening**

1. The vault file cannot be opened with a constant, and the key derives from a machine-held key in
   the platform's session-less store: System Keychain, TPM-sealed systemd credential, or
   machine-scope DPAPI.
2. A process running as the person cannot read the broker's store, attempted through the
   platform's own tooling. This is the test the account move exists for, and it is why the move
   comes first.
3. A caller is rejected when its signature is absent, self-signed, or from another team or
   publisher on macOS and Windows, and when its binary is outside a system path or writable by the
   caller on Linux. A valid signature alone is not sufficient. The same check applies to the
   helper: a process that is not the helper binary cannot open a helper session.
4. No secret reaches the broker from the environment in a release build, and none appears in the
   environment of a process it spawns. On Linux a child of the broker inherits neither
   `$CREDENTIALS_DIRECTORY` nor the unlocking key.
5. The broker answers with no user session logged in. Being reachable whenever the machine is on
   is the accepted cost of starting unattended, so it is tested rather than assumed.
6. Where the channel is a socket, bravebot refuses a peer whose uid is not `_bravevault`, so one
   bound or replaced by a process running as the person is not mistaken for the broker. That is
   the control channel on Linux and Windows, and the egress channel on all three platforms.
7. The broker's socket path cannot be created or replaced by a process running as the person, and
   on macOS a user process cannot claim the control service's name in the system Mach namespace.
8. On Linux, a `$HOME/.local/bin` install refuses every credential-backed capability and says why,
   rather than working with a verification it cannot perform.

**Approval integrity has no test in this phase because approvals begin in phase 2.** Its
enforceable consequence is test 9.

**Phase 2, enrolment**

9. A forged approval cannot produce a grant wider than the enrolled binary's ceiling. This stands
   in for confirmation integrity, because no local mechanism provides it.
10. bravebot's enrolment cannot request `reveal`, `grab-session`, or either variant of `use`,
    enforced against the verified binary rather than the caller's claim.
11. Re-enrolling, under any label, cannot widen the action set the bravebot binary is allowed.
12. No call available to bravebot returns credential bytes across its whole permitted surface,
    including any path that would hand plaintext to a process bravebot started.

**Phase 3, the first capability and its bounds**

13. A grant does not answer for a different item, a different action, a different destination, or
    after expiry.
14. A `send_mail` recipient drawn from untrusted content is refused.
15. With the broker stopped, every credential-backed capability refuses and none degrades.
16. The byte cap counts encoded wire bytes, including transfer encoding, and refuses at the
    boundary whatever any approval said. This is the only bound on the body that exists in
    phase 3.
17. Using a grant does not extend it.
18. Caps count at issue, count failures, and apply across grants of a capability rather than
    within one, so single-use grants cannot be issued in a loop underneath a rate cap.
19. Concurrent requests cannot exceed a cap together.
20. The trail records every grant and every use and contains no credential bytes, asserted by
    scanning it for each secret the broker holds.
21. A process running as the person cannot append to, alter or truncate the trail. This is half
    the residual-risk claim, and until it holds, "a legible trail" is an aspiration.
22. A token obtained by pasting is recorded as a lowering, and the default acquisition path never
    puts one through bravebot.
23. The broker binds no loopback listener, the PKCE verifier never leaves it, and the helper
    receives no client secret. The helper relays a code and nothing else.
24. Every envelope address matches the granted recipient or is absent, `Cc`, `Bcc`, `Reply-To`
    and `From` included. Unknown fields in `perform` args are refused.
25. A standing grant is refused when no helper session is connected for that uid. A process that
    is not the helper binary cannot create one. A helper session for one uid does not redeem
    another uid's grants.
26. No permission mode answers a credential approval, bypass included, and a delegate spawned
    under bypass inherits no credential capability from it.
27. An unattended run with no standing grant refuses, and never prompts.

**Blocked, not a gate.** A `send_mail` body above the releasable level should be refused without a
per-call lowering, and that test cannot fail while the planner's output is labelled releasable
whatever its context held. It ships with phase 3 as a disabled check and becomes a gate when
confidentiality propagates through the planner. Test 16 is what bounds the body until then.

**Phase 4, substitution**

28. A placeholder appearing anywhere other than the position the grant names fails the request.
29. A redirect is not followed by the proxy on the client's behalf, and a hop that is followed
    revalidates the destination against the grant first.
30. Of `CONNECT`, the TLS server name and the inner `Host` or `:authority`, every field present
    on the connection matches the grant and the others; a mismatch between any two fails the
    request. Cleartext HTTP is matched on `Host` alone. Matching is on the exact name, never a
    suffix or a literal address.
31. A child that ignores its proxy configuration cannot authenticate with a broker token, and the
    token does not leave the machine.
32. A request with an unsigned payload, a missing date, or a signing scheme the boundary does not
    implement is refused rather than forwarded.
33. The local certificate authority is absent from the person's login keychain.
34. A pinned server and a runtime whose trust store we cannot configure produce distinct errors,
    each naming its own cause.
