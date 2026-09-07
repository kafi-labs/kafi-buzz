# pre-push hook: independent verification report

Verifier scope: `scripts/hooks/pre-push` (305 lines) as of commit `980ccb3bf`,
cross-checked against `scripts/hooks/README.md`. Author-reported 15/15 tests
were not re-run; this report covers only adversarial probing outside the
author's own test suite, per the verification brief.

**Verdict: SOUND-WITH-FINDINGS**

The two headline properties the hardening was meant to establish both hold:
fail-closed-on-unknown survives adversarial input (blank stdin, unreadable
sha, failed `git ls-tree`, failed `git rev-list` all block, with or without
`BUZZ_ALLOW_ADVISORY_PUSH=1` set), and the override is now correctly scoped
to the advisory-filename check only — it cannot bypass the secret-content
check or any fail-closed branch. The advisory-by-filename tree-scope check
matches its documented behavior exactly (verified against
`docs/web-chat-client-plan`, a derived branch with no advisory-touching
commits of its own that still gets blocked). The false-positive argument
holds empirically: `feat/buzz-web-consolidated`, `feat/web-chat-phase2`,
`ops/split-tier-topology`, `feat/intel-turn-quota` all pass clean, both
against their real origin tips and simulated as brand-new branch pushes.

However, the secret-content scan (check b) has several confirmed, concrete
bypasses — none requiring adversarial sophistication beyond ordinary
accident shapes. Ranked by severity:

## Findings

### 1. HIGH — secret-content scan never inspects the commit message
`scan_secrets()` operates entirely on `git diff-tree -p` file-patch output.
Commit messages are never part of that output. A real `nsec1...` key pasted
into a commit message, with completely clean file content, produces exit 0
with zero output — this bypasses **both** the high-confidence and ambiguous
buckets, for **any** secret shape, not just an edge case in one pattern.

Repro: dangling commit via plumbing, tree = one clean file, message =
`"oops pasted key <NSEC-VECTOR> into message"`, where `<NSEC-VECTOR>` is the
canonical NIP-19 bech32 test vector published in the Nostr spec itself
(`nostr-protocol/nips`, `19.md`, "Examples") — already tracked verbatim and
unredacted in this repo, e.g. `scripts/test-k8s-provider-release.sh` and
`benchmarks/harbor-buzz-orchestra/testbed/tests/test_keys.py` (one of the
~10 files noted in `scripts/hooks/README.md`'s false-positive argument).
Redacted here only because pasting the literal string back into this doc's
own tracked content would re-trip this very check on this file — substitute
the real value from either source above to re-run the repro. want=1, got=0.

### 2. HIGH — realistic base64 secret with embedded `/` evades the ambiguous bucket even in perfect context
`B64_CORE` is `[A-Za-z0-9+]{24,}={0,2}` — deliberately excluding `/` to avoid
path false positives (documented in the README). But most real base64
secrets (AWS access keys, many API secrets) contain `/`, which fragments the
run into pieces under the 24-char threshold.

Repro: `.env` file (path-context match) containing
`AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY` (AWS's own
canonical docs example format, 40 chars, two `/`s). Fragments are 13/7/19
chars — none reach 24. want=1, got=0, zero output. This is not a contrived
case: it is the single most common realistic leaked-secret shape, in the
single most obviously "this is a secret" filename, with the exact variable
name the LINE_CONTEXT keyword list is built to catch — and it still evades.
The README documents the `/` exclusion as a false-positive fix without
disclosing this true-negative cost.

### 3. HIGH — a secret split across two lines evades both buckets
`scan_secrets` greps single `+`-prefixed diff lines. Any contiguous secret
broken across a line boundary (string-literal wrapping, concatenation,
editor auto-wrap) is invisible to the character-run patterns regardless of
bucket — this is structural, not one pattern's bug.

Repro: take the same `<NSEC-VECTOR>` as finding 1 and split it after the 7th
character following its `nsec1` prefix:
```
PREFIX = "nsec1" + <first 7 chars of <NSEC-VECTOR> after the prefix>  # under HC's 20-char minimum
SUFFIX = <remaining chars of <NSEC-VECTOR>>  # no nsec1 prefix, no line-context keyword, filename has no path-context match
```
Concatenated at runtime this reconstructs the real, complete test vector used
elsewhere in this test suite. want=1, got=0.

### 4. HIGH — current-format OpenAI keys (`sk-proj-...`) do not match `HC_PATTERN`
`HC_PATTERN` includes `sk-[A-Za-z0-9]{20,}` (no hyphen in the character
class). OpenAI's current default key format is `sk-proj-<...>` — the
embedded hyphen after 4 chars ("proj") breaks the run long before reaching
20 characters, so the pattern never matches.

Verified as a direct regex test (no git needed): a string shaped like
`OPENAI_API_KEY = "sk-proj-<40 random alphanumeric chars>"` (e.g. `sk-proj-`
followed by the output of `openssl rand -hex 20`) does not match
`(^|[^A-Za-z0-9_])sk-[A-Za-z0-9]{20,}` — the embedded hyphen after "proj"
breaks the contiguous-alnum run before it reaches 20 chars. The classic
`sk-XXXXXXXX...` (48 contiguous alnum) format still matches fine — only the
newer, now-default project-scoped format is missed. Redacted to a shape
description rather than a literal value because this finding is now closed
— `HC_PATTERN` gained a `sk-proj-[A-Za-z0-9_-]{10,}` term in the follow-up
commit `7d8a8068a` — so reconstructing the literal fixture verbatim now
correctly re-trips the scan on this file, which is exactly what happened
when this line was first drafted with a real fixture value.

### 5. MEDIUM-HIGH — the diff-scope predicate's belt-and-suspenders `--not "$remote_sha"` term is a no-op due to git's `--not` toggle semantics
`new_commits_for()` builds:
```sh
git rev-list "$sha" --not --remotes --not "$extra_not"
```
In git, `--not` **toggles** the include/exclude sense for subsequent
revision arguments — it is not a scoped prefix that applies only to the
next term. A second `--not` flips back to *include* mode, so `$extra_not`
(`remote_sha`) is added as an extra **positive** tip, not excluded.

Confirmed directly (git 2.50.1, Apple Git-155), isolated from the hook: on a
clean 3-commit chain A→B→C, `git rev-list C --not --not A` returns **all
three** commits (C, B, A) — identical to plain `git rev-list C` with no
`--not` at all. The second `--not` fully cancels the first.

End-to-end repro: isolated repo, no remote-tracking refs fetched (so
`--remotes` is empty), `remote_sha` pointed at a commit that already
contains a planted nsec fixture. Expected clean (the fixture is supposed to
be excluded via the explicit `remote_sha` term). Hook still blocked on the
fixture — because `remote_sha` was never actually excluded.

**Direction is safe**: analytically and empirically this bug can only
*widen* the scanned set (an extra positive tip can add commits to a rev-list
union, never remove protection that `--remotes` already provides), so it
does not create a disclosure risk. But the documented protection — "a
stale/missing remote-tracking ref for this branch shouldn't widen the scan
beyond what the pre-push contract already told us the remote has" — does
not exist in the code as written. It silently degrades to plain
`--not --remotes` on every invocation where `remote_sha` is supplied,
exactly reproducing the cry-wolf false-positive risk the comment says this
term prevents. Suggested fix (not applied — verification only): a single
`--not` covering both exclusion terms, e.g.
`git rev-list "$sha" --not --remotes "$extra_not"`.

### 6. MEDIUM — binary-file and submodule-pointer content is invisible to the scanner
A secret embedded in a NUL-containing blob (git renders it as
`Binary files a/... and b/... differ`, no `+` lines) and a submodule pointer
add (`Subproject commit <sha>`, no file content) both fully evade
`scan_secrets`. This is architecturally inherent to any `diff -p | grep`
scanner and lower-surprise than 1–4, but it is real, reproducible, and
undisclosed in the README's "Ceiling" section, which currently only
discloses `--no-verify` bypass and clone-scoped coverage — not the
content-shape blind spots in 1, 3, and this one.

## Confirmed-correct (adversarial probes that did NOT find a defect)

These are reported because a scanner that finds nothing needs a positive
control — each of these could have come out the other way, and several
early runs of my own harness did (see notes):

- CRLF line endings do not defeat detection — an `nsec1...` key on a
  `\r\n`-terminated line is still caught (the trailing `\r` is `[:space:]`,
  satisfying `POST_BOUND`).
- Detached-HEAD push (`local_ref` literally `HEAD`), annotated-tag push
  (`local_sha` is the tag object, not the commit), a disjoint/orphan
  force-push history, and a two-ref push where only one ref is dirty all
  behave correctly — no crash, correct scoping, and in the multi-ref case
  only the dirty ref's block is reported while the clean ref stays silent.
- `BUZZ_ALLOW_ADVISORY_PUSH=1` correctly fails to bypass: blank/unparseable
  stdin, an unreadable/bogus sha (`git ls-tree` failure), and a real planted
  secret-content block. It correctly *does* bypass a real advisory-filename
  block (tested against the actual `feat/intel-acp-adapter` advisory
  branch) — this is the one thing it is supposed to do.
- The no-remotes-fetched widening claim in the README is accurate: with no
  `refs/remotes/**` at all, the predicate degrades to "all of `local_sha`'s
  history," which is how my positive control was built (an old, already-
  "public" fixture key got re-flagged as new — proving the harness can
  detect a real block, not just report clean by default).
- The false-positive sweep (`feat/buzz-web-consolidated`,
  `feat/web-chat-phase2`, `ops/split-tier-topology`,
  `feat/intel-turn-quota`), both against real origin tips and simulated as
  brand-new pushes, is clean in all 8 runs.

One methodology note: my first pass at the false-positive sweep for
`feat/buzz-web-consolidated` produced a spurious "unparseable pre-push
input" block. Root cause was my own harness — `git rev-parse` on a
non-existent ref echoes the literal ref string to stdout *and* exits
non-zero, and my `||` fallback appended a second line, feeding the hook a
malformed two-line stdin record. Not a hook defect; corrected with
`git show-ref --verify -q` before resolving the ref. Noted here in the
interest of not double-counting a testing artifact as a finding.

## Method

All testing was direct hook invocation via crafted stdin
(`<local_ref> <local_sha> <remote_ref> <remote_sha>`), or `git push
--dry-run`-equivalent construction against disposable local bare repos.
No real push was performed at any point. All planted secrets are synthetic
values already used elsewhere in this hook's own comments/tests (the
canonical NIP-19 bech32 test vector) or well-known vendor documentation
examples (AWS's own example access key). Dangling commits were built via
plumbing (`read-tree`/`hash-object`/`update-index`/`write-tree`/
`commit-tree`), never attached to a ref, per the pattern in
`scripts/hooks/README.md`. Two isolated bare-repo test environments were
used for scenarios that needed controllable remote-tracking state (no-fetch,
disjoint history); everything else ran read-only against this worktree's
real (shared) `.git`, which was never mutated.
