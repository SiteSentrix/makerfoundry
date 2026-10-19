---
name: debug
description: Systematic debugging workflow (reproduce, probe, examine, fix) for MakerFoundry. Use when a page or request gives the wrong result, a test fails and the cause is not obvious, an upload is read wrongly, a payment or webhook misbehaves, a Railway deploy is unhealthy, or a change "should have fixed it" and did not. Based on Kuhlmann, The Book of Debugging (No Starch Press); chapter and page numbers refer to it, and docs/debug-book-notes.md has the detail.
---

# Debugging workflow

Use this when something fails and the cause is not obvious. The book's core claim: the computer is
deterministic, so every bug has a cause that can be found. A workflow keeps effort from scattering.
Loop through the four steps until the bug cannot be reproduced; always start with Reproduce.
`docs/debug-book-notes.md` has the chapter-by-chapter notes behind each line here.

## 0. Before anything else

- Take responsibility: assume the bug is in our code, not in axum, Postgres, Stripe, Railway or the
  user's file (Intro). Library code many people use is less likely to be wrong than ours; the glue
  between our code and it is the likeliest place.
- Open a debugging log (template below) and write the **symptom** (objective, no theory), what you
  **expected**, the exact request or command that shows it, and how often it happens (attempts and
  failures, or always / often / rarely / once).
- **Check you are running what you think** (p. 8, stale code): is the binary rebuilt, is the
  browser showing cached CSS, is the Railway deploy the commit you think (`/health` shows the
  version; the Deployments tab shows the commit), is it the right branch and the right database?
  Make an obvious change and confirm it shows up before trusting any result.
- One bug at a time. Park new ideas in the log's leads list instead of chasing them (ch. 18).

## 1. Reproduce (ch. 1-3)

- Make it happen on demand, from a named input and a command. Can't reproduce it? Then a factor is
  missing; find it before touching code.
- **Shorten the loop first** (ch. 2): prefer a test over clicking through pages. An integration test
  that sends the request to `makerfoundry::app()` with `tower::ServiceExt::oneshot`
  (`apps/makerfoundry/tests/pages.rs` shows how) runs in milliseconds; `cargo test -p makerfoundry
  <name>` runs just that one. For a bad upload, a parser test fed only that file.
- **Cut the case down** (ch. 20): the smallest file, request or row set that still fails. If it
  stops failing, the part you removed matters; put it back in halves. Minimise the input when the
  code isn't ours.
- Vary one thing at a time (order, size, user, browser, phone vs desktop) to learn which steps matter.
- Production-only bug: reproduce locally with the same data shape, never by experimenting on live
  users' data. Payments: **Stripe test mode only**, with the Stripe CLI to replay webhooks.

## 2. Probe: find an entry point (ch. 4-12)

- Make an educated guess, but **time-box it**: stop after about an hour, or two or three guesses that
  didn't pan out, or when guesses stop resting on evidence (ch. 4). Then switch strategy.
- Rule systems out as well as in; cross them off in the log. Remember surprising couplings (shared
  state, a shared database row, a shared cache).
- Search the exact error text, minus the parts filled in at run time (ch. 5, 11). Search everything:
  code, templates, CSS, migrations, config, logs, `docs/`, git history.
- **Bisect when it used to work** (ch. 6): `git bisect` between a good and a bad commit, then read
  the culprit diff change by change. Look at other branches and upstream changelogs too.
- **Instrument before theorising** (ch. 8): checks that fail where the assumption is first broken,
  every `Result` handled, bad data made to look obviously bad. Here shipped code can't `assert!` or
  `panic!` (CLAUDE.md › Never crash), so use `debug_assert!`, tests, or an `Err` with a clear message.
- **Instrumentation must not change the bug** (p. 69): re-check the bug still happens after adding it.
  Keep useful diagnostics behind a switch (env variable or flag), off by default; delete probe-only
  prints before committing (tag them `Bug-<topic>:` so a search finds them all).
- **Never log secrets, passwords, tokens, card data or file contents** (CLAUDE.md › Privacy), not even
  while debugging. Log sizes, counts, ids and hashes instead.
- **Remove randomness** (ch. 9): fixed time, fixed seeds, fixed ordering (sort map output), one
  request at a time. A "random" failure has a cause: time, uninitialised state, concurrency, input.
- Ask for help after about 20 minutes of real, stuck effort (ch. 10), with a short summary from the
  log; for the owner, plain words and exactly what you need.
- Production: Railway's deploy logs (Deployments → the deploy → View logs) are the crash dump (ch. 12).

## 3. Examine: find the root cause (ch. 13-23)

- **Follow the data** (ch. 15): find the original input (the request, the uploaded file, the row),
  follow it through each transformation, and find the **last good and first bad** step. Tell
  *undesired* data (the logic is wrong) from *corrupt* data (an implementation bug: NaN or infinite
  floats, negative sizes, garbled text, so check the encoding first). Compare a good case and a bad
  case side by side.
- **Question everything** (ch. 16): list your assumptions and verify each one. Don't trust code for
  being old, popular or "obviously fine"; check that a branch actually runs (count it).
- **Read the code as the machine would** (ch. 14), line by line, including the code you "know" works.
  Rust pitfalls: integer overflow wraps in release and panics in debug, `as` casts truncate silently,
  float-to-int casts saturate, `.await` points let other requests interleave, an `Option` or `Result`
  quietly swallowed by `.ok()`, `unwrap_or_default()` or `let _ =`.
- **Experiment with a hypothesis** (ch. 17). For each experiment write: the hypothesis, how it will be
  tested, and what you learn either way. Test with the least effort. Try the **hackiest possible fix
  first** to confirm you found the right place, then write the clean one. Keep experiments revertible:
  a clean tree so `git diff` shows them, or a unique marker comment.
- Don't discard a failed experiment as "no good" until it has been re-tested on a baseline known to
  be correct; if the baseline had a bug, re-run the reverted experiments.
- **Stub a subsystem** with known-good answers to rule it in or out (p. 212): a fixed user, a fake
  storage, a canned Stripe event.
- **Build tools** (ch. 19) when you keep wanting to see something: a flag, an admin-only page, an
  `xtask` command. Document it, or it doesn't help anyone.
- Talk it through (ch. 23): write the problem out in the log as if explaining it; the wrong
  assumption often shows itself.

## 4. Fix (Part IV, ch. 24-30)

- A great fix is **minimal, complete, permanent, clear, tested** and timely, at the root cause in the
  original sources (not a generated file). **One bug per change**; two causes are two commits.
- **Verify**: reproduce several times; **undo the fix and see the bug return**, re-apply and see it
  go; test it alone, with other local edits put away. The repro becomes a regression test.
- **Check side effects separately** from the fix (ch. 24). Watch for **dependent behaviour** (p. 198):
  code or stored data that only worked because of the bug. Data written while the bug was live may
  need a migration or a repair script.
- Money: if the bug touched charges, fees, payouts or refunds, check the ledger rows it wrote and say
  what needs correcting. Don't correct real money without the owner.
- **Share what you learned** (ch. 25): commit message with the symptom, root cause and fix; update
  the plan if it changes it. **Ask how the cause got in** (a gap in knowledge or a slip in process)
  and add the safeguard that stops it coming back: a test, a lint, a check in `cargo xtask ci`, a type
  that can't hold the bad value, a clearer interface.
- **Search for related problems** (ch. 26): the same mistake elsewhere (search for the pattern), other
  callers of the same API, the same misunderstanding. Verify each spot; bulk speculative edits are risky.
- Can't fix it cleanly yet (ch. 28)? Unblock first: **roll back** (Railway: redeploy the last good
  deploy; git: `git revert`, never force-push) or a workaround, and say so in the commit message with a
  `FIXME(<topic>)` and a line in the plan. Then keep looking: a workaround is not the end.
- Rewrite (ch. 29): from a spec of inputs, outputs and failures (clean room), not by copying the buggy
  code. Keep the old version runnable and compare outputs; it is a test oracle. If both are wrong in
  the same way, the bug is elsewhere.
- Last resorts (ch. 30): call it a feature (then implement it properly), turn the feature off
  (**disable, don't delete**: a flag), or leave it as a known issue. Each needs the owner's consent;
  record the decision and the technical debt.

## Anti-patterns (avoid)

- Changing things one at a time hoping the result moves (desperate guessing, ch. 4).
- Trusting a result from a binary, deploy, cache or database you didn't confirm is current.
- Reading a passing test or a green check as proof the feature works: look at what it actually did.
- Running a slow manual check when a fast test answers the same question.
- Keeping a log line or a timing change that made the bug "go away" without knowing why.

## Debugging log template

Create `docs/debug-logs/YYYY-MM-DD-<short-topic>.md` and link it from the fixing commit:

```
# <topic>  (page or request, commit, local / Railway)
Symptom (objective, no theory):
Expected:
Repro (request or command + input):   (fast version: the test name)
Frequency (attempts / failures):

## Leads (prioritised: effort vs chance)
1. ...

## Experiments
- [date] Hypothesis / test / result / what it rules in or out

## Ruled out
- ...

## Root cause / fix / verification (undo-redo) / how it got in / safeguard / related places checked
```

No secrets, tokens, personal data or users' file contents in a log: it is committed.
