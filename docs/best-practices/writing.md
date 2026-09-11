# Writing

<!-- applicability: always -->

Every word the repository ships: comments, commit messages, pull request bodies, documents, and
replies on a review.

---

<a id="WR-001"></a>

## Comments explain why, never what

**Prefer no comment to a restatement of the code.** A comment earns its place by saying something
the code cannot: the reason for a choice, the constraint that rules out the obvious alternative,
the cost that was accepted.

```rust
// Wrong: the line below already says this.
// Increment the counter.
count += 1;
```

---

<a id="WR-002"></a>

## Never write about your own process

**Not in a commit message, a spec, a comment, a pull request, or a reply.** "I was wrong earlier",
"as I said above", "this corrects what I claimed", "I initially thought", and narration of what was
checked, guessed or assumed are all noise. Nobody reading this later shares the conversation it came
from.

State what is true about the code, in the present tense, as though saying it for the first time.
Where a correction matters, the corrected fact is the whole of it: write "an absent store reports
nothing and the turn runs on the free tier", not "I said it warns, but it does not". This applies
most where it is most tempting, which is immediately after getting something wrong.
