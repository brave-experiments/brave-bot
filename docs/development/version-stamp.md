# Which build wrote a session

Every session record carries the build that produced it, and `bravebot --version` prints the same
string:

```
bravebot 0.1.0 (f2a6e1a, modified)
```

The commit is what the binary was compiled from, and `modified` means the tree had uncommitted
changes at that point. Both matter when reading a transcript back: a session that behaved oddly
is usually being read against code that has moved since, and the alternative to a stamp is
inferring the build from the transcript's own symptoms. Resuming a session recorded by a
different build says so, beside the note about a changed branch.

The stamp is taken by `crates/tui/build.rs`, which watches every crate's sources rather than only
its own, so `modified` cannot go stale while another crate changes underneath it. A build with no
git available says `(no git)` rather than naming a commit it cannot see.
