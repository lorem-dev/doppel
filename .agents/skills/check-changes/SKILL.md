---
name: check-changes
description: Use after a batch of commits, before opening a pull request, or before a release. Verifies the CHANGES.md Development section reflects every change that landed.
---

# Check CHANGES.md

`CHANGES.md` has a Development section collecting everything since the last
release. This skill checks that it is true.

## Method

List what actually landed, then compare:

```bash
git log --oneline <last-release-tag>..HEAD
```

If there is no release yet, use the first commit. For each commit, decide which
of three buckets it falls into:

- **Belongs in CHANGES.md.** A user of Doppel -- an operator writing a config, a
  developer pointing a client at it -- would behave differently knowing about
  it. New behaviour, changed behaviour, a fixed defect they could have hit, a
  configuration field.
- **Does not.** Refactoring with no observable effect, test-only work,
  documentation, formatting, dependency hygiene that changes nothing outward.
- **Ambiguous.** Write it down. A reader skimming for what changed is better
  served by one line too many than by silence.

## What a good entry says

**Twenty-five words at most per entry.** Count them, and count wrapped lines
too -- a bullet spilling over three lines is over the limit however it looks in
the file. An entry that needs more is either two entries or an explanation that
belongs elsewhere.

Say what changed for the reader, not what was edited. "Rejects a request path
containing `..`" tells an operator something; "hardened `join_upstream`" does
not.

Add as few entries as the change honestly needs. A changelog is scanned, not
read: every line that could have been left out costs the reader attention on the
lines that could not.

Where to put what the fifteen words cannot hold:

| The reasoning, the measurements, the rejected alternative | the commit message |
| How the thing works and how to configure it | `docs/` |
| Why the code is shaped that way | a comment next to it |

None of that belongs in `CHANGES.md`. A reader wanting it has `git log` and the
documentation; a reader wanting to know whether to care has one line.

Where a behaviour changed rather than appeared, one clause on what it was before
is worth the words -- inside the fifteen, not in addition to them.

Keep the existing Added / Changed / Fixed / Notes grouping. Entries are written
as part of the change that caused them; if you are adding several at once
after the fact, that is worth mentioning in your report, because it means the
habit slipped.

## What this skill must not do

Do not invent entries from commit subjects alone. A commit message describes an
edit; an entry describes an effect, and the two frequently differ. Read the
diff when the subject is not enough.

Do not treat a green checklist as the goal. If the Development section is
accurate and short because little user-visible changed, say so.

Do not restore length that was cut. An entry trimmed to fifteen words has not
lost anything a reader of a changelog wanted: check that what was cut is
recorded in the commit message or the documentation, and leave the entry short.
