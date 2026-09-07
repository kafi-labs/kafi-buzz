# pre-push hook: advisory + secret content scan

## Authoritative copy

**`.git/hooks/pre-push` (the raw hook git actually runs) is authoritative for
execution.** `scripts/hooks/pre-push` in this repo is the tracked, reviewable
source of truth. Git never reads hooks from inside a tracked path — you must
copy this file into place after cloning or after any change to it:

```sh
cp scripts/hooks/pre-push .git/hooks/pre-push
chmod +x .git/hooks/pre-push
```

Because `.git` in a clone with worktrees is shared (`git rev-parse
--git-common-dir` resolves to the parent clone's `.git` from every worktree),
installing it once in the main clone covers every worktree of that clone. It
does **not** cover a separate clone — see "Ceiling" below.

Verify the two copies match after installing:

```sh
diff scripts/hooks/pre-push "$(git rev-parse --git-common-dir)/hooks/pre-push"
```

## What it does

Two independent checks, run for every ref in the push:

1. **Advisory-by-filename** (unchanged from the original hook): refuses to
   push any ref whose tip **tree** contains `specs/SECURITY-ADVISORY*`,
   `specs/DISCLOSURE*`, or `specs/UPSTREAM-ISSUE-DRAFTS*`. Exists because
   kafi-labs/kafi-buzz is a public fork carrying an undisclosed third-party
   security advisory on `feat/intel-acp-adapter`.
2. **Secret-by-content** (new): refuses to push any ref whose push would
   **newly introduce** key/credential-shaped material — high-confidence
   patterns (bech32 private keys, PEM private-key blocks, known service token
   prefixes) unconditionally, and ambiguous high-entropy hex/base64 only when
   it appears in a secret-shaped variable or file name.

## The scope split — READ THIS BEFORE "SIMPLIFYING" THE TWO CHECKS INTO ONE SCAN

The two checks use **different scopes on purpose**, because they guard
**different hazards**:

| Check | Scope | Hazard | Why that scope |
|---|---|---|---|
| Advisory-by-filename | **Tree** of the tip commit (`git ls-tree -r`) | **Presence.** A branch discloses the advisory just by carrying the file at its tip, regardless of who added it or when. | A docs-only branch cut off the advisory branch (no new commits of its own touching the advisory files) still carries them in its tree and must still be blocked. Diff-scoping this check would miss exactly that case — verified against `docs/web-chat-client-plan`, which contains nothing but a web chat plan yet still carries the advisory files because it branched off the advisory branch. |
| Secret-by-content | **Diff**: only commits reachable from the pushed sha that are **not already reachable from a remote-tracking ref this clone knows about** (`git rev-list <sha> --not --remotes`) | **New introduction**, not presence. Material already public via a remote we know about is not disclosed again by re-pushing it. | block/buzz's own upstream history ships real, full-length bech32 `nsec1...` private keys as test fixtures — e.g. `benchmarks/harbor-buzz-orchestra/testbed/tests/test_keys.py` and `desktop/tests/e2e/key-import-reveal.spec.ts` use the **canonical NIP-19 test vector published in the Nostr spec itself**; a wider mining pass found 10+ such files. Tree-scoping the secret check would re-flag every one of those fixtures on **every push of every branch**, since every branch inherits `main`. That is a 100% false-positive rate on day one — the exact cry-wolf failure that gets a hook deleted, and a deleted hook protects nothing. |

Do not merge these into one scan. If you're tempted to "simplify" — the
advisory check has to stay tree-scoped or it stops catching the rename/cherry-pick/derived-branch case it exists for, and the secret check has to stay
diff-scoped or it reproduces the false-positive blowup above.

### Diff-scope predicate, precisely

```sh
git rev-list "$local_sha" --not --remotes --not "$remote_sha"   # (remote_sha term added only if it resolves to a known, non-zero commit)
```

- `--remotes` (all `refs/remotes/**`) is the primary boundary: "already visible
  to something we know about" for **any** remote, not just the one being
  pushed to (this repo tracks both `origin` = kafi-labs/kafi-buzz and
  `upstream` = block/buzz; either counts as already-public for this fork's
  purposes).
- The explicit `--not "$remote_sha"` is belt-and-suspenders for the case
  where the local remote-tracking ref for *this specific branch* is stale or
  missing but the pre-push contract still told us what the remote already
  has.
- Deliberately **not** `--branches`/`--all`: other local branches (e.g. the
  one carrying the undisclosed advisory) are not public. Their content must
  still count as "new" if it shows up on a different, pushed branch.
- **Known limitation:** if `--remotes` resolves to nothing at all (a clone
  with no remote-tracking refs fetched), the predicate degrades to "all of
  `local_sha`'s history," which re-widens the scan and can reproduce the
  fixture false positives above. This is the deliberate fail-closed direction
  (scan *more*, not less) rather than silently skipping the scan — but it
  means this hook is only well-behaved in a clone that has actually fetched
  its remotes. In this shared clone that's always true in practice.
- New-branch pushes (`remote_sha` = all-zero) are handled natively by this
  predicate — no special case needed — because it only depends on local
  remote-tracking state, not on `remote_sha`.

## Where the ambiguous line is drawn, and why

A bare 64-hex string is genuinely ambiguous in this repo: Nostr public keys,
event ids, and NIP-11 fields are 64-hex and are everywhere legitimately (e.g.
a relay's public `"self"` field in a NIP-11 response). A rule that blocks all
64-hex would have blocked a real, legitimate push made an hour before this
hook was hardened.

**High-confidence indicators block outright, unconditionally, on any
occurrence in newly-introduced content:**
- bech32 private-key prefixes: `nsec1…`, `ncryptsec1…` (bech32-alphabet
  continuation of 20+ chars, to reject bare prose mentions like "nsec1..."
  while still catching real keys — bech32's alphabet already excludes `1`,
  `b`, `i`, `o`, which keeps this from matching ordinary English runs)
- PEM private-key headers: `-----BEGIN [RSA|DSA|EC|OPENSSH|ENCRYPTED] PRIVATE KEY-----`
- known service-token prefixes: `intel_`, `sk-`, `ghp_`/`gho_`/`ghs_`,
  `AKIA…`, `xoxb-`/`xoxp-`/`xoxa-`/`xoxr-` — each with a word-boundary guard
  and a minimum trailing length, so `desk-lamp`/`task-list`-style English
  hyphenation can't accidentally satisfy `sk-[A-Za-z0-9]{20,}` (the class
  excludes `-`, so a real hyphenated phrase breaks the run; a real token is
  one unbroken alnum blob)

**Ambiguous material — a long hex or base64-looking run — blocks only when
it also looks like a *value*, not prose or an identifier, AND appears in a
secret-shaped context:**

- *Candidate shape*: `[0-9a-fA-F]{32,}` (hex) or `[A-Za-z0-9+]{24,}={0,2}`
  (base64, deliberately **without** `/`) — see "Boundary anchoring" below for
  why the naive version of this was wrong.
- *Context* (either is sufficient):
  - **Line context**: the same line names a secret-shaped variable —
    `secret`, `token`, `password`/`passwd`, `credential`,
    `priv(ate)?[_-]?key`, `api[_-]?key`. Deliberately **excludes bare
    "key"**: in this codebase "key" alone names public material as often as
    private (`pubkey`, `relay_key`, `channel_key`) — only `private`/`priv`
    combined with `key`, or `secret`/`token`/`credential`/`password`/`api_key`,
    name a variable as secret-shaped with acceptable precision. Verified: a
    64-hex value assigned to `relay_pubkey =` does **not** trip this; the
    same value assigned to `secret_token =` does.
  - **Path context**: the file name itself says `secret`, `credential`,
    `password`, `.env`, `id_rsa`, `id_ed25519`, `.pem`, `.p12`, `.pfx`, or
    `private_key`/`priv_key`.
- *Degenerate-value filter*: a candidate that's a single repeated character
  (all-zero, all-`f`, etc.) is never flagged even in a secret-shaped context.
  These are common, zero-entropy placeholders (this hook's own all-zero sha
  check is exactly this shape) and would otherwise be a steady drip of false
  positives.
- **There is no override for either bucket.** See "Override policy" below.

### Boundary anchoring (why the candidate patterns aren't a bare character-run)

The first version of this hook used bare `[A-Za-z0-9+/]{24,}` for the base64
candidate. Tested against `feat/intel-acp-adapter`'s own (large, legitimate)
diff before shipping, it lit up repeatedly on **architecture-doc prose that
never contained a value at all** — because `/` is part of the base64
alphabet, so a path like `services/gateway/app/middleware/auth.py` is 36
straight base64-class characters, and a descriptive sentence containing the
word "token" or "API keys" nearby turned that into an "ambiguous hit." The
same shape also matches long camelCase identifiers in code
(`validateApiKeyForWorkspace...`).

The fix: candidates must be **flanked by punctuation that marks a value**,
not by another identifier character, a `/` (path separator), or a `(`
(which would make it a call, not a value):

- allowed before: start-of-line, whitespace, `"`, `'`, `=`, `:`, `,`, `(`
- allowed after: end-of-line, whitespace, `"`, `'`, `,`, `)`, `;`

This is checked on the *candidate*, separately from the LINE_CONTEXT keyword
gate, and only affects whether the ambiguous bucket fires — it does not apply
to the high-confidence patterns, which are already unambiguous by shape.

## Override policy

`BUZZ_ALLOW_ADVISORY_PUSH=1` is preserved, **narrowed to exactly the
advisory-by-filename check**. It exists for one purpose: a deliberate,
owner-authorised disclosure decision about the advisory. It does not, and
must not, touch the secret-content check.

**There is no override for the secret-content check, in either bucket.**
Considered and rejected:

- A private key or credential should never leave a laptop into a public fork
  on purpose. Unlike the advisory (a real, reviewable file whose disclosure
  is sometimes the right call), there is no legitimate reason to *force*
  push of matched key material — the correct action is always to remove it
  (and rotate it, if real) before pushing again.
- For the ambiguous bucket specifically, a false positive is real (an
  intentionally-named test fixture, a variable that happens to say
  "secret"/"token" while holding non-secret data). The remedy for that case
  is to **rename the variable or file so it no longer reads as
  secret-shaped**, not to add an escape hatch. This is also a healthy nudge:
  don't name public data "secret" or "private_key."
- One more consideration deliberately **not** re-used: the original
  `BUZZ_ALLOW_ADVISORY_PUSH` override, at the end of the hook, was checked
  against the *global* `blocked` flag — meaning it would also have silently
  bypassed the fail-closed-on-unknown paths (unparseable stdin, unreadable
  tree) had that env var been set for an unrelated reason. This version
  scopes the override strictly to the advisory-filename hit; the
  fail-closed-on-unknown paths and the secret-content check always block
  regardless of the override's value. This is a narrowing of the flag's
  blast radius, not a new capability, and is a strict hardening over the
  pre-existing behavior.

## Ceiling — what this hook is, and is not

**This hook is a convenience that catches accidents. It is not a boundary,
and must never be cited as one.**

- **It is bypassable by design.** `git push --no-verify` skips it entirely —
  demonstrated directly against the branch carrying the undisclosed advisory
  (exit 0, zero hook output; the hook did not run). That is what the
  mechanism *is*, not a defect to fix.
- **Its coverage boundary is the clone, not the machine and not the lane.**
  `git rev-parse --git-path hooks` from inside a worktree resolves to the
  *parent clone's* `.git/hooks` — every worktree lane on this clone inherits
  it, but a **separate clone** (a fresh `git clone` of the same remote, a
  different machine, CI) gets nothing. Silently — no error, no signal that a
  control is missing.
- **It protects only people who installed it and did not pass a flag.** That
  is worth having. It is not something anyone should point at and say
  "secrets cannot leave this repo."
- A real boundary sits where the pusher has no authority to switch it off:
  server-side push protection, secret scanning on the receiving end (e.g.
  GitHub secret scanning / push protection on kafi-labs/kafi-buzz), or a CI
  job that fails a branch after the fact. **That work is tracked separately
  and is deliberately out of scope here**, so that hardening this hook is
  never mistaken for having established a boundary.

## Testing

Never test by performing a real push. Either:

- Invoke the hook directly, feeding it stdin in the format git provides:
  `<local_ref> <local_sha> <remote_ref> <remote_sha>`, e.g.:
  ```sh
  printf 'refs/heads/x %s refs/heads/x %s\n' "$LOCAL_SHA" "$REMOTE_SHA" \
    | .git/hooks/pre-push origin git@github.com:kafi-labs/kafi-buzz.git
  ```
- Or `git push --dry-run` against a **disposable local bare repo**
  (`git init --bare /tmp/scratch.git`), never the real remote. The hook runs
  client-side using your own clone's remote-tracking refs regardless of
  which remote you're dry-running against, so this exercises the real `git
  push` code path safely.

For planting a synthetic secret to test the positive-control path, build a
**dangling commit via plumbing** rather than committing to any real branch —
it never touches the working directory or any ref, so there's nothing to
clean up beyond letting normal git gc reclaim the unreferenced object later:

```sh
idx=$(mktemp)
GIT_INDEX_FILE="$idx" git read-tree main
blob=$(printf '%s\n' 'SYNTHETIC-TEST-VALUE' | GIT_INDEX_FILE="$idx" git hash-object -w --stdin)
GIT_INDEX_FILE="$idx" git update-index --add --cacheinfo 100644,"$blob",scratch/PLANTED.txt
tree=$(GIT_INDEX_FILE="$idx" git write-tree)
commit=$(git commit-tree "$tree" -p main -m "test: planted synthetic secret, never pushed")
rm -f "$idx"
```
