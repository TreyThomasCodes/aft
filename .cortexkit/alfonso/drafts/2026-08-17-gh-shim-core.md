---
title: "gh routing shim: CORE binary, rung, manifest, and passthrough"
date: 2026-08-17
status: draft
---


## intent
Build AFT's half of the fleet GitHub identity design (five-seat room, adjourned
2026-08-16; my spec artifact v1.2 accepted as the room's build reference): a
credential-free `gh` ROUTING SHIM that consults a reviewed routing manifest and
routes identity-bearing operations (issue/PR comments, reviews, reactions,
closes, merges) through the governance seam under the calling agent's own GitHub
App identity, while passing every mechanical operation (runs, api reads, clone,
checks) through to real gh byte-transparently.

Governance is staged on three rungs, named R1/R2/R3 throughout this spec (prose
aliases, given once here: R1 = standalone/no-subc, R2 = subc-without-PLEX,
R3 = governed). The tokens R1/R2/R3 name RUNGS exclusively everywhere in this
document; historical chair rulings are cited only as CR-numbers. R1 and R2 are
byte-transparent passthrough with no per-invocation output of any kind; the
manifest only ACTS at R3, where classification is allowlist-driven in both
directions and anything matching neither the governed nor the mechanical
allowlist fails closed with a stable machine refusal code.

The precondition that parked this build has now fired: every fleet Alfonso has
its own GitHub App (per-agent bot handle + avatar), so ambient machine-account
PATs can retire and R3 becomes activatable. The shim closes the bypass class
this repo hit on 2026-08-17 (agent running bare `gh` under the operator's
handle) structurally instead of by discipline: cross-agent attribution on the
governed path is contained arithmetically by custody-scoped App tokens, and the
write fence for undeclared verbs is held by fail-closed classification rather
than by the absence of the operator's own gh login, which the shim leaves
untouched. The containment arithmetic is claimed only for AGENT App tokens; on a
seat that still carries operator credentials, an R1/R2 social-verb passthrough
may succeed under the operator's identity, and the fence claim is scoped to R3.

Read-side output compression is OUT of scope for this campaign entirely (chair
amendment A1). Mechanical passthrough is always exec of real gh with argv
unmodified; the shim never execs `gh api` on an agent's behalf, never
substitutes porcelain with api calls, and never rewrites mechanical output
bytes. Porcelain read compression (issue view / pr view / run list / checks)
moves to a follow-on campaign (owner note #2168) with its own corpus fixtures
and thresholds.

Until the governed routing campaign ships a live seam, an R3 invocation classified as a governed verb is refused with stable code `gh_shim_seam_unavailable`, naming the seam dependency; CORE never executes a governed verb under any ambient credential.

## glossary
- **argv[0] dispatch** — the busybox-style self-detection by which the aft
  binary, when invoked under the name `gh`, enters shim mode without a flag. The
  dispatch check runs BEFORE all existing global argument scans of the binary
  (A6), so the binary's own `--version` scan cannot intercept a shim argv vector.
  No flag forces, suppresses, or reconfigures shim mode.

- **self-report / `--status`** — `aft gh-shim --status` (canonical;
  `--shim-version` is an accepted alias in shim mode) printing shim version,
  schema floor, cached manifest version, the LAST-DETERMINED rung with its
  as-of timestamp and per-input determination record, the resolved PATH position
  of the executing image, and any recorded operator-bypass uses, from the binary
  and caches alone. Dials nothing (CR2); the forensic answer to "was the fence
  installed".

- **connection file** — the standard-path file by which the shim discovers the
  subc daemon, resolved exactly as the AFT plugin resolves its
  `subc.connection_file` user-tier config (read from
  `~/.config/cortexkit/aft.jsonc`; no env override, no walk-up). Its presence or
  absence is exactly the R1-versus-R2 boundary; absent or unparseable is R1.

- **discovery budget** — 150ms total for rung determination (connection-file
  stat plus catalog probe), with the determination cached 15 seconds in the rung
  cache (A6). On budget exhaustion the invocation resolves DOWNWARD to the
  highest rung already proven by the cache, else R1.

- **R1 / R2 / R3 (the degradation ladder)** — the three rungs, one name per
  concept (CR10). Prose aliases, given once: R1 = standalone/no-subc,
  R2 = subc-without-PLEX, R3 = governed. R1 and R2 are byte-transparent
  passthrough with ZERO per-invocation output; the manifest only ACTS at R3.
  R1 vs R2 is exactly connection-file presence.

- **rung cache** — the shim's on-disk record of the last rung determination: its
  per-input results (connection file, catalog probe for `gh.route`, binding
  resolution, custody readiness, and the `agent_credentials_present` arm with
  its offending path), the resulting rung with a timestamp, the manifest version
  consulted, and the 15-second discovery cache. The only place R2's "which input
  failed" lives, and the only place un-governance is surfaced. Per-user and
  durable, never session-scoped: its whole value is being readable by a LATER
  process on a cold seat. The write is best-effort and silent (CR3).

- **`agent_credentials_present`** — the R2 determination input recorded, with
  the offending path, when every other input is ready but an AGENT-scoped
  ambient credential store is still on the seat. The deliberate cutover
  mechanism, not an error state; it never blocks the invocation and emits
  nothing.

- **routing manifest / `gh-routing-manifest`** — the reviewed, versioned, ONE
  fleet-served artifact declaring, per (verb, subcommand, api_match) tuple, an
  action's tier. Carries a platform field so unsupported platforms are visible
  rather than silent, and the CR9 canonicalization for governed tuples.

- **two-way allowlist (CR4)** — the manifest declares BOTH the governed verb
  tuples AND the mechanical allowlist (tuples plus api path/method patterns for
  reads). At R3 the mechanical allowlist is the only thing that authorises
  passthrough. No write-detection heuristics exist anywhere in the shim.

- **canonicalization (CR9)** — the manifest's per-governed-tuple declaration of
  how (target, body) is derived: which argv forms map onto the tuple (flag order
  normalized, repo inferred from the git remote when omitted, explicit `--repo`
  wins) and which body fields are forwarded. The shim performs ONLY this
  declared mapping and does not otherwise parse or validate gh arguments.

- **`GH_SHIM_BYPASS=operator`** — the operator bypass for admin-tier ops. Any
  process on the seat could technically set it; it is made safe not by secrecy
  but by AUDIT (A4). The bypass requires a successful append to the durable
  bypass sink BEFORE the exec; if that append fails the bypass is REFUSED with
  `gh_shim_bypass_audit_unavailable` and the invocation behaves exactly as if
  the variable were unset. There is no ordering in which an unrecorded bypass
  executes. Governed verbs remain seam-refused for agent identities server-side
  regardless of the variable.

- **version handshake / schema floor (`gh_routing_schema_floor`)** — the shim's
  check on a served manifest. Refuses loudly with
  `gh_shim_manifest_below_floor`, naming both versions, when the manifest is
  OLDER than the shim's schema floor (not merely newer-refuses-older).

- **stable refusal code** — a machine-readable identifier minted by every
  fail-closed path. The enumeration is CLOSED (A6): `gh_shim_unclassified`,
  `gh_shim_admin_tier`, `gh_shim_manifest_stale`,
  `gh_shim_manifest_below_floor`, `gh_shim_seam_schema_mismatch`,
  `gh_shim_unbound_identity`, `gh_shim_bypass_audit_unavailable`,
  `gh_shim_no_real_gh`, `gh_shim_seam_unavailable`, and
  `gh_shim_seam_refusal`. PLEX's enumeration suite
  keys on these codes, never on message text. R2 passthrough is not a refusal
  and has no code.

- **ambient credentials (CR8-scoped)** — AGENT credential stores reachable on
  the seat: fleet machine-account PATs in env and agent wrapper config dirs,
  including `GH_CONFIG_DIR`-scoped ones such as the interim wrapper's. Reaching
  `GH_CONFIG_DIR`-scoped directories is load-bearing, not incidental: a scan of
  the default gh config location alone would let a seat activate R3 with a live
  wrapper PAT still on disk. The operator's normal gh login is explicitly NOT
  ambient for this check and never blocks activation. The check inspects ambient
  reachability only, never custodied handles — "a precondition that is always
  false looks identical to one that is never met" (CKCRED).

## constraints
- Room-fold decisions are FIXED inputs, not open questions: (2) Tier is a
  manifest-declared property of the action, enumeration-property-tested against
  blur. (4) Split verbs (`gh pr close` api-form) carry deliberately-empty
  api_match with the body-determined-operation rationale inline (path globs
  cannot see state=closed in a request body).

- R3-with-daemon-down corner, stated honestly (A5): the shim degrades to R1/R2
  byte-transparent passthrough — social verbs included — and no shim-side
  lockout path is invented. With AGENT-scoped ambient credentials gone (the R3
  activation precondition), a passthrough-exec'd social verb carries no agent
  credential, so it can never land as ANOTHER agent; but on a seat that still
  carries operator credentials it MAY succeed under the operator's identity.
  The acceptance criterion is therefore byte-transparency at R1/R2, not
  "passthrough fails authentication". The fence claim is scoped to R3, where
  fail-closed classification refuses unmapped writes, and the
  credential-arithmetic containment claim is scoped to AGENT App tokens
  (custody-held) and is never claimed for operator-credentialed hosts.

- NO READ COMPRESSION IN THIS CAMPAIGN (A1). Mechanical passthrough is exec of
  real gh with argv unmodified: the shim never execs `gh api` on an agent's
  behalf, never substitutes porcelain with api calls, and never rewrites
  mechanical output bytes. The ONLY rendering this campaign ships is for
  GOVERNED-verb responses, which arrive as seam structured data and have no raw
  gh output to preserve. Porcelain read compression (issue view / pr view / run
  list / checks) moves to a FOLLOW-ON campaign (owner note #2168) with its own
  corpus fixtures and thresholds; every sentence elsewhere describing big-four
  compression, corpus byte budgets, or weak-model parse gates for mechanical
  reads is void, and the slice plan carries no compression slices.

- RUNG DETECTION (CR-RUNGDET): the shim does NOT reuse fleet_status.rs's
  env-gated dial (SUBC_MODULE_ID/SUBC_LAUNCH_NONCE are module-launch env a
  shell-invoked shim never inherits — reusing that gate would leave the shim
  permanently dormant at R1). The shim discovers the daemon via the standard
  connection-file path (same discovery as the --subc invocation and
  subc-client), opens its own bind, and determines the rung by catalog probe
  for the governance op. Connection file absent or daemon unreachable = R1/R2
  passthrough. fleet_status.rs is the PATTERN SOURCE (probe for an advertised
  op, dormant when absent), not a code dependency.

- NAMING AND REFUSAL CODES (CR-NAMES, blanket acceptance of the round-0
  continuity findings): one name per concept — rungs R1/R2/R3 with prose
  aliases given once; the seam is verify_identity_assertion; the manifest
  artifact id is `gh-routing-manifest` with schema floor name
  `gh_routing_schema_floor`. Every fail-closed path mints a stable machine
  refusal code for PLEX's enumeration suite to key on. Ruling labels are
  CR-numbers only; R1/R2/R3 name rungs exclusively (A2).

### CR1 - effective-rung state machine (testable, complete)

Determined per invocation, in order, first match wins:
- **R1**: no connection file at the standard path. Passthrough. No note, no
  dial, no state written.
- **R2**: connection file exists, but any of: daemon unreachable, catalog does
  not advertise the governance op, no agent binding resolves for the cwd's
  project root, custody not ready (seam refuses readiness probe), OR — the arm
  completed by A3 — every preceding input is ready but the AGENT-credential
  activation check fails because an ambient agent credential store is still
  present, in which case the rung cache records determination input
  `agent_credentials_present` with the offending path. That last arm is the
  deliberate cutover mechanism (deleting the wrapper credential is the per-seat
  cutover act); it is not an error state and never blocks the invocation. All
  R2 arms are passthrough with ZERO per-invocation output (see CR3), and the
  determination inputs and result are written to the shim's rung cache.
- **R3**: connection file + governance op advertised + binding resolves +
  ambient-credential check passes. Governed routing active.
Distinctions the panel demanded: R1 vs R2 = connection-file presence.
Daemon-reachable-without-governance vs daemon-unreachable are both R2 but the
rung cache records WHICH input failed, so self-report and forensics
distinguish them without inventing rungs.

### CR2 - self-report never dials

`aft gh-shim --status` (canonical; `--shim-version` is an accepted alias in
shim mode) prints shim version, the shim's manifest schema floor
(`gh_routing_schema_floor`), the cached manifest version, the LAST-DETERMINED
rung from the rung cache with its as-of timestamp and the per-input
determination record ("R2 as of 14:02:11: daemon unreachable"), the resolved
PATH position of the executing image, and any recorded operator-bypass uses
(CR5). Zero dials, works with everything down. The rung cache and the bypass
record are per-user and DURABLE, never session-scoped, because their whole
value is being readable by a LATER process on a cold seat. A stale rung in
self-report is honest because it is timestamped; the next real invocation
refreshes the cache.

### CR3 - R2 is byte-transparent, period (the one-time note is dead)

The R2 "one-time note" contradicted byte-transparency and is removed entirely.
Discovery of un-governance lives in `aft gh-shim --status` and the rung cache,
never on a passthrough invocation's stderr. (Config-philosophy: a note nobody
asked for on someone else's command output is noise; the forensic question
"was the fence installed" is answered by self-report.) Every earlier phrasing
of "passthrough with a one-time note" is superseded wherever it appears.

### CR4 - classification is allowlist-driven both ways; unknown is refused at R3

The manifest declares BOTH the governed verb tuples AND the mechanical
allowlist (verb tuples + api path/method patterns for reads). At R3, an
invocation matching neither is refused with stable code
`gh_shim_unclassified` (fail-closed; unknown is never assumed-read). No
write-detection heuristics anywhere. At R1/R2 classification is not consulted
(pure passthrough). Porcelain evolution = manifest update, which the version
handshake already distributes.

### CR5 - one classification for App-incapable ops (closes stay coherent), as corrected by A4

Ops the App identity cannot perform (closeIssue on non-authored issues today)
are declared in the manifest as tier `admin`. At R3 the shim refuses them with
stable code `gh_shim_admin_tier` and text naming the operator bypass. There is
no dual "governed and also administrative" description anywhere in the spec.

Operator bypass with REQUIRED-WRITE semantics (A4): `GH_SHIM_BYPASS=operator`
is an operator convenience that any process could technically set, made SAFE
not by secrecy but by audit. The bypass requires a successful append to the
durable audit sink BEFORE exec; if the audit write fails (sink missing,
unwritable, full), the bypass is REFUSED with
`gh_shim_bypass_audit_unavailable` and the invocation proceeds as if the
variable were unset. Recorded uses are surfaced by `--status` from a later
process — auditable, never silent. Governed verbs remain seam-refused for agent
identities server-side regardless of the variable. (The former justification
that the bypass "only helps whoever holds real credentials" is deleted: it is
falsified while operator credentials remain ambient under CR8.)

### CR6 - manifest cache numbers

TTL 15 minutes. Post-expiry: re-probe; if the fetch fails, serve the cached
manifest up to a 24h staleness grace (stale-but-signed beats no-governance),
refusing only when the cache is older than the grace OR below the shim's
schema floor. Both refusals carry stable codes (`gh_shim_manifest_stale`,
`gh_shim_manifest_below_floor`) with the versions in the text. This closes the
former unreachable-past-TTL open question: mechanical passthrough is
unaffected under every branch.

### CR8 - mechanical ops authenticate as today; the activation check is scoped to AGENT credential stores

Owner correction (dissolves ask_45bf80f7): mechanical operations route to
plain gh and authenticate with whatever the MACHINE already has - the
operator's own gh login, untouched, exactly as today. No custody read-tokens,
no seam relay for mechanical ops. Consequently the R3 ambient-credential
activation check is scoped to AGENT credential stores only: fleet
machine-account PATs in env and agent wrapper config dirs
(gh-alfonso-aft-style GH_CONFIG_DIRs). The operator's normal gh auth is NOT
ambient for this check and never blocks activation.

Honesty consequence the spec must state: with operator credentials present on
the shared user account, the write-fence for UNDECLARED verbs is enforced by
classification (CR4 fail-closed refusal at R3), not by credential arithmetic -
the arithmetic containment claim applies to the GOVERNED path (custody-scoped
App tokens), which is where cross-agent attribution lives. A deliberate
absolute-path bypass by the operator remains possible and is the operator's
prerogative.

Rendering under this ruling, as narrowed by A1: governed responses render from
seam structured data, and that is the ONLY rendering this campaign ships.
Mechanical reads are exec-passthrough with argv unmodified and their output
bytes are never rewritten; R1/R2 output stays untouched by construction (CR3).

### CR9 - canonicalization of the governed request shape

The manifest declares, per governed tuple, the canonicalization of (target,
body): which argv forms map to it (flag order normalized, repo inferred from
git remote when omitted, explicit --repo wins) and which body fields are
forwarded. The shim performs ONLY this declared mapping - it does not
otherwise parse or validate gh arguments. Undeclared argv shapes for a
governed verb head are `gh_shim_unclassified` (CR4), which is exactly the
fail-closed default the api-form split-verb ruling already established.

### CR10 - naming pass (CR-NAMES integration, mechanical, blanket)

One name per concept everywhere: rungs are R1/R2/R3; the seam is
verify_identity_assertion; the manifest artifact id is `gh-routing-manifest`
with schema floor name `gh_routing_schema_floor`. Every refusal named in these
rulings is a stable machine code. This is CR-integration work for the refire
assembler, not a design question.

### A6 - literal identifiers (CORE clauses)

- Standard connection-file path: identical resolution to the AFT plugin's
  subc.connection_file user-tier config (read from
  ~/.config/cortexkit/aft.jsonc; no env override, no walk-up). Absent or
  unparseable config = R1.

- Refusal codes (complete enumeration, all stable): gh_shim_unclassified,
  gh_shim_admin_tier, gh_shim_manifest_stale, gh_shim_manifest_below_floor,
  gh_shim_seam_schema_mismatch, gh_shim_unbound_identity,
  gh_shim_bypass_audit_unavailable, gh_shim_no_real_gh,
  gh_shim_seam_unavailable, gh_shim_seam_refusal (wrapping a seam refusal_code
  passthrough). Rung-2 passthrough is not a
  refusal and has no code.

- Rung determination time budget: 150ms total for discovery (connection-file
  stat + catalog probe, cached 15s in the rung cache); on budget exhaustion
  the invocation resolves DOWNWARD to the highest rung already proven by the
  cache, else R1. Passthrough latency added at R1 (no connection file): one
  stat call.

- argv dispatch: gh-shim dispatch is by argv[0] basename `gh` (symlink) or
  first argument `gh-shim` on the aft binary; the dispatch check precedes ALL
  existing global argument scans of the binary (the --version scan defect is
  named and fenced in the dispatch slice).

## design
### Dispatch and process model

Entry is by argv[0] self-detection: the aft binary dispatches into shim mode when
argv[0]'s basename is `gh` (the symlink) or when the first argument is `gh-shim`.
The two forms MUST classify identically — an acceptance criterion, not an
implementation detail, because the subcommand form is how the shim is tested and
debugged while the symlink form is how it is actually reached.

The dispatch check runs BEFORE all existing global argument scans of the binary.
This ordering is load-bearing rather than stylistic: the binary's current
`--version` scan would otherwise intercept a shim argv vector before shim mode is
entered. That defect is named and fenced in the dispatch slice rather than left
to be discovered by an agent typing `gh --version`.

The symlink points at the stable deploy path rather than an inode, so replacing
the aft binary in place requires no re-link step.

**Real-gh resolution.** Passthrough execs the upstream GitHub CLI resolved by
scanning PATH entries in order and SKIPPING the shim's own entry — matched by
resolved path of the executing image, not by directory-name heuristic, so a
seat with a differently-placed symlink cannot exec-loop back into the shim. The
match must survive the shapes that make path comparison lie: a symlink reached
THROUGH a symlinked parent directory, `/private/var` versus `/var` on macOS, and
container bind mounts — which is why the no-loop property is asserted against
those shapes rather than assumed from the ordinary case. If the scan finds no
real gh, the shim exits with a distinct status and the stable code
`gh_shim_no_real_gh`, naming the scan result; it does not silently succeed.

**Byte-transparency.** Passthrough is exec (process replacement), not
spawn-and-relay, so stdout/stderr/exit status are real gh's unaltered, TTY
detection and pagination behave as they do without the shim, and signals land
on the right process. Mechanical passthrough is exec with argv UNMODIFIED: the
shim never substitutes porcelain with `gh api`, never execs gh on an agent's
behalf, and never rewrites mechanical output bytes (A1). At R1 and R2 this is
absolute: no note, no banner, no stderr of the shim's own on any invocation
(CR3). At R3 the deliberate exceptions are governed verbs (which do not exec gh
at all), admin-tier refusals, and unclassified refusals; every other verb is
byte-transparent. The byte-identity acceptance corpus covers every argv shape
except the two reserved self-report tokens, which are not a delegating
invocation at all (see the short-circuit below). The corpus records the real gh
VERSION its fixtures were captured against and fails loudly on mismatch, so a gh
upgrade re-baselines the corpus as an explicit act instead of surfacing as
phantom shim drift.

### Bash side-gate

The bash permission layer gains a side-gate steering social gh verbs (issue
comment, pr review, pr close, issue close) toward the governed path with naming
text, cat-to-read style. Mechanical gh is untouched. The side-gate respects the
ladder: no steering at R1/R2, where routing does not exist and steering would be
a lie. It matches a command token, so it does not see an absolute-path
invocation of real gh; that residual shape is covered by the credential floor
rather than by the gate.

### Coexistence and cutover

`gh-alfonso-aft` (the interim wrapper) and the prepare-commit-msg trailer hook
keep working unchanged until cutover. The shim replaces the wrapper; it never
touches the co-author trailer machinery, which is out of scope.

The R3 activation precondition is the cutover coordinator; no absorption
machinery or choreographed fleet flip is added. The wrapper's
`GH_CONFIG_DIR`-scoped config directory (`~/.config/gh-alfonso-aft`) contains an
agent PAT and therefore counts as an agent-scoped ambient credential under CR8.
Symlink placement is inert on arrival: at R1/R2 the shim passes through
silently, while the wrapper keeps working under its distinct command name.
Per-seat cutover is deletion of the wrapper and its config directory, which both
retires the legacy path and removes the ambient credential blocking R3. The recorded `agent_credentials_present` determination, surfaced in `--status`
with the offending path, tells any seat exactly which directory is still
blocking it. Fleet coordination is an announcement followed by each seat's own
deletion; a seat that does not delete remains safely at R2 with the wrapper
working and is visibly ungoverned in `--status`.

Detect-and-absorb is rejected: the shim never reads or migrates the wrapper's
credential store, because doing so would add coexistence machinery and move a
PAT through the shim in violation of the credential exclusion. The acceptance
suite asserts the shim never opens that file.

The manifest expresses the bot-App scope split as tier `admin` rather than as
prose (CR5): closeIssue on non-authored issues is outside the App scope, so
those closes are admin-tier — operator-only, with the bypass named in the
refusal — unless the permission set grows.

## interfaces
This section fixes the surfaces this campaign OWNS and names the surfaces it
only CONSUMES. Owned surfaces are normative here; consumed surfaces are
recorded as the contract shape this build codes against, with degradation
behaviour stated for each, because PLEX's seam and CKCRED's custody ship on
their own seats' schedules. Rungs are named R1/R2/R3 throughout (CR10).

### Owned: command-line surface

One executable image, two invocation names, identical classification:

- `gh <args...>` — reached through the symlink in the CortexKit bin directory
  (`~/.local/share/cortexkit/bin/gh` -> the stable aft deploy path). Entry is by
  argv[0] self-detection; no flag is required and none is accepted to force
  shim mode.
- `aft gh-shim <args...>` — the subcommand form, used for testing and debugging.
  `<args...>` are the same argv vector the symlink form would receive.

The two forms MUST produce the same tier decision for the same argv vector;
this is an acceptance criterion, not an implementation convenience. Dispatch is
by argv[0] basename `gh` or by first argument `gh-shim` on the aft binary, and
the dispatch check runs BEFORE all existing global argument scans of the binary
(A6). That ordering is load-bearing, not stylistic: the binary's current
`--version` scan would otherwise intercept a shim argv vector before shim mode
is entered, and that defect is named and fenced in the dispatch slice rather
than left for an agent typing `gh --version` to discover.

**Self-report (CR2).** `aft gh-shim --status` is canonical; `--shim-version` is
an accepted alias in shim mode (`gh --status` / `gh --shim-version`). It prints
shim version, the shim's manifest schema floor (`gh_routing_schema_floor`), the
cached manifest version if present, the LAST-DETERMINED rung from the rung
cache with its as-of timestamp and per-input determination record (e.g. "R2 as
of 14:02:11: daemon unreachable", or `agent_credentials_present` with the
offending path), the resolved PATH position of the executing image, and any
recorded operator-bypass uses (CR5). It is a short-circuit ahead of rung
determination and manifest fetch: no daemon dial, no network, no fetch.
It must succeed with the daemon stopped, the AFT module dead, and the machine
offline, and it must report state written by an EARLIER process, since that is
what makes it a forensic answer rather than a live probe. The acceptance suite
asserts zero dial attempts on this path. A stale rung here is honest because it
is timestamped; the next real invocation refreshes it. The printed PATH position
is also the instrument by which reach is probed from each context the fence must
cover, rather than inferred from a developer shell.

**Reserved argv tokens.** `--status` and `--shim-version` as the FIRST argument
are the only argv shapes the shim intercepts rather than classifies; they are
shim-reserved and are never forwarded to real gh. They are consequently the only
invocations at which the shim emits output at R1/R2 — which is not an exception
to CR3 but a different surface, because CR3 governs invocations that DELEGATE to
real gh and a self-report invocation never delegates. The byte-identity
acceptance corpus therefore excludes these two tokens and asserts every other
argv shape; the suite asserts the exclusion list has EXACTLY those two members,
so a third exclusion cannot accumulate quietly. Real gh's `gh status` SUBCOMMAND
(no leading dashes) is an ordinary mechanical read that passes through untouched
at every rung; the shim never conflates the reserved flag with the subcommand.
No other flag forces, suppresses, or reconfigures shim mode, and should upstream
gh ever adopt a colliding flag name, this reserved-token list is the single edit
point — the acceptance suite asserts the reserved names against gh's own flag
list so an upstream addition fails a test rather than silently shadowing real
behaviour in exactly the place the corpus does not look.

**Corpus baseline.** The byte-identity corpus records the real gh VERSION its
fixtures were captured against and fails loudly on version mismatch, so a gh
upgrade re-baselines the corpus as an explicit act rather than surfacing as
phantom shim drift. Byte-identity is a construction property of exec-based
passthrough; the corpus is how that construction is kept honest across upstream
movement.

**Argument opacity.** Outside the enumerated self-report flags, the
classification match on `(verb, subcommand)` / `gh api` method and path, and the
per-tuple canonicalization the manifest declares (CR9), the shim does not parse,
rewrite, reorder, or validate real gh's arguments. Unknown flags are not an
error condition for the shim on the passthrough path.

**Streams and status.** Passthrough is exec (process replacement): stdout,
stderr, exit status, TTY detection, pagination, and signal delivery are real
gh's, unaltered, with argv UNMODIFIED — the shim never substitutes porcelain
with `gh api`, never execs gh on an agent's behalf, and never rewrites
mechanical output bytes (A1). At R1 and R2 this is absolute — no note, no
banner, no shim-originated stderr on any invocation (CR3). At R3 the enumerated
exceptions are exactly three: governed verbs (which never exec gh at all and
therefore render the seam's structured response), admin-tier refusals, and
unclassified refusals. Every other verb is byte-transparent. There is no
read-compression table in this campaign and no fourth exception.

**Shim-originated exit statuses and stable refusal codes.** The shim reserves a
small distinct band for failures and refusals that are its own or the seam's
rather than real gh's, so a transcript can tell them apart without guesswork.
Each is emitted with a stable machine refusal code — PLEX's enumeration suite
keys on the codes, never on message text — and a message naming the reason. The
enumeration is closed (A6):

- `gh_shim_no_real_gh` — no upstream gh found on PATH, naming the scan result.
- `gh_shim_manifest_below_floor` — served manifest older than
  `gh_routing_schema_floor`, naming both versions.
- `gh_shim_manifest_stale` — cached manifest past the 24h staleness grace,
  naming both versions.
- `gh_shim_unclassified` — at R3, an invocation matching neither allowlist
  (including an undeclared argv shape under a governed verb head), naming the
  manifest version that produced the decision (CR4, CR9).
- `gh_shim_admin_tier` — an op declared tier `admin`, naming the operator
  bypass (CR5).
- `gh_shim_unbound_identity` — the governed path reached with no agent binding
  resolved for the cwd's project root.
- `gh_shim_bypass_audit_unavailable` — `GH_SHIM_BYPASS=operator` set but the
  required audit append failed, so the bypass is refused (A4).
- `gh_shim_seam_schema_mismatch` — the `gh.route` holder answered with a higher
  schema major than `gh_route_schema=1`.
- `gh_shim_seam_refusal` — wrapping a seam-returned refusal_code verbatim.
- `gh_shim_seam_unavailable`.

R2 passthrough is NOT a refusal and has no code; in particular, a seat that
cannot activate R3 because an agent-scoped ambient credential store is still
present is an R2 determination input (`agent_credentials_present`, recorded with
the offending path per CR1/A3), surfaced by `--status`, never an emitted refusal
on an invocation. Real gh's own statuses pass through unmapped on the
passthrough path only.

**Exit-code fidelity is explicitly NOT a goal for governed verbs.** Governed
verbs do not run gh, so there is no gh status to reproduce; the shim maps seam
refusals into its own reserved band with the refusal code on stderr. Callers
that need gh-identical statuses are on the passthrough path by construction,
where fidelity IS asserted, unmapped, for every passthrough verb.

### Owned: routing manifest schema

The manifest is a reviewed, versioned, fixture-shaped artifact with id
`gh-routing-manifest` — the declared-facts target PLEX's enumeration suite runs
against, not a parallel copy. It carries:

- a schema version (checked against the shim's schema FLOOR,
  `gh_routing_schema_floor`) and a manifest version (the cache key and the
  string named in refusals);
- a fleet base declaring BOTH allowlists (CR4): per `(verb, subcommand,
  api_match)` tuple, a `tier` from the closed set {mechanical, governed, admin},
  a `platform` field so unsupported platforms are visible rather than silent,
  and, for split verbs, an inline rationale accompanying a deliberately-empty
  `api_match`. The mechanical allowlist is declared with the same rigour as the
  governed one, because under CR4 it is the only thing that authorises
  passthrough at R3;
- per governed tuple, the CR9 canonicalization of `(target, body)`: which argv
  forms map onto it (flag order normalized, repo inferred from the git remote
  when omitted, explicit `--repo` winning) and which body fields are forwarded.
  An undeclared argv shape under a governed verb head is `gh_shim_unclassified`;
- `api_match`: a closure over `gh api` invocations expressed as method plus path
  glob, mapping an api call onto the same tier as its equivalent porcelain verb.
  An empty `api_match` on a split verb is a declared fact with its
  body-determined-operation rationale attached, because a path glob cannot see
  `state=closed` in a request body; the api form then falls through the
  fail-closed `gh_shim_unclassified` default;
- repo-scoped sections, repo-keyed, INSIDE this same artifact. There is no
  repo-local manifest file in v1.

**Tighten-only is a schema property, not a merge rule.** A repo section may
raise a verb's tier or remove a verb; it may not lower a tier or add a verb.
The schema validator rejects a lowering or adding section outright, so the
one-way-hardening property holds with no merge algorithm and no
downgrade-refusal proof obligation. A tighten-only property test over the
enumerated verb set is an acceptance criterion.

**Enumeration property.** Every declared tuple maps to exactly one tier. Tier is
a property of the ACTION, never of the caller's phrasing. There are no
write-detection heuristics anywhere in the shim; unknown is refused, never
assumed-read.

### Owned: on-disk state (rung cache and bypass record)

The shim owns exactly two pieces of durable state beyond the manifest cache, and
no others. Both live in a per-user location that survives process exit and is
not session-scoped, because their whole value is being readable by a LATER
process with everything else on the seat dead.

- **Rung cache.** The per-input determination record (connection file, catalog
  probe for `gh.route`, binding resolution, custody readiness, and the
  `agent_credentials_present` arm with its offending path), the resulting rung,
  the as-of timestamp, and the manifest version consulted. It also holds the
  15-second discovery cache the 150ms rung-determination budget resolves
  against (A6). Written at R2 and R3 determination; R1 writes nothing at all. It
  is the only place "which input failed" lives, and the only place un-governance
  is surfaced (CR3) — never a passthrough invocation's stderr. The write is
  best-effort and silent: a failure (read-only home, full disk) degrades without
  emitting anything, because zero per-invocation output at R1/R2 is absolute.
- **Bypass record.** Each `GH_SHIM_BYPASS=operator` passthrough is appended to a
  durable sink alongside the rung cache with its timestamp, verb tuple, and repo
  context, so `--status` surfaces it from a later process (CR5). The append is
  REQUIRED-WRITE and happens BEFORE the exec (A4): if it fails, the bypass is
  refused with `gh_shim_bypass_audit_unavailable`. "Auditable, never silent"
  means this record, not a one-off stderr line nobody keeps.

Neither is a network surface, neither is consulted at R1, and neither is
required for the dependency-free passthrough path to work. No once-per-seat
note store exists or is built: the dead R2 note (CR3) has no persistence
surface anywhere in this design.

### Owned: admin-tier surface (CR5, as corrected by A4)

Ops the App identity cannot perform — closeIssue on non-authored issues today —
are declared tier `admin` and refused at R3 with `gh_shim_admin_tier`, the
refusal text naming the operator bypass. Nothing in this spec describes an op as
both governed and administrative.

`GH_SHIM_BYPASS=operator` is an operator convenience that any process on the seat
could technically set; it is made safe not by secrecy but by AUDIT, and the
interface therefore gives the audit write required-write semantics. The bypass
requires a successful append to the durable bypass sink BEFORE the exec. If that
append fails — sink missing, unwritable, full — the bypass is REFUSED with
`gh_shim_bypass_audit_unavailable` and the invocation proceeds exactly as if the
variable were unset, i.e. into the ordinary admin-tier refusal. There is no
ordering in which an unrecorded bypass executes. Recorded uses are surfaced by
`--status` from a later process. Governed verbs remain seam-refused for agent
identities server-side regardless of the variable; the former claim that the
bypass "only helps whoever holds real credentials" is deleted, since it is
falsified while operator credentials remain ambient under CR8.

### Owned: placement

A symlink named `gh` in the CortexKit bin directory pointing at the stable aft
deploy path, not at an inode, so replacing the aft binary in place requires no
re-link step. No second staging, signing, or codesign path is introduced: the
shim revs atomically with the aft release train, which is what dissolves version
skew between shim logic, manifest schema floor, and self-report. Placement is
inert on arrival — at R1/R2 the shim passes through silently — so the symlink can
land fleet-wide ahead of any seat's cutover. Because the macOS hardened-runtime
and notarization ritual is precisely what could normalise argv[0], the dispatch
path is proven against a freshly staged, signed binary rather than a locally
built one.

### Consumed: capability detection and daemon discovery

Rung determination reuses the existing status.line probe pattern as PATTERN
SOURCE (probe for an advertised op, dormant when absent). The daemon is
discovered via the standard connection-file path, resolved exactly as the AFT
plugin resolves its `subc.connection_file` user-tier config (read from
`~/.config/cortexkit/aft.jsonc`; no env override, no walk-up) — absent or
unparseable config is R1 (A6). The module-launch env gate (SUBC_MODULE_ID /
SUBC_LAUNCH_NONCE) is NOT reused and is asserted unused, because a shell-invoked
shim never inherits it. Capability detection is `catalog.list` advertising
`gh.route`. Discovery carries a 150ms total budget (connection-file stat plus
catalog probe, cached 15s in the rung cache); on budget exhaustion the
invocation resolves DOWNWARD to the highest rung already proven by the cache,
else R1. Every probe failure resolves downward toward passthrough, never upward
into governed mode, and the determination inputs are written to the rung cache
so `--status` can say which input failed. At R1 the added passthrough latency is
one stat call.

## acceptance sketch
- Dispatch: the `gh` symlink in the CortexKit bin directory resolves to the aft
  binary and `aft gh-shim` self-detects via argv[0]; invoking it as `gh` and as
  `aft gh-shim` classify identically. The dispatch check is asserted to run
  BEFORE all existing global argument scans of the binary — `gh --version`
  reaches shim classification and is not intercepted by the binary's own
  `--version` scan. Replacing the aft binary in place (deploy path unchanged)
  leaves the symlink working with no re-link step. The dispatch case runs
  against a FRESHLY STAGED, SIGNED macOS binary, not a locally built one, so the
  hardened-runtime/notarization path is proven to preserve argv[0] rather than
  assumed to.

- Reach: the suite invokes the shim from every context the fence must cover —
  interactive login shell, non-interactive bash tool invocation, both plugin
  transports, and a daemon-supervised session with no plugin present — and
  asserts in each that the executing image is the shim (self-report prints the
  resolved PATH position of the image it ran from). A context where real gh wins
  PATH precedence is a FAILING case, not a documentation note.

- On this repo WITHOUT PLEX configured (R2): every gh invocation passes through
  unchanged and emits ZERO shim-originated bytes on stdout or stderr — the
  acceptance suite asserts byte-identity with real gh across the corpus and
  asserts NO note, banner, or first-run message exists on any invocation (CR3).
  Un-governance is discoverable only via `aft gh-shim --status`, which reports
  R2, the as-of timestamp, and WHICH determination input failed (e.g. "daemon
  unreachable" vs "governance op not advertised" vs `agent_credentials_present`
  with the offending path).

- Corpus baseline hygiene: the byte-identity corpus records the real gh VERSION
  its fixtures were captured against and fails loudly on version mismatch rather
  than diffing against a stale baseline — a gh upgrade re-baselines the corpus
  as an explicit act instead of silently reporting shim drift.

- Argv opacity on the passthrough path: mechanical passthrough is exec with argv
  UNMODIFIED — the suite asserts the shim never substitutes porcelain with
  `gh api`, never execs gh on an agent's behalf, and never rewrites a single
  output byte of a mechanical read (A1).

- Reserved-token isolation: `gh --status` and `gh --shim-version` self-report
  without forwarding to real gh, and they are the ONLY argv shapes excluded from
  the byte-identity corpus — the suite asserts the exclusion list has exactly
  those two members, and asserts those two names against real gh's own global
  flag list, so an upstream gh that adopts a colliding flag fails a test instead
  of being silently shadowed. Complementarily, `gh status` (real gh's
  subcommand, no leading dashes) passes through byte-transparently at R1, R2,
  and R3 and is never intercepted as self-report, so the reserved flag and the
  upstream subcommand are provably distinct.

- Rung state machine (CR1): with no connection file at the standard path — or an
  unparseable `~/.config/cortexkit/aft.jsonc` — the shim is R1 (passthrough, no
  dial, no state written, asserted by dial counter); with a connection file
  present but the daemon unreachable, or the catalog not advertising `gh.route`,
  or no binding resolving for the cwd's project root, or custody refusing the
  readiness probe, or an agent-scoped ambient credential still present, the shim
  is R2 and the rung cache records which input failed. Every failure resolves
  DOWNWARD. Discovery is asserted to honour the 150ms budget with a 15s cached
  determination, resolving downward to the highest cache-proven rung (else R1)
  on exhaustion. The module-launch env gate (SUBC_MODULE_ID / SUBC_LAUNCH_NONCE)
  is asserted UNUSED — a shell-invoked shim with those vars unset still reaches
  R3.

- Dependency-free passthrough: with the daemon stopped AND the AFT module dead,
  mechanical verbs still pass through (argv[0] detect -> cached manifest read ->
  classify -> exec real gh resolved by PATH scan that skips the shim's own
  entry, matched by resolved image path, with no exec loop). The no-loop
  assertion is exercised with the symlink reached THROUGH A SYMLINKED PARENT
  DIRECTORY, so resolved-path matching is proven rather than assumed. A seat with
  no real gh on PATH gets `gh_shim_no_real_gh` naming the scan result, never a
  silent success.

- Offline self-report (CR2): `aft gh-shim --status` (and the `--shim-version`
  alias, and both through the `gh` symlink) prints shim version, schema floor,
  cached manifest version, last-determined rung with timestamp and per-input
  record, the resolved PATH position of the executing image, and recorded
  operator-bypass uses, from the binary and caches alone — verified with the
  daemon down, the module dead, and no network; the test asserts zero dial
  attempts.

- State durability across processes: one process determines a rung and exits;
  a SECOND, fresh process with the daemon down reads that rung, its timestamp,
  and its per-input record out of `--status`. Likewise `GH_SHIM_BYPASS=operator`
  is used in one process and found in `--status` from a later one. An ephemeral
  or session-scoped cache fails these cases — "was the fence installed" must be
  answerable on a cold seat.

- Silent-write property: with the rung cache location unwritable (read-only
  home), an R2 invocation still passes through byte-identically and emits ZERO
  shim-originated bytes; the cache write degrades silently (CR3 is absolute).

- Governed-mode daemon-down corner (A5): a social verb degrades to R1/R2
  byte-transparent passthrough — the asserted criterion is BYTE-TRANSPARENCY,
  not "passthrough fails authentication". The suite asserts no shim-side lockout
  path exists, and asserts the honest scope: with agent-scoped ambient
  credentials gone (the R3 activation precondition) the passthrough carries no
  agent credential and can never land as ANOTHER agent, while on a seat still
  carrying operator credentials it MAY succeed under the operator's identity.

- R3 activation is withheld with a recorded reason (`agent_credentials_present`
  plus the offending path in the rung cache and `--status`) while an
  AGENT-SCOPED ambient credential exists (fleet machine-account PAT in env, or
  an agent wrapper config dir), and activates after it is removed; the
  invocation itself is unaffected and emits nothing (CR1/A3). The ambient scan
  reaches `GH_CONFIG_DIR`-scoped config directories, asserted with the DEFAULT
  gh config location clean so a scan that only inspects the default location
  fails the case. Complementary negative case (CR8): with ONLY the operator's
  normal gh login present, activation is NOT blocked — the operator's auth is
  not ambient for this check.

- Cutover corner: with `gh-alfonso-aft`'s config dir present, R3 does not
  activate and `--status` NAMES that directory as the blocking ambient source;
  deleting the wrapper and its config dir both retires the legacy path and
  activates R3, with no absorption step and no shim read of the wrapper's
  credential store (asserted: the shim never opens that file).

- Split-verb probe: bare `gh pr close N` gets the probe-and-refusal naming the
  declared action; the api body-form (`gh api ... -f state=closed`) falls
  through the generic `gh_shim_unclassified` default (fail-closed) rather than
  through bespoke code — the manifest's empty `api_match` plus rationale is a
  declared fact, not a special-case branch.

- Admin tier (CR5, as corrected by A4): an op declared tier `admin` (closeIssue
  on a non-authored issue) refuses at R3 with `gh_shim_admin_tier` and text
  naming the operator bypass; `GH_SHIM_BYPASS=operator` produces passthrough
  only AFTER a successful append to the durable bypass sink, visible in
  self-report from a later process. With the sink unwritable or missing, the
  bypass is REFUSED with `gh_shim_bypass_audit_unavailable` and the invocation
  behaves exactly as if the variable were unset — the suite asserts no ordering
  exists in which an unrecorded bypass executes. No spec text or manifest entry
  describes an op as both governed and administrative.

- Manifest enumeration test: every declared verb tuple maps to exactly one tier
  across the closed set {mechanical, governed, admin}; an invocation matching
  NEITHER the governed nor the mechanical allowlist in R3 refuses with
  `gh_shim_unclassified` and the manifest version in the text; the suite asserts
  no write-detection heuristic exists (an unknown WRITE and an unknown READ take
  the identical refusal path). Known-hard cases ride as expected fail-closed
  entries: a `gh alias set` expansion of a social verb, and `gh api --input -`
  with a stdin body.

- Manifest distribution: the shim consumes ONE fleet-served artifact
  (`gh-routing-manifest`); a repo-scoped section that raises a verb's tier or
  removes a verb validates, while a section that lowers a tier or adds a verb is
  rejected by the SCHEMA VALIDATOR outright (tighten-only property test over the
  enumerated verb set, no merge algorithm exercised).

- Manifest cache/handshake (CR6): cache is keyed by manifest version; a tier
  change published centrally takes effect within the 15-minute TTL; past expiry
  with the fetch failing, the cached manifest is served up to the 24-hour
  staleness grace and refuses beyond it with `gh_shim_manifest_stale`; a served
  manifest OLDER than `gh_routing_schema_floor` is refused loudly with
  `gh_shim_manifest_below_floor`, and both refusals name both versions. Under
  every branch mechanical passthrough is unaffected — asserted explicitly.

- Refusal-code enumeration: every fail-closed path emits its stable machine code
  from the closed A6 set — `gh_shim_unclassified`, `gh_shim_admin_tier`,
  `gh_shim_manifest_stale`, `gh_shim_manifest_below_floor`,
  `gh_shim_seam_schema_mismatch`, `gh_shim_unbound_identity`,
  `gh_shim_bypass_audit_unavailable`, `gh_shim_no_real_gh`,
  `gh_shim_seam_unavailable`, and `gh_shim_seam_refusal` — and PLEX's suite
  keys on those codes rather than on
  message text. The suite asserts R2 passthrough emits NO code, because it is
  not a refusal.

- Byte-transparency across the whole campaign: with no read compression shipped
  (A1), ALL non-governed output is byte-identical to real gh's over the
  acceptance corpus, and R1/R2 output stays byte-identical permanently by
  construction. The suite asserts no compression renderer, byte-budget gate, or
  weak-model parse gate exists in this campaign's surface; porcelain read
  compression is a follow-on campaign (owner note #2168) with its own fixtures.

## non-goals
- No use of the module-launch env gate (SUBC_MODULE_ID / SUBC_LAUNCH_NONCE) for
  rung detection, and no code dependency on `fleet_status.rs` — it is the
  PATTERN SOURCE only. Discovery is the standard connection-file path, with no
  env override and no walk-up.

- No standalone shim binary and no setup-installed shim script: the shim is an
  aft subcommand reached by symlink, so we do not maintain a second
  staging/signing/placement path or a second version line.

- No repo-local manifest file (.cortexkit/gh-routing.jsonc) and no fleet-base +
  repo-overlay layering with tier-downgrade refusal in v1 — documented
  deliberate exclusion, held in reserve and to be built only if central
  repo-scoped sections prove unwieldy in practice. Rationale carried in the
  spec: two distribution channels, a merge algorithm, and a downgrade-refusal
  proof obligation, all defending against a party (the repo) that is already
  fleet-controlled in governed mode.

- No merge algorithm for repo-scoped sections: tighten-only is enforced by the
  SCHEMA VALIDATOR rejecting a lowering or adding section outright, not by a
  runtime combination step.

- No push semantics for manifest distribution: propagation is the CR6 TTL, with
  the staleness grace and the schema-floor handshake as the only other branches.

- No shim-side lockout or refusal mode for the governed-but-daemon-down corner
  (A5): passthrough is the correct behaviour, byte-transparency is the asserted
  criterion, and the acceptance suite asserts no lockout path exists.

- No detect-and-absorb of gh-alfonso-aft's config directory and no automated
  migration of its credential store: cutover is a per-seat deletion named by the
  recorded `agent_credentials_present` determination in `--status`, and the suite
  asserts the shim never opens that file. Reading another tool's credential store would also move a PAT
  through the shim, which the credential exclusion forbids. No choreographed
  fleet flip is built; a seat that never deletes stays safely at R2.

- No read-side output compression in this campaign at all (A1): no big-four
  renderer (issue view / pr view / run list / checks), no corpus byte budgets,
  no weak-model parse gates, no default-on compression knob, and no compression
  slices in the plan. Mechanical passthrough is exec of real gh with argv
  UNMODIFIED — the shim never substitutes porcelain with `gh api`, never execs
  gh on an agent's behalf, and never rewrites a mechanical output byte. The only
  rendering shipped is for governed-verb responses, which arrive as seam
  structured data and have no raw gh output to preserve. Porcelain read
  compression is a follow-on campaign (owner note #2168) with its own fixtures
  and thresholds, and every earlier sentence describing it as in-campaign is
  void.

- No Windows shim in v1 (fleet Alfonsos run macOS/Linux; Windows is a
  documented gap with the manifest carrying a platform field so the gap is
  visible, not silent).

## open_assumptions
This section records beliefs this campaign TAKES AS TRUE without having
verified them, distinct from `open_questions` (dispositions not yet decided;
currently empty). Each entry names the assumption, what breaks if it is false,
and the cheap check that would settle it. Several of the FIXED decisions —
room-fold rulings and chair rulings CR1-CR10 and amendments A1-A7 alike — rest
on assumptions listed here; naming them is not reopening those decisions, it is
recording precisely what would have to be false for one to need revisiting.
Rungs are named R1/R2/R3 throughout (CR10).

### Placement and dispatch

- **PATH precedence holds in every context the shim must cover.**
  `~/.local/share/cortexkit/bin/` is assumed to precede real gh on fleet PATHs
  not only in interactive login shells but also in non-interactive bash tool
  invocations, daemon-supervised sessions with no plugin present, and both
  plugin transports. If false in any of those, the fence is silently absent
  exactly where the 2026-08-17 bypass shape lives. Cheap check: have
  `aft gh-shim --status` print the resolved PATH position of the executing
  image, and probe it from each context in the acceptance suite rather than
  from a developer shell only; a context where real gh wins is a failing case.
- **Nothing on a governed seat invokes gh by absolute path.** Hooks, wrapper
  scripts, MCP servers, and other tooling are assumed to call bare `gh`. A
  caller that execs `/opt/homebrew/bin/gh` or `/usr/bin/gh` bypasses the shim
  entirely, and the bash side-gate cannot see it because the side-gate matches
  a command token. This is the residual hole in the structural closure claim,
  and it is the same hole the containment-honesty ruling (CR8) already states:
  the credential floor (agent-scoped ambient credentials gone before R3
  activates) is what keeps it from being a cross-agent identity bypass rather
  than merely an inconvenience. Cheap check: grep fleet hook/script surfaces
  for absolute gh paths at cutover.
- **argv[0] dispatch survives the macOS signing ritual.** A signed,
  hardened-runtime aft binary reached through a symlink under a different name
  is assumed to launch normally and to see the symlink name in argv[0]. If
  Gatekeeper, notarization, or a launcher normalises argv[0] to the real path,
  self-detection fails and the busybox form needs a fallback trigger. Cheap
  check: run the acceptance dispatch test on a freshly staged, signed macOS
  binary, not on a locally built one.
- **The dispatch check can in fact be moved ahead of the binary's existing
  global argument scans.** A6 fixes that ordering, and the `--version` scan
  defect is named and fenced in the dispatch slice; the assumption is that no
  earlier scan is load-bearing for ordinary aft invocations in a way that
  reordering would disturb. If one is, the dispatch slice grows a
  compatibility case rather than a redesign. Cheap check: the acceptance case
  asserting `gh --version` reaches shim classification while `aft --version`
  still behaves as today.
- **The CortexKit bin directory is writable at setup time on every seat** (no
  root requirement, no MDM restriction on symlink creation in the user data
  dir). If false, placement needs a second story and the "no second
  staging/placement path" property of the fixed artifact ruling is weakened.
- **The executing image's own PATH entry is identifiable by resolved path on
  every seat.** Real-gh resolution skips the shim's entry by matching the
  resolved path of the executing image rather than by directory-name
  heuristic. This assumes the resolution is reliable under symlinked home
  directories, `/private/var` vs `/var` prefixes on macOS, and container bind
  mounts. If it is not, an exec loop is possible where a `gh_shim_no_real_gh`
  refusal was intended. Cheap check: acceptance case with the symlink reached
  through a symlinked parent directory.
- **exec-based passthrough preserves stdout/stderr/exit status, TTY detection,
  pagination, and signal delivery on both supported platforms.** Byte
  transparency at R1/R2 is asserted against real gh, so if process replacement
  ever has to become spawn-and-relay on some platform, the byte-identity
  criterion becomes a re-derivation rather than a construction.
- **Real gh's output is stable enough for byte-identity to be a testable
  property.** The byte-identity corpus compares against whatever gh version the
  seat has installed, so the criterion assumes a pinned or recorded gh version
  rather than a moving target; a gh upgrade re-baselines the corpus rather than
  falsifying the shim. Cheap check: record the gh version in the corpus
  fixtures and fail loudly on version mismatch instead of diffing against a
  stale baseline.
- **Upstream gh defines no `--status` / `--shim-version` global flag today.** The
  reserved argv tokens are the only shapes the shim intercepts, and they are the
  only exclusions from the byte-identity corpus — so if upstream adopts a
  colliding flag, the shim shadows real behaviour in exactly the place the
  corpus does not look. The reserved-token list is the single edit point. Cheap
  check: assert the reserved names against gh's own flag list in the acceptance
  suite, so an upstream addition fails a test rather than silently shadowing;
  complementarily, assert `gh status` (the subcommand) passes through untouched
  at every rung.

### Identity and credentials

- **The activation check's ambient scan can in fact reach GH_CONFIG_DIR-scoped
  config directories.** The cutover ruling depends on it: the wrapper's config
  dir is only a cutover coordinator if the scan finds it and records it as
  `agent_credentials_present` with the offending path (CR8/A3). If the scan
  inspects only the default gh config location, a
  seat can activate R3 with a live wrapper PAT still on disk — the exact
  ambient credential the floor is supposed to have removed. Cheap check: the
  acceptance case that places a wrapper config dir with the default location
  clean, plus its complement (operator login only, activation NOT blocked).

- **Deleting `gh-alfonso-aft` and its config dir breaks nothing else.** Assumed
  no hook, script, or alias reaches into that directory for anything but the
  wrapper's own use. Cheap check: grep at cutover, same sweep as the absolute-
  path check above.

- **The precondition really has fired fleet-wide.** Every Alfonso is assumed to
  have a provisioned App handle AND to have no remaining task that requires an
  agent-scoped ambient PAT (https clone credentials, rate-limit headroom, a
  mechanical API call the App cannot make). Any such residual need keeps an
  agent credential on the seat, and the invocation then correctly resolves to
  R2 there — governed mode simply never turns on. Cheap check: per-seat ambient-
  credential audit before cutover, treating an `agent_credentials_present` R2
  determination as a finding rather than a failure, with the wrapper's config
  dir as the EXPECTED first finding.

- **Mechanical auth on a governed seat is genuinely the operator's own login
  (CR8).** Assumed that after agent-scoped ambient credentials are removed, the
  machine still has SOME working gh auth for mechanical verbs. If a seat has
  none, mechanical reads start failing auth at GitHub after cutover; that is a
  seat-provisioning finding, not a shim behaviour, and the shim reports it as
  real gh's own unmapped status.

### Classification and manifest

- **`(verb, subcommand)` plus `gh api` method-and-path-glob is a COMPLETE
  classification surface for the enumerated set.** Assumed absent or
  fail-closed-covered: user-defined `gh alias set` expansions (which expand
  inside real gh, AFTER the shim classified, so an aliased social verb
  classifies as unmapped), `gh api --input -` bodies read from stdin, and flag
  forms that relocate the operation out of the matched positions. Fail-closed
  (`gh_shim_unclassified`, CR4) is the safety net for all of these at R3; the
  assumption is that the net is rarely hit, not that the surface is exhaustive.
  Both known-hard cases ride the enumeration suite as expected fail-closed
  entries.
- **The CR9 canonicalization set covers the argv forms agents actually type.**
  Declared forms are permuted flag order, inferred `--repo`, and explicit
  `--repo`. Undeclared shapes refuse rather than being guessed at, which is
  correct but only tolerable if the declared set is the common set. If agents
  routinely hit refusals on ordinary phrasings, the manifest gains declared
  forms — a manifest edit, not a code change. Cheap check: sample real agent
  gh invocations from transcripts before locking the declared forms.
- **Repo inference from the git remote is unambiguous where governed verbs are
  issued.** The canonicalization declares repo-inferred-when-omitted; this
  assumes a single relevant remote (no fork-plus-upstream ambiguity that would
  make the inferred target differ from the one the agent meant). If ambiguous,
  the safe form is to require explicit `--repo` for that tuple — again a
  manifest edit. Cheap check: exercise the inference in a fork checkout
  carrying both a fork and an upstream remote.
- **A served-manifest channel exists or can be reached without new machinery.**
  Fetch is assumed to ride an existing route. If no such channel exists at
  build time, v1 ships with a locally staged manifest and the cache-read path
  becomes the only reader — which the dependency-free path already requires, so
  this degrades rather than blocks.
- **The CR6 numbers are right for this fleet.** A 15-minute TTL is assumed
  sufficient for tier-change propagation (no push semantics), and a 24-hour
  staleness grace is assumed to be the right trade between stale-but-signed
  governance and refusing outright. If a tightening ever needs to land faster
  than the TTL, the fetch boundary is the single edit point; the numbers are
  fixed inputs, not derived ones.
- **Tighten-only is expressible in schema, not merely checkable in code.** The
  fixed manifest ruling's "no merge algorithm" property depends on a validator
  rejecting a lowering or adding repo section outright. If the schema language
  cannot express the constraint against the fleet base, tighten-only becomes a
  validator rule with a proof obligation — still correct, but no longer
  free-by-construction, and the tighten-only property test carries more weight.

### Capability detection and degradation

- **The status.line probe pattern is reusable for rung determination and its
  failure mode is unambiguous.** Assumed that probe failure is distinguishable
  from probe-says-ungoverned, so downward resolution is a decision rather than
  an accident, and that probing does not itself require the AFT module alive
  (the dependency-free path must not depend on it).
- **The 150ms discovery budget is achievable on a cold seat.** A6 fixes the
  budget as connection-file stat plus catalog probe with a 15-second cached
  determination. Assumed a first-invocation-of-the-day probe fits inside it on
  a loaded machine; if it routinely does not, seats resolve downward to R1/R2
  under load and governed mode becomes intermittent rather than absent, which
  is the worse failure to diagnose. Cheap check: measure cold-cache
  determination latency on the slowest supported seat class and confirm the
  rung cache records the exhaustion path.
- **The rung cache has a durable, writable location that survives process exit
  and is not wiped per session.** CR2's non-dialing self-report is only
  forensically useful if the last determination — rung, timestamp, and
  per-input record — is still there when someone asks. If the cache is
  ephemeral, `--status` on a cold seat reports nothing and the "was the fence
  installed" question loses its answer. Cheap check: acceptance case that
  determines a rung in one process, exits, and reads `--status` in a fresh one
  with the daemon down.
- **The operator bypass has a durable, self-reportable sink of its own.** CR5
  and A4 require `GH_SHIM_BYPASS=operator` passthrough to be recorded BEFORE
  the exec and visible in self-report; that assumes a record alongside the rung
  cache rather than a one-off stderr line nobody keeps, and assumes the env var
  name collides with nothing on fleet seats. If no durable sink exists, the
  bypass is simply refused with `gh_shim_bypass_audit_unavailable` rather than
  becoming silent — the required-write semantics make the assumption's failure
  safe rather than invisible. Cheap check: acceptance case that uses the bypass
  in one process and finds it in `--status` from a fresh one, plus its negative
  with the sink unwritable.
- **Writing the rung cache never becomes an output or a failure on a
  passthrough invocation.** R1 writes no state and R2 writes the determination
  record; the assumption is that a cache write failure (read-only home, full
  disk) degrades silently rather than emitting stderr, since ZERO
  per-invocation output at R1/R2 is absolute (CR3).
- **A seat left at R2 indefinitely is acceptable.** The cutover ruling makes
  non-deletion a stable state rather than a failure. Assumed that a visibly
  un-governed seat is tolerable for as long as it takes, and that `--status` is
  where anyone looks to find out.

### Evidence currency

- Evidence was gathered at `d0e8323de301`; HEAD is now `7d6e99279dd1`, and none
  of the files cited in evidence changed in that span. The cited surfaces —
  subc client linkage, the CortexKit bin path and its PATH position, the
  status.line probe pattern, the bash permission layer's gating shape, and the
  interim wrapper's config-dir layout — are therefore assumed current as of
  HEAD. Any of them moving invalidates the corresponding assumption above, not
  the fixed rulings that cite them as grounds.

## open questions
- none: closed by chair rulings CR1-CR10 and fold-v4 chair amendments A1-A7 (constraints).
