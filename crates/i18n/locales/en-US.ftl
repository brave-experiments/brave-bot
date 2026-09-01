# The reference catalog. It owns the set of messages and the name and kind of every
# argument, so a translation can add none of its own and change no call site.
#
# Ids are kebab-case and grouped by the surface they appear on. A message is named for
# what it says, never for where it happens to sit, so moving a line between two panels
# does not rename it.

## Counting

count-turns = { $count ->
    [one] { $count } turn
   *[other] { $count } turns
    }
