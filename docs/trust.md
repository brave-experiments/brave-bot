# Trusted directories

At startup you are asked whether you trust the working directory. Trusting it means files
there are read as **trusted**, which is what lets ordinary work proceed without a prompt for
every edit. Decline and nothing is trusted, so every write is shown to you.

Trust is per path, and the **most specific rule wins**, so a trusted project can contain an
untrusted subtree, and that subtree can contain a trusted path again.

A prompt asks you one thing: **may this path stop being trusted?** That is the only
consequence a later step cannot undo, since a path recorded as untrusted can no longer be
examined or edited.

The table below is the specification.

| data | destination | prompt? | effect on the trust map |
|---|---|---|---|
| trusted | trusted | no | unchanged |
| untrusted | trusted | **yes** | that path becomes untrusted |
| trusted | untrusted | no | that path becomes trusted |
| untrusted | untrusted | no | unchanged |
| either | *never mentioned* | **yes** | that path takes the data's trust |

Writing trusted data never asks. For data to be trusted the turn must have observed nothing
untrusted, so there is no attacker-influenced byte in it, and the destination only gains trust,
never loses it.

The last row is why a path nobody has mentioned differs from one you deliberately marked
untrusted: the first has no decision behind it, so the first write there is the moment to ask.
This is also what makes declining at startup meaningful: with nothing vouched for, every write
is shown.

The second row is what closes the obvious hole. If untrusted data, meaning anything derived
from the web or from a file outside a trusted path, is written into a trusted directory, that
file is recorded as untrusted. Reading it back returns untrusted data. Otherwise a round trip
through the filesystem would launder injected text into trusted input, and the trust map would
become a bypass for the gate it exists to support.

Marking is always per file, never per directory: one untrusted file does not taint its
siblings.

## The map outlives the session

The second row is worth nothing if the map is forgotten when you quit. A file recorded as
untrusted on Tuesday and read back as trusted on Wednesday is the same laundering, taking a
restart instead of a round trip. So the map is written to `~/.bua/trust`, one file per working
directory, and the next session in that directory starts from it.

That is also why the startup question is asked only once per directory. Having vouched for it
is standing permission, and a session that asked again would be offering to overwrite the rules
the last one recorded. It does not: an answer applies **on top of** the stored map, and the
rules already there are more specific, so saying yes to a directory cannot un-say what a write
recorded inside it.

A map that cannot be read is not a reason to ask again, since the rules that would have
overridden the answer are exactly the ones that were lost. Nothing is trusted for that session
and the session says so. Deleting the file is what asks again, and doing that is a decision you
are making with your eyes open rather than one a corrupt file made for you.
