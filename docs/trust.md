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

## The map belongs to the session, not the directory

**You are asked every time a session starts.** Whatever you or anyone else answered in this
directory before makes no difference. The question grants standing permission, so a launch that
skipped it because someone said yes last week would be granting that permission on behalf of a
user who was never asked, and trust assumed from silence is not trust granted.

Resuming with `--resume` is the one case that does not ask. That is not an exception to the rule
above: the map comes out of the record of the session you picked, so the answer being honoured is
the one you gave that session. It carries the rules that session's writes recorded along with it,
which is what stops a resumed turn reading back a file an earlier turn of the same session
poisoned. A record from before maps were kept has none, and is asked about.

The consequence is worth stating plainly. A file that one session recorded as untrusted is **not**
remembered by the next session started fresh in that directory: say yes to the directory and it is
read as trusted again. Within a session, and across a resume of it, the second row of the table
holds. Across a fresh start it does not, because a fresh start has no memory to hold it in. If a
file holds content you do not trust, the answer is to say no to the directory, or to not leave it
there.
