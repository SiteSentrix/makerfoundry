# Notes on *The Book of Debugging* (Johannes Kuhlmann, No Starch Press)

Read in full on 2026-10-19 (the tool lists in ch. 22 on networking, process and memory tools were
skimmed). These are paraphrased working notes, not a reproduction of the book. Page numbers are the
book's own printed numbers, for looking things up. Copied from the autolayout project
(`bwattendorf/autolayout`, `docs/debug-book-notes.md`), where they were first written.
The short operational version is the `/debug` skill (`.claude/skills/debug/SKILL.md`); this file is the
detail behind it. Lines marked **[MakerFoundry]** are a mapping to this project, not the book's words.

## The core claim and the workflow (Intro, ch. "The Workflow", pp. xvii-9)

- Computers are deterministic, so every bug has a cause that can be found, however many steps of
  indirection sit between symptom and root cause. This is the book's reason for hope.
- Four steps, iterated until the bug can no longer be reproduced: **Reproduce -> Probe -> Examine -> Fix.**
  The order is not rigid except that you always start with Reproduce. Probe and Examine run in short
  loops; you can drop back to Reproduce or jump ahead to Fix when you think you have the answer.
  - **Reproduce**: make it happen on demand. Cannot reproduce = fixing it is luck. A reported bug is
    probably real; if you cannot reproduce, you are probably missing a factor.
  - **Probe**: find an *entry point*, anything linking the symptom to the sources. Wide and fast, like
    surveying land. Sources include code, assets, config, docs, third-party parts, not only code.
  - **Examine**: narrow and detailed. Follow connections (function calls, data dependencies) across
    layers and file types to the root cause. If you cannot reach full understanding, the root cause is
    the spot where you can change things to fix the bug and nothing obvious lies underneath.
  - **Fix**: verify by going back to step 1. Be flexible about fix quality: a hacky workaround to
    unblock others while you keep looking is legitimate (see ch. 28).
- Accept responsibility first. Assume the bug is yours, not the compiler's, library's or another
  team's. Do not push back because the report is thin or "that can't be broken." Third-party code others
  have used is less likely to be wrong than code written for this project.
- Team process: bugs go through the tracker; one bug has exactly one assignee; the fixer never closes
  their own bug, a neutral party (reporter/QA) verifies. Outcomes: fixed / unable to reproduce / won't fix.
  Reopen an old report for a recurrence so history is available.
- Expectation management: debugging is high cognitive load and estimates are unreliable, so
  under-promise. Stuck? Sleep on it.
- **Common root causes** (checklist for hypotheses): bad data (nonsensical or corrupted), bad pointers
  (unassigned, stale, dangling, arithmetic), bad logic, edge cases (off-by-one, first/last element,
  time zones, unclean shutdown), floating-point inaccuracy (float vs double can be a quick fix),
  **stale code or assets** (you are not running what you edited: wrong config, wrong file, deploy to
  wrong place, read-only/in-use file, wrong branch; defend by making an *obvious* change and checking it
  shows up), wrong timing or order (worse with threads), unclear/bad requirements (code is right, the
  spec is wrong), UX issues.

## Part I: Reproduce (pp. 11-35)

### What makes a great bug report (p. 11)
Fields: **symptoms** (objective, *no theories*, because theories bias the investigator), **expected
behaviour** (separates bugs from surprising-but-intended behaviour), **numbered repro steps** from
launch to symptom, **reproduction frequency** (attempts and successes, or always/frequently/rarely/once),
severity, platform, **build details** (version, exact commit, configuration), workarounds already tried,
history of previous investigations (reopen the old report), attachments (video, screenshot, input file,
saved state, crash dump), contact info. Split mandatory vs optional so the barrier to reporting stays
low. Add everything you learn to the report: you will not be the last person to look at it. Also ask
reporters "when did this last work?" (feeds bisecting).

### Ch. 1 Reproduce the bug yourself (p. 15)
- Do it yourself, as the reporter did, before debugging code. Example: "network drops when a controller
  is unplugged" turned out to be a modal dialog blocking the network pump, obvious only by using the
  program.
- Match the reporter's setup (hardware, OS, drivers, resolution, background programs, disk space). If
  stuck, do every step in one sitting from a fresh install, including idle delays. If still stuck, watch
  the reporter do it, then have them do it on your machine.
- Skipping slow steps is fine until the bug vanishes; then that step matters, and you have learned
  something. Example: convert tool crashing on file #900 of 1000: find that file and feed only it.
- **Introduce variations** to find which steps are essential: skip steps, reorder, change timing, try
  other places/dialogs, avoid the action entirely. If the bug disappears and nothing else breaks, that
  one action is suspect; if a new symptom appears elsewhere, an earlier step probably corrupted state.
- Networked bugs: first confirm they need a network; run several instances locally; if you need other
  people, write the plan down (variations to try, alternatives, fixes lined up).

### Ch. 2 Shorten the feedback loop (p. 19)
Time from change to seeing the result = build + deploy + startup + getting to the right place.
- **Build**: unity builds, distributed builds, caches (ccache/Bazel), precompiled headers, fewer
  includes/PIMPL, lower optimisation for local builds (but a bug may exist only at -O3). For quick
  experiments put a declaration in the .cpp instead of touching a widely-included header, or use a
  global; mark such hacks with a unique comment (`// stupid`, `// <YOURNAME>`) so they are findable and
  never committed. Assets: profile the pipeline, parallelise, build on demand, lower quality in dev.
- **Deploy**: faster links, overlay only changed assets, deploy less (cut to the one level/enemy), but
  confirm the bug still behaves the same after shrinking (timing bugs can vanish). **Hot reload** helps
  but is itself buggy; restart clean if it looks guilty.
- **Startup**: remove splash/legal screens, menus, preloads the bug does not need; every wait/act switch
  loses time and invites distraction. Shorten repeated steps (5 repeats instead of 10) *only if the bug
  still reproduces*. Optimise loading (SSD, batch reads, compression, parallel decompress).
- **Jump to the location**: command-line args/config to land at the right screen; ideally automated.

### Ch. 3 Automate the reproduction (p. 29)
- Worth it when: many/slow steps, rare bug, exact timing, or you want to leave it running overnight or on
  several devices. The automation must *notify you and leave evidence* (debugger attach, logs, crash
  dump); a repro you cannot inspect gains nothing.
- Methods: command-line/config to reach the spot; hard-code or script the steps (repurposing existing AI
  bots, existing test tools); record/replay inputs (needs determinism; recordings go stale when the
  interface changes; handle device connect/disconnect, axis ranges, very short presses);
  **call the suspect function directly early in `main()`**, in a loop over input ranges or repeatedly
  for flaky ones (example: a path-join function with two separator kinds); load only the offending asset;
  **save/restore state** (save game just before the bug, a parameters file between two executables).
  Caveat: a saved state may itself be wrong because of a different earlier bug.
- Cannot automate => a human factor (timing, movement) or a skipped step matters. That is information.
- **[MakerFoundry]** an integration test that sends the failing request to `makerfoundry::app()`
  (`tests/pages.rs` shows how) is this chapter; the uploaded file or the database rows are the
  "saved state".

## Part II: Probe (pp. 37-97)

### Ch. 4 Try shortcuts (p. 38)
- Educated guesses from experience: the particular program (its quirks, tech debt, long-lived
  workarounds), similar functionality elsewhere (garbled-character box means non-ASCII + missing font
  glyph or bad decoding), frameworks (their pitfalls), foundational APIs (an ambiguous signature:
  `play_samples(data, length, bits)` where `length` meant sample count not bytes), common patterns
  (MVC lag; input lost = polling vs event handling), general knowledge (freeze = deadlock or infinite
  loop; cannot write = disk full or read-only).
- **Suspicious systems**: poorly understood, complex, or foundational systems (task schedulers, network
  glue between sync and async); heavily commented code signals difficulty; systems with many past bugs
  breed more (and fixes on top of fixes add risk). Do not assign blame.
- **Rule systems out**: know responsibilities, work outward from the obviously relevant ones; but
  remember surprising couplings (memory corruption links unrelated systems; a shared `rand()` state
  couples two modules, so give each module its own RNG state).
- Prefer the glue between systems and your own code over trusted libraries: the HTTP-library bug was in
  the usage.
- **Stop shortcuts after about an hour, or after two or three tries, or when guesses stop being
  evidence-based.** Cross investigated systems off the list.

### Ch. 5 Search the internet (p. 47)
Search the exact error text (quoted; but drop the parts the program composes at runtime). Go past page 2
rarely; change terms. Use engine operators (`-term`, `"phrase"`, `site:`, `filetype:`, `before:/after:`),
specialised sites (docs, FAQs, bug trackers, forums, chat archives), code search on the host (GitHub),
disambiguate generic names ("rust lang"). Search the intranet and old tickets too.

### Ch. 6 Check version histories (p. 52)
- If it used to work: find a good and a bad version, **binary search** (git bisect: start/good/bad/
  reset). Only valid if the bug is continuously present after the introducing change. Use existing binary
  builds to narrow the range, then source builds; mix and match code and assets from different versions
  to isolate which side matters. A temporary debug-only change that got committed
  (optimisation disabled on some functions) was found by reading every commit in a two-day window.
- Then read the culprit commit: review modifications one by one (bisect inside a big commit), undo only
  the problematic component, talk to the author. Commit messages and file lists can mislead by omission.
- Look at the future and at other branches/forks too: someone may have fixed it. For third-party code:
  changelogs, roadmaps, upstream repo; consider backporting a single fix vs upgrading. Push for source
  licences on third-party dependencies.

### Ch. 7 Attach the debugger by default (p. 59)
Debugger shows the gap between code-as-template and actual execution (values, branches taken, early
exits via break/return/throw/goto). Break on exceptions; put first breakpoints at the top of a function
or higher in the stack (not inside loops/ifs you "know" run); few and close to the hypothesis; go
higher if never hit. Remove breakpoints when done. When the debugger changes behaviour (timing,
anti-debug), attach *after* the bug has happened and inspect the fallout. Invest in getting a debugger
on difficult platforms and report debugger bugs to the platform owner.

### Ch. 8 Instrument the program to report problems (p. 62)
- **Assertions** express pre/post-conditions and fail *as early as possible, where the assumption is
  first violated*. Use asserts for "should never happen", error handling for things that can happen in
  normal runs (open-file failure). Both can coexist: assert in dev, graceful in shipped builds. Put a
  message in the assert; make it break into the debugger; static asserts for compile-time facts.
- **Check every return value**, including from the standard library and other people's code
  (installer exit code 3010 = reboot pending). Recover where possible, tell the user when recovery changes
  the experience. Signal errors with codes/enums/result types/exceptions.
- **Magic values / sentinels / guard fences**: fill memory with obvious patterns (0xdeadbeef; odd, bad as
  address, bad as code) so untouched/stale/overrun data is recognisable; Visual Studio debug fills 0xcd
  (new), 0xdd (freed), 0xfd (fences). Show data in hex.
- **Do not break the repro with your instrumentation.** Adding a log line once "fixed" a bug by changing
  timing; the team shipped that, never found the cause. Never do that on purpose. Re-confirm the bug
  after each modification.
- **Make instrumentation optional** (flags/compile switches, off by default in shipped builds) and keep
  it: bugs return and error handling stays useful.

### Ch. 9 Remove randomness (p. 69)
- A "random" bug has a cause. Sources: time (wall clock, elapsed, delta), uninitialised/garbage data,
  concurrency (races, scheduler), PRNGs (fix the seed or return constants; note PRNGs can differ across
  platforms: own the algorithm if it matters), external input (users, files: fix contents and
  metadata), communication (network/IPC: replay recordings), then rare hardware (bit flips, last).
- Full determinism needs everything to cooperate: fixed time step, same floating-point operation order
  (fast-math breaks it), deterministic libraries, scheduler independence. Usually aim only for the
  feature in question.
- Make allocation zero-init through a wrapper so you can switch and track it. Serialise threads with
  mutexes to emulate single-threaded behaviour.
- Once deterministic you can (a) reproduce on demand, (b) trust "gone" means gone, (c) compare a good and
  a bad run by logs or stepping. Fixing the weather to "rain" made the rain bug reproducible every start.

### Ch. 10 Ask colleagues for help (p. 79)
Spend about 20 minutes first, and have a short summary ready; asking early and often beats asking never.
Investigating first makes questions precise. Calibrate detail to the listener (sparse facts keep them
open-minded, full context saves their time); keep theories separate from facts; trickle info if stuck.
Wrong person assigned? Catch it early. For outsiders: most specific channel first (framework support,
issue tracker), give a full report plus a **minimal repro**, mind NDAs, be insistent past first-line
triage, use your network, expect days to weeks.

### Ch. 11 Search in all files (p. 86)
Text connects everything; search source, assets, config, logs, databases, docs, tracker, VCS history.
Start broad (substring, case-insensitive), then restrict. Learn regex. Use strings from program output,
but **search partial strings**, since text is composed in code ("Hello" not "Hello 42"; `Hello \d+`).
Binary files contain readable fragments (zip names). Choose unique, searchable names when writing code;
use unique comment markers for changes.

### Ch. 12 Collect crash dumps (p. 91)
For bugs in the wild: handle unhandled exceptions, write a dump (call stack, threads, modules,
optionally memory). Keep **symbol files per build** archived; stripped builds are useless without them.
Out-of-process handler is more reliable than doing work inside the crashed process. Group crashes by
stack, track frequency and release stability; use off-the-shelf (WER, Breakpad/Crashpad, Sentry...).
Optimised builds make dumps harder to read.

## Part III: Examine (pp. 99-178)

### Ch. 13 Step through the code (p. 100)
Use a debug build (optimised builds reorder, merge, delete lines; breakpoints move; -O3 and signed-
overflow UB can change behaviour; some bugs exist *only* in release or *only* in debug because of fill
patterns). Launch the right executable (launchers, updaters, child processes; find real command lines
with ps/Process Explorer). Use breakpoints; **avoid slow conditional breakpoints, write a real `if` and
break inside it, and never commit it**. Read the call stack (what decisions led here); look at locals,
statics, members, globals, memory blocks, other threads. Step over/into/out, run-to-cursor; after each
step re-evaluate. Reason backward from symptom to cause. Assembly view for multi-instruction lines.
Data breakpoints (watch an address, 1-8 bytes, per run) find who corrupts a variable. Corrupted call
stack: infinite recursion or virtual call on garbage; inch toward the call site.

### Ch. 14 Read the code (p. 113)
Be the virtual machine: execute line by line in your head, skim first then verify each instruction,
don't exempt code you "know" works. Record intermediate results (paper, spreadsheet, calculator).
Know the language's pitfalls: implicit conversions, exceptions, polymorphism, operator overloading;
C++ (constructors/destructors run invisibly, temporaries, pointer/array decay: `sizeof` of a parameter
array, `char*` comparison compares addresses, UB, macros that evaluate args twice, shadowing); C#
(properties with side effects, value vs reference, boxing, GC and unmanaged resources, string compare
variants); Python (mutable default args, scope leakage, late-binding closures, Py2 vs Py3 division).
When another program/language is involved, mock it with a *realistic* result or go read it; take notes
on where you left off. Readable code (consistent style, descriptive names, small units, named constants,
error handling) hides fewer bugs; review regularly.

### Ch. 15 Follow the data (p. 122)
Data = inputs, assets, settings, transient results. Bugs live in *transformations*. Find the original
input. **Distinguish undesired data (logic wrong) from corrupt data (implementation bug).** Corrupt-data
tells: values that do not belong (negative sizes), huge or float oddities, **NaN/inf/denormals**
(NaN propagates), garbled text (check encoding first), malformed JSON/XML, null pointers, misaligned or
suspicious pointers (32-bit truncation left pointers like `0x00000000ABAAFE30`). Observe data via
debugger, GUI, logs, output files; prefer config over code changes to expose it. **Bisect the data
flow:** breakpoints/log at each transformation, last-good to first-bad; verify with asserts against known
good data; compare good vs bad (diff tools; UTF-16 read as UTF-8 shows 0x00 every second byte). Also
follow *good* data to learn the system.

### Ch. 16 Question everything (p. 127)
The cause must exist somewhere; if the obvious places are exhausted it is in something you assumed.
Do not trust code because it is old, popular, or written by a respected colleague; symptoms can sit far
from causes (a crash in the physics library was a mis-set collision shape on one object). Look into
trusted code, standard library, data structures (replace with a simpler one to compare), drivers,
graphics APIs (new Vulkan drivers were buggy), battle-tested internal code (new use hits an old edge
case). Check external data, unexpected use cases (10x inputs), user interaction, other programs,
time/clock/time zone, the OS and hardware (disk space, network). **Verify each assumption**: debug it,
instrument it, test the component in isolation (minimal case), swap hardware, read "known issues"/errata,
try a similar program on the same hardware. Study the context (hardware, OS pitfalls, protocols, build
system, language/framework) up front. Do not get lost in irrelevant detail: keep the goal.

### Ch. 17 Experiment (p. 134)
- Exploratory changes: rewrite unclear code into something you understand (that is experiment #1);
  remove or move code; try to make the bug go away or happen more; remove randomness; try all
  combinations when there are few (matrix multiplication orders).
- **For every experiment write: the hypothesis, how you will test it, and what you learn either way.**
  Test with minimum effort; sloppy code is fine for a test. **Try the hackiest possible fix first**; if
  it works you are in the right place, then do the clean one with confidence and keep the dirty one
  around to compare.
- Techniques: hard-code a variable or force a branch; feed fixed values to a function; restructure so
  it runs at startup; rewrite the section more simply and compare (the difference locates the bug); call
  an equivalent API (`size()==0` vs `isEmpty()`, index vs iterator); different abstraction layer;
  comment out or `#if 0` code; add prints/asserts; run old and new implementations and compare
  programmatically.
- Hypothesis sources: suspicious C functions (scanf/memcpy/strcpy buffers), over-complexity, edge cases
  (first/last element, 0, negatives, extremes), control-flow conditions, environment.
- Worked example (p. 138): a stray `;` after `if (size == 0)` meant the loop never ran, then fixing it
  exposed `i <= size`. Test hypotheses one at a time; each fix can reveal the next bug.
- **Make experiments revertible**: unique searchable markers, and/or a clean VCS tree so `git diff` shows
  exactly what you changed. Markers allow parallel experiments and sharing a temporary workaround.

### Ch. 18 Keep a debugging log (p. 140)
Write down everything: places checked and ruled out, hypotheses refuted, supporting data (screenshots,
input/output copies, code with file/line/version, diagrams, CSV for charts). Date/time, project, name.
Name the document by the bug; link it from the bug report. **Log what you intend to do and hope to learn
before doing it**, then the result. Record the final fix and its weaknesses. **Don't chase every new
idea immediately:** write it down specifically, finish the current lead, then prioritise leads by effort
vs probability; quick ones first, weeks-long ones as contingency. "Always have a plan" (have a lead lined
up for Friday). Plain text/markdown is simplest and diffable; wikis give sharing and history. Archive in
a central place; chats count as logs, and answer a chat question in the chat if the solution came
elsewhere.

### Ch. 19 Build your own tools (p. 146)
Tools tailored to your program show what generic tools cannot. Integrated (command console, debug GUI,
hidden commands; accessible but limited by the host), external (nimbler, safer, own language; talk by
files/IPC/network), or hybrid (e.g. built-in HTTP page). Principles: as easy as possible to add a new
tool; easy to discover. Command-line/backend staples: **verbose mode, dry run, run-in-foreground**, an
embedded web UI; libraries: callbacks for diagnostics, an external viewer, record/replay. Games: load
level, jump to checkpoint, god mode, FPS/time breakdown, collision overlay. Launch-mode flags
(single-threaded, alternate backend). Promote and document the tool, collect feedback, decide what to
strip from release builds. Example: an external asset validator that checks every file committed.

### Ch. 20 Build a minimal repro (p. 153)
Strip to the smallest thing that still triggers the bug, either from scratch (empty project with only
the needed libraries, add pieces one by one, base it on a vendor sample for third-party issues) or by
cutting down the current state (disable, don't delete: comment out, move assets away). When the bug
disappears you removed a cause; take that part apart. You can also **minimise the input** when the code
is not yours. Share *source, not binaries*; strip secrets; test on another device first; describe what
you cannot ship (hardware, OS). Turn it into a regression test afterwards (rewrite cleanly rather than
leaving a chopped-up project). Tool captures (e.g. C++ frame capture) can generate one automatically.

### Ch. 21 Debug with printf (p. 158)
A fallback and sometimes the better tool. Downsides: needs rebuild, can change/hide the bug (itself a
clue), inflexible, floods output. Use when no debugger exists/works, the debugger perturbs behaviour,
complex structures are easier to dump yourself, or to watch evolution over time. Add them liberally
because rebuilds are expensive, then iterate by closing holes. Three tiers: a unique searchable marker,
a plain description of what the code is doing, concrete data (values, counts, names). Add metadata:
file:line, severity, category, and a **bug ID tag** (`Bug42:`) to find and clean up all of them. Print
containers element-wise; sort maps; JSON-dump nested structures. Logging libraries for long-running
programs. Cleanup: keep error-case and warning prints with concrete data, drop progress chatter; zero-
cost macros that compile out below a severity; filter at runtime; protect privacy. Save a copy before
removing prints, and re-test after.

### Ch. 22 Use off-the-shelf tools (p. 164)
Choose tools that interpret data for you over raw ones. General: diff/merge (finds hidden Unicode,
whitespace, line endings), disassemblers, file search, format validators/linters/schemas, **fuzzers**,
hex editors, screen recorders, playgrounds (Godbolt, Rust playground). Static analysers and compiler
warnings (enable all, then warnings-as-errors when clean; run several analysers in CI). Sanitizers
(address, memory, thread, UB, leak). Network (browser dev tools, proxies, Wireshark). Plus graphics,
memory, system/process tools (Process Explorer, strace, DTrace, valgrind...). Always try the most
specialised tool the framework provides first.

### Ch. 23 Talk it through (p. 173)
Explaining the bug (rubber duck, a colleague, a group) exposes missing information and contradictions
(Feynman technique). Goal is new leads, not answers: find knowledge gaps and spot correlations. Example:
"all other videos work except this one" -> that video has an odd resolution. Outsiders ask better naive
questions; people sharing your assumptions miss the same things. Schedule it distraction-free, prepare
(your log is the preparation), cover symptoms, repro, location, what you ruled out; allow questions any
time; rephrase to check understanding; be a good listener when roles reverse.

## Part IV: Fix (pp. 179-219)

### What makes a great fix (p. 180)
**Minimal, Complete, Permanent, Clear, Tested, Timely** (timeliness conflicts with the rest). **One bug
per change.** Future-proof it against other data sizes/users/features. **Make it revertible**: needed to
back out a bad fix and to verify that it matters. Root cause may be an asset, config, or process rather
than code. Obstacles to a perfect fix: time pressure, limited control, extensive changes, lack of
expertise, technical constraints.
**Verify:** reproduce repeatedly; *undo the change, see the bug return, re-apply, see it go*; test the fix
alone with other local edits reverted; two causes => two separate commits; have the reporter confirm in
the same environment.

### Ch. 24 Just fix it (p. 182)
Surgical change to the root cause in the *original sources* (not an intermediate artifact that gets
regenerated). Be sure before editing. Watch for features disguised as bugs (route them properly,
unblock the person manually meanwhile). Spec/protocol/design errors can force wide changes; the later
found, the more costly. Example of a confident-but-wrong large fix: rescaling the world by 100 to cure
float precision, later reverted for a floating-origin approach. Bring in experts for unfamiliar areas.
**Test for side effects separately from verifying the fix** (other code may depend on the bug); ideal if
you can switch old/new at runtime.

### Ch. 25 Share what you've learned (p. 186)
Write an informative commit message (root cause, symptom, solution, concrete names, bug link); same in
the tracker; a document for big fixes; a comment in the code where bugs recur; mention in meetings
("Bug of the Fortnight"). **Ask how the cause got in:** ignorance (educate) or oversight (fix process),
or knowing tech debt; check the commit, docs, calendar (deadlines), and ask people; postmortem for big
ones. **Prevent recurrence**: keep useful asserts/error handling/tools (or make them switchable), turn
the automated repro into a regression test (a library of tests from fixed bugs), add targeted safeguards
(loud alerts where possible), change specs/interfaces that invite misuse, improve processes (reviews,
branching), coding and asset standards.

### Ch. 26 Search for related problems (p. 191)
After the root cause is fixed, hunt the same mistake elsewhere: literal copies (search fragments without
concrete data, e.g. `int i = 0; i <=`), conceptual copies (same misconception, same difficult API),
callers of the same dependency (find all references), the same system/author via blame and history,
duplicate files. Dependency changes that compile silently (changed parameter meaning, compatible-type
change, new precondition) are a classic source: version numbers in data, fail the build on change,
verify preconditions first thing. **Bulk speculative changes are risky**: reproduce where you can, verify
each spot, give QA the list, review. "How did this ever work?" is common (dead code, or everyone assumed
it was reported).

### Ch. 27 Fix early (p. 196)
Fresh context makes it cheaper; later, other code comes to depend on the bug, and people leave. Prioritise
by severity *and* by long-term effects (reputation, slowing testers, duplicate reports). Prepare for
attrition: shared ownership, handover lists, bus factor. **Dependent behaviour**: something that only
works because of the bug (saved/persistent data needs a migration path, assets need repair; an example:
two dead-zone layers on one input). Don't keep a bug as an accident without implementing it properly.
Reserve regular time for bug fixing; own your systems' bugs.

### Ch. 28 Deploy a quick fix (p. 201)
Unblock others while you search. Option 1: **revert the offending change** (git revert; manual revert for
partial; swap binary assets for old copies; a tool that deleted users' Documents folder was reverted
immediately). Option 2: **workaround** that avoids or hides symptoms (alter conditions, thresholds,
hard-code, simpler code, linear search instead of hash, disable threading, ignore the error, disable
optimisation for a function, restart, replace/minimal assets, change environment/hardware). Risks:
confusing users, new code building on the workaround. **Communicate manual workarounds** to users.
**Don't stop at the workaround:** say in the commit message it is a workaround, `FIXME` + name, a tracker
task, and keep looking.

### Ch. 29 Start over (p. 206)
Rewrite the offending function from scratch (not refactor), ideally by someone else, simpler (no raw
pointers, threads), with unit/integration tests including the failing case, incrementally, standalone
with stubs, same interface so it can be swapped in. **Clean room** (from a spec: valid inputs, outputs,
failure cases, derivation; not the buggy code) vs **dirty room** (read the original, abstract it, never
copy line by line, keep it out of sight). Test it in the program; if both are identically wrong, the bug
is elsewhere (maybe your function is only a dependency). **Compare outputs**, switchable at runtime or by
flag, automated with an alert on first divergence, copy inputs so one run cannot mutate the other's,
then work backward through the data. Keep the new implementation as a test oracle. Stubs: hard-coded
known-good answers to rule a subsystem in or out.

### Ch. 30 Find a creative solution (p. 212)
Last resorts, after everything else: **declare it a feature** (but then implement it properly, since
accidental behaviour is unstable); **remove the feature** (disable, don't delete: comment out,
compile-time switch, or runtime flag; watch technical and logical dependencies; keep a hidden way to
enable for testing); **leave it** (known-issues list, triage). Get stakeholder consent, document the
decision and link it from the code, tell users with a pessimistic date, and **track technical debt** in a
shared list.

## Conclusion: four parting thoughts (p. 225)
1. **Reproduce the bug** first, last (to confirm the fix) and constantly in between.
2. **Build a debuggable program**: readable source, descriptive names, switchable diagnostics, simple
   designs.
3. **Keep practicing**; intuition about likely entry points comes from repetition.
4. **Every bug has a cause.** Work systematically and do not miss details.
Beyond bugs, the same method applies to verifying changes, performance profiling (the profiler is the
probe step), real-world faults (a tripped breaker found by switching circuits back on one at a time),
and learning an unfamiliar project. Prevention is outside the book's scope: training, reviews, CI,
automated tools, QA.

---

## How MakerFoundry uses these notes

The `/debug` skill is the short version. It was first written for autolayout; MakerFoundry's copy
is adapted to a Rust web service and also carries the points the autolayout skill left out: the
stale-code check (p. 8), symptoms-only reports with a reproduction frequency, dependent behaviour
(p. 198), clean-room vs dirty-room rewrites with the old code as an oracle (p. 208), stubbing a
subsystem with known-good answers (p. 212), disable-don't-delete and tracked technical debt
(pp. 215-219), asking how the cause got in (p. 187), last-good/first-bad bisecting of the data flow
(p. 126), the time-boxes (pp. 47, 80), and keeping probe-only logging switchable (p. 69).
