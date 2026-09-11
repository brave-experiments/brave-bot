# Testing the interface

`cargo test` covers the interface a piece at a time: a key press becomes an action, an action is
handled, a screen is drawn. What it cannot reach is the wiring between those pieces, and that is
where the interface bugs have been. `contrib/drive_tui.py` runs a scripted session against a real
terminal so those paths can be exercised, and `contrib/README.md` says how. It needs a backend and
writes real sessions, so it is a tool to reach for deliberately rather than part of `make check`.
