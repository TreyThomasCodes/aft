# GitHub Integration Product Map — v8 (final: room-settled, probe-executed, verdict-closed)

Room rm_toolu_01Wz1a8EKNJAF98Pys3G8sDC, 2026-08-27: five questions settled in
round one (seq 10-12), three structural additions in round two (seq 16-18),
PLEX deliverables + CKCRED's third status specimen folded (seq 20-21).

## The generalization test (CKCRED, verbatim design rule)

"A ceremony that looks rigorous and defends nothing is worse than no ceremony
— it stops people asking." The test that catches it: state, PER SHAPE, what
the mechanism defends against. Inherited doctrine survives that question or
it does not. Applied twice in this room (approval-before-signature; ask-first
approval) — apply it to every ceremony a consumer shape inherits from the fleet.

Goal: when Alfonso client apps arrive, GitHub integration is UI + wiring only.
Every mechanism decision is settled here first.

## The three shapes

| | S1: per-agent App | S2: one fleet/user App | S3: no App |
|---|---|---|---|
| Identity on the wire | `<agent>-alfonso[bot]` per seat | one `alfonso[bot]` for everything | user's own account |
| Who it's for | multi-agent fleets (us today) | consumers with 1..n agents | default; everyone on day one |
| Setup cost | App per agent + manifest rows | ONE App install click + auto-manifest | zero |
| Shim behavior | R3 governed routing | R3 governed routing (same mechanics, one agent id on all rows) | R1/R2 byte-transparent passthrough |
| Status today | LIVE (v7, 22 seats, 26 repos) | mechanically supported, never exercised | live since day one |

Key mechanism fact: the shim has NO mode switch. The rung ladder detects what
exists (manifest? daemon? holder? ambient creds?) and resolves per invocation.
S2 is purely a manifest shape (all rows -> one agent id), not new code.

## Parties, and how they collapse for consumers

Fleet topology (today): AFT (shim + manifest verification) · prefrontal/ALF
(gh.route holder, agent registry) · CKCRED (key custody, App PEMs, signing
ceremony, audit chain) · PLEX (GitHub event ingestion/digests).

The four-party split exists because 22 agents share one machine and no agent
may be able to extract a key another identity depends on. A consumer client
has no such adversary separation: **the client app can be its own holder and
custody** (holds the App key in the OS keychain, mints installation tokens,
answers the holder function in-process). The manifest signature's role
degrades from authority to integrity (tamper-detection on the local cache) —
already documented at the trust-set site in gh_shim.rs.

RULING (Q1, unanimous): the consumer client EMBEDS holder+custody — App key in
the OS keychain under app-scoped access, tokens minted in-process, offline-
capable. Hosted mint is a later revocation option, never the v1 dependency.
Three conditions attached:
- ALF: collapse the PARTIES, keep the WIRE SHAPE — the embedded holder speaks
  `gh_route_request_v1` internally, byte-identical to the fleet seam, so
  S1<->S2 upgrades are a manifest swap and a hosted mint can slot in later
  without touching callers.
- PLEX: the mint path must resolve fresh per call and FAIL LOUD — never
  silently serve a stale token; 401-vs-403 semantics preserved.
- CKCRED (the deep one): APPROVAL-BEFORE-SIGNATURE DOES NOT SURVIVE THE
  COLLAPSE. The fleet property rests on a gate asymmetry (approval is
  master-key-gated; signing needs only a capability handle) that dies when one
  party holds both — an embedded approval entry is a note the signer wrote to
  itself in a log it can rewrite. Ruling: the ordering stays FLEET doctrine
  (S1); in S2/S3 the ceremony DOWNGRADES EXPLICITLY — the signature defends
  on-disk manifest integrity (malware / bad sync / stale file, caught via
  keychain-scoped key), NOT approval provenance, and no consumer is walked
  through an approval step whose forensic value is self-attestation. Anyone
  wanting fleet-grade provenance in a consumer product adds a SECOND PARTY
  (hosted approver), not a second log.

TWO FURTHER Q1 RULINGS (round two):
- ONE VALIDATOR (SUBC, from the live failed probe) — WITH CKCRED'S REFINEMENT
  (their own surface produced the third specimen within 20 minutes of the rule
  being named: `credential.status` ready=true while `credential.get` refused,
  because `ready` read plaintext state while the verb also gated on a field
  inside the sealed envelope): sharing a validator FUNCTION is necessary but
  not sufficient — the status surface must be able to SEE every input the verb
  gates on, or the cheap caller passes different arguments and gets a
  different answer from the same code. Consumer test form: assert the
  indicator and the action AGREE on every broken fixture. Both fleet
  divergences were found by DRIVING the surface, never by reading it — a
  status surface with no instrument that can call it goes unexercised on
  every deploy. Status surfaces MUST derive from the enforcement path's
  validator — never a parallel arm. The
  specimen: settings-equivalent status said "valid" seconds before the verb
  arm refused the same artifact's key (multi-image shared state made it look
  like one binary lying; a consumer app with a parallel status arm produces
  "Connected ✓ while every post fails" by construction). Corollary shipped
  fleet-side: status/rung records carry image + trust-era provenance.
- EMBEDDED CUSTODY PORTING NOTES (CKCRED, source-verified): the fleet engine
  has NO fallback-to-stale-token arm — both refresh-failure arms return Err
  (invalid_grant latches needs_reauth; transient failures leave the record
  active and STILL fail the call, so a provider rotation self-heals into
  invalid_grant next attempt). Embedded custody must keep exactly this shape:
  "helpfully" serving the old token on transient failure converts self-healing
  into permanent silent failure. The one uncoverable case — a token revoked
  earlier than the clock believes — requires the report-auth-failure path to
  exist in the embedded shape too (the app is both reporter and custodian).

Per-shape answer to "what does the signature defend against":
- S1: a compromised/buggy shim widening its own routing without custody
  approval (real separation; signature load-bearing as built).
- S2/S3: tampering with the on-disk manifest by anything OTHER than the app.
  Smaller property, still real, ceremony sized to it.

## Setup path per shape (target UX)

- S3: nothing. Must stay literally invisible — no prompts, no doctor steps.
- S2: "Connect GitHub" in the client -> GitHub App install page (one click,
  user picks repos) -> client detects installation -> auto-mints + signs local
  manifest -> done. Acceptance bar: under a minute, zero terminal.

DELIVERED (seq 21): the five-step wire-driven connect ceremony (zero
terminal, idempotent per-step, every refusal maps to a wording-table class):
1. issue_ticket — **TRAP A (bold by room ruling): the ticket lives ~60s and
   is single-use; mint only when step 2 runs in the same breath — a slow user
   interaction between mint and connect bricks the flow by design.**
2. connect {app_key, ticket_id} — discovery "unverified" is NOT an error
   (render connected-pending-first-use).
3. grant connection_admin — TRAP B: MUST precede configure (no implicit owner
   privilege; connection_admin_required otherwise).
4. configure write fence from the SAME repo list the install poll returned —
   never a wildcard.
5. grant invoke to the agent principal (+ poll grant, until-revoked default).
Optional: allow-policy for reaction-tier writes; comment-tier stays ask-first
for the approval card.

RULING (Q2): detection is CLIENT-LOCAL — a bounded poll of
`GET /app/installations` with the freshly minted App JWT while the user is on
the install page (seconds). PLEX has NO inbound webhook surface by
load-bearing design (no exposed ports), and its 300s watch cadence fails the
under-a-minute bar anyway; PLEX remains the fleet's ongoing-events plane only.
The whole connect ceremony is wire-driven and programmable by a client today;
PLEX contributes the exact call sequence with two annotated traps (ticket TTL
~1min => mint-and-connect in one breath; connection_admin before configure).
The manifest-conversion redirect returns App id + PEM in one exchange —
fleet-proven 22 times in one day.
- S1: S2 repeated per agent, or a bulk flow. Fleet-only until proven needed
  for consumers.

## Failure UX (today operator-grade; consumer language needed)

| Refusal today | Consumer wording needed |
|---|---|
| `governance_unavailable` | "GitHub posting is paused (service reconnecting). Retry in a moment." |
| ambient-credential ambiguity | "You have a personal GitHub token set (GH_TOKEN); unset it or unlink this repo so posts aren't ambiguous." |
| `admin_tier` refusal | "This action (merge/release) uses your own GitHub account by design." |
| unclassified verb | "This gh command isn't in the routed set; it ran under your account." (or silent?) |

Round-two specimens for the table (SUBC, from the live refusal):
- The mechanical-read notice "manifest invalid (...); executing with ambient
  gh credentials" is success-shaped for a consumer: "executed as YOU, not as
  your agent" IS the attribution failure and must read as one, with the fix
  named.
- "refused until the manifest is repaired" names no remedy ACTOR (repaired by
  whom?) — fleet-tolerable, consumer-table must carry the actor per string.

DELIVERED (seq 21): PLEX's seven-class wording table, census-derived from
~90 wire codes. A class PATTERN is the ruling for every code in it — client
teams do not invent per-code strings outside the patterns:

1. AUTHORITY ANSWERS (retrying wrong; actor = user via settings): "Your agent
   isn't allowed to do that yet. Allow it in Settings > GitHub > Permissions."
   Never render grant/ceiling/selector/principal/policy vocabulary — the
   settings screen owns it. Exception row: secret_input_rejected is an agent
   bug — "Something tried to include a credential in a request. Blocked.
   Nothing was sent." (reassurance-shaped: the block WORKED).
2. APPROVAL FLOW (most-hit; actor = human, in-app): parking is NOT an error —
   "Waiting for your approval" + the approval card (see-what-you-approve, two
   clicks). Never error styling. approval_expired asks for a fresh attempt;
   decision-plane faults say "your request is safe and waiting."
3. FAULTS (retry right; actor = nobody): "GitHub connection hiccup. Retrying
   automatically." THE ONE EXCEPTION, non-negotiable: outcome_unknown on a
   WRITE — "We can't confirm whether that went through. Check the page before
   retrying." Auto-retry FORBIDDEN in wording and mechanism (half-posted +
   auto-retry = double post; ALF's effect-dedup machinery exists entirely
   because of this class). The only code whose honest wording is uncertainty.
3b. RATE LIMITS (ALF; wait-shaped, distinct from retry-now AND safety-pause):
   rate_limited / any retry_after reply — "GitHub is rate-limiting your agent.
   It will resume on its own — nothing needed." Honor retry_after
   mechanically; NEVER surface a manual retry button (an invitation to make
   it worse).
3c. REQUEST-BOUNDS (CKCRED; actor = client, remedy = ASK FOR LESS, never
   wait-and-retry — fixed compile-time caps fail identically forever, and
   ttl_unsatisfiable burns a real vendor mint per retry): too_many_items /
   sign_payload_too_large = Class-7 telemetry in practice;
   ttl_unsatisfiable = "That needs a credential that stays valid longer than
   this one can. Nothing to fix here — report it."
4. CREDENTIAL/CONNECTION (actor = user; remedy = reconnect): "GitHub needs to
   be reconnected. Settings > GitHub > Reconnect." Rows added by review:
   - UNINSTALL (PLEX, measured on the fleet's biggest mint consumer, 59
     watches): uninstalling the App latches needs_reauth on FIRST contact —
     the agent pauses cleanly and spends ZERO further vendor mints (pre-latch,
     every backoff retry bought a futile App JWT against the shared budget).
     Wording: "GitHub access was removed. Your agent has paused."
   - REINSTALL (CKCRED correction, source-checked — the doc must NOT promise
     button-free resume): a latched record NEVER self-clears (get fails closed
     before any refresh path; that early return IS the latch working). Two
     recoveries that read identically from outside and are different code
     paths: a STALE MARK on an active record self-heals next call (~1.6s —
     PLEX's real measurement, of the OTHER path); a LATCH requires explicit
     reactivate. Consumer shape: reinstall maps to a RESUME control calling
     reactivate — safe to press wrongly (vault re-verifies on next use; a
     wrong press costs one failed request). Sanctioned enhancement:
     DETECTION-TRIGGERED auto-resume (the client's own install poll detecting
     the reinstall fires reactivate once per detection) — never
     timer-triggered, which re-enables the retry loop the latch exists to
     stop. A shipped "no button needed" promise here is a support ticket with
     no self-service path.
     CUSTODY CONTRACT for the detection-triggered form (CKCRED, four terms):
     (1) the call is `reactivate` — clears the verdict without touching stored
     material; vault re-verifies on next use (the safe-to-press-wrongly
     property). (2) It REFUSES `corrupt` deliberately (a claim about vault
     bytes no client assertion can contradict) — a refused reactivate is
     Class-4 reconnect, never a retry. (3) THE BOUND IS CLIENT-ONLY AND IT IS
     THE ONLY DEFENCE: the vault cannot see "a detection" — it performs every
     authenticated reactivate. "Once per detection" must be the
     absent-then-present INSTALL TRANSITION (timer-free by construction, not
     by discipline); any implementation reducing to "check periodically,
     resume if latched" has silently removed the only bound in the system.
     (4) Misbehaviour is visible cheaply: reactivate on an active record is a
     no-op writing NOTHING (buggy detectors don't grow the audit chain), and
     every real resume is one durable entry — count reactivate entries against
     actual reinstalls to diagnose a detector defect without instrumenting
     the client.
     PLEX's retraction of the measured-wrong-arm row is the doc's named
     specimen for cross-seat review: "a true measurement, attached to the
     wrong mechanism" — the tell was only visible from the other side of the
     custody boundary, so anything that becomes a PROMISE gets cross-seat
     review, not just code. And the design lesson beside it: a constraint met
     by construction beats a constraint met by discipline, every time.
   - unmapped_operation (ALF): "That repository isn't connected to your
     agent. Add it in Settings > GitHub > Repositories." — NOT Class-1
     wording, which would send the user hunting a permission toggle that
     cannot exist for an uninstalled repo.
   - credential_permanent SPLITS (CKCRED): corrupt = reconnect repairs
     (Class 4 proper); not_found = "This connection's access token is no
     longer recognised. Reconnect to issue a new one." — never "remove and
     re-add" (the permanent class forbids config reaping).
   - custody_refused = Class 4; custody_unreachable/unavailable = Class 3.
5. VENDOR ANSWERS: http_403 = "the app doesn't have access to that
   repository" (NOT a reconnect — the 401-vs-403 split surfaced as different
   remedies). Vendor text renders attributed ("GitHub said: …"), never
   re-voiced.
6. SAFETY PAUSES (protective, not broken): "Paused for a safety check after
   GitHub changed its interface. Resumes after review." The pause is the
   feature working.
7. CLIENT BUGS (consumer must never see; assert in client CI): generic string
   + telemetry. A Class-7 string in the wild is a client defect by definition.
   Review-added rows: identity_mismatch, schema_unsupported.

THE PROBE-ARM RULE (from the S2 probe re-steer): a probe that MUTATES what it
probes is an arm, not a read — fire order, repair ownership, and second-fire
semantics are designed before the first fire (the uninstalled-row refusal
latches custody state; its two fires prove different things: vendor-asked vs
latch-holds-without-vendor-call).

THE WILDCARD RULE (CKCRED's generalization; print beside the classes): a
class NAME is read faster than the contract sentence beside it, and the name
wins — both gaps found in review (rate_limited, context_overflow) were filed
by the family their name suggests rather than the remedy they point at. The
class determines RETRY BEHAVIOUR, so a wildcard arm routing unknown codes by
family inherits that family's remedy: verify the remedy is right for every
code the family CAN carry, not just the ones seen so far. Clients WILL meet
codes that postdate this table.

SUBC's two specimens verified against the table (ambient-fallback = attribution
failure wording; manifest_regressed = Class 6 with actor named). Full per-code
rows: room seq 21.

PLEX SELF-AUDIT (one-validator rule at home): connections op=check is
COMPLIANT by construction — check returns the stored connection.status column
and the dispatch gate reads the same enforcement-written column (dispatch
outcomes flip it, recovery clears it on verb paths). CLIENT NOTE (binding):
`credential_bound` is a SETUP fact ("a binding blob exists"), never a health
fact — rendering it as "credential OK" rebuilds the support-ticket machine
from an honest field read for the wrong claim. STATUS is the health signal.
OPEN PRODUCT QUESTION (flagged, built on client demand, not speculatively):
a composite "GitHub is ready" readiness op derived from the same gate chain
invoke runs — never three client-side reads glued together.

RULING (Q3) — principles the table was built under:
- Every consumer string encodes the REMEDY and its ACTOR + retry class, not
  the mechanism: a policy deny is an ANSWER (retrying is wrong); an
  infrastructure fault is transient (retry is right). A transient must never
  wear a configuration error's clothes (two fleet support-loops this week).
- Never render ambient-ambiguity as success-shaped text: "posted, but as you
  rather than your agent" is an attribution FAILURE even though HTTP said 200.

## Known gaps (from fleet first-exercise, 2026-08-27)

1. Manifest provisioning has no product path (hand-assembled + chaired ceremony today).
2. S2 unexercised — needs a first-class test config before any client ships it.
3. `gh-shim --status` is operator JSON; clients need a friendly status surface.
4. `git.co_author` auto-derivation requires a manifest; no-App users need the
   explicit string form surfaced in doctor/settings.
5. Speech-as-bot vs administration-as-operator doctrine — RULED (Q4,
   unanimous): keep the doctrine for consumers, default fleet-shape, exposed
   as a VISIBLE setting. Constraints: the setting is REPO-STABLE (per-repo
   default changed deliberately, never per-action — oscillating authorship
   destroys audit reconstruction and echo-suppression ledgers); the "my own
   name" option states in its own words that the audit trail then cannot
   distinguish user from agent, and that this is irreversible for anything
   already posted. Registry fact supporting the default: agent identity
   (name, avatar, bot login) is one continuity even for solo users, and
   operator-name administration is what collaborators and history expect.

## Settled by fleet experience (import, don't re-litigate)

- Fail-closed disposition: bound repo + governed verb never falls through to
  ambient identity (structural, compile-enumerated).
- Signing raw payload bytes (no canonicalization) — cross-language safe.
- Version-monotonic manifest validity (no TTL expiry outages).
- Approval-before-signature audit ordering (CKCRED procedure, seq 4700).
- Widening a manifest SHRINKS the key-compromise surface (CKCRED's inversion).
- Approval-before-signature: FLEET-SCOPED (see Q1 ruling — the property's
  mechanism does not survive party collapse; S2/S3 downgrade explicitly).

## Ask-first approval: the second party-collapse hole (PLEX, round two)

Approve/deny are ops on PLEX's agent-facing tool facade — an agent can approve
its own parked write. Fleet shape: acknowledged theatre by operator ruling
(the gate activates when confinement gives it something to enforce; an agent
with the operator's shell IS the operator). CONSUMER REQUIREMENT (binding):
clients route approve/deny EXCLUSIVELY through the human UI principal — the
app renders the parked request, a human tap resolves it under the app's
operator identity, and approval ops are never reachable from the embedded
agent's tool surface. PLEX owes the matching server gate (approval ops refuse
agent-tier principals) the day a client principal model exists; until then
this is a stated dependency, not a silently inherited ceremony.

Approval-UX bar (imported precedent, operator-ratified this week for surface
grants): SEE WHAT YOU APPROVE, TWO CLICKS, ON THE MACHINE THE POLICY RESIDES
ON — record lands on the enforcing machine, decision portable. "Connect
GitHub" and ask-first rendering are held to that bar, not a new one.

## S2 exercise — EXECUTED (Q5 closed)

Fixtures on main (one agent id across 3 repos; untrusted-key, foreign-binding,
and holder-unreachable arms each with status-verb agreement asserted;
provenance fields rendering on every broken fixture). Live probe results:
- HAPPY: governed post on the scratch repo as cke2e-alfonso[bot],
  author-verified, R3 end to end under a dev-signed S2 manifest in an
  isolated state universe.
- Both of the latch arm's declared CANNOT-DISTINGUISH cases OCCURRED, once
  each, in opposite directions: a presumed-uninstalled repo that WAS covered
  (token minted; 404 on the missing issue refuted the premise), and a
  wrong-client_id hypothesis REFUTED BY DEDUCTION (custody verdict: only
  InvalidGrant latches, only the no-matching-installation branch produces it,
  and a JWT auth failure is transient-by-design — so the latch itself PROVES
  the credential pair is valid and the App is installed nowhere; the operator
  remedy stayed install-the-App, and flipping it would have sent the operator
  re-depositing a provably fine key). The honesty note is load-bearing, not
  decorative.
- THE NAMING RULE (CKCRED, from the misreading's post-mortem): a DISPOSITION
  variant needs a DISPOSITION name — `invalid_grant` was chosen for its
  disposition (unserviceable until a human acts) but reads as its OAuth
  origin (credential rejected), and the doc comment explaining that is
  invisible from outside the crate. The wildcard rule, one level in: a
  variant named for its origin gets read as its origin.
- FIRE-2 SILENCE PROOF, complete: custody's own event table shows exactly two
  rows (pre-reclassification applied=0; fire-1 applied=1) and NO third row
  for fire 2 — the latch is load-bearing, proven from the custody side by
  ABSENCE, after fire 1 established what a real attempt writes. Silence is
  the proof only because the positive control preceded it. Bonus: the two
  rows are a production before/after differential of the reclassification on
  the same input — evidence that normally needs a contrived experiment.
- The latch landed exactly as custody pre-announced (vault degraded 46/47,
  fleet-visible, story-first announcement pattern followed).
- OBSERVABILITY RULE (three seats' versions, printed once): a fail-closed
  layer that degrades silently is invisible from every seat except its own —
  THE COUNTER LIVES WITH THE LAYER THAT OWNS THE SILENCE. (Shim transcripts
  cannot distinguish latched-refusal from repeated-vendor-refusal; only
  custody call counters prove the latch is load-bearing. Same rule as PLEX's
  NULL-enrichment gap and CKCRED's unexercised-op lesson.)
- THE WIRE-AXIS PAIR (CKCRED, closing the observability finding): when a
  refusal is CORRECTLY identical on the wire (the consumer's remedy is the
  same either way, so the wire deliberately does not leak which internal path
  produced it), the thing the wire hides is provable ONLY from the producer's
  own records - no consumer-side probe can close it, however well built.
  Same root as one-validator, different axis: there the cheap surface could
  not SEE an input; here the honest wire deliberately does not CARRY one.
  And the absence-as-evidence caveat rides with it: an absent record proves
  something only when a positive control established, in the same table,
  what presence looks like - without the control the same emptiness proves
  nothing (fire 1 before fire 2; the trap this fleet has hit repeatedly).
- PLEX's ride discharged by PRODUCTION evidence, deliberately: every property
  S2 asks of their layer runs live daily at larger scale (29-repo
  single-grant reads since Aug 14, multi-repo write fences in the estate, the
  seq-21 ceremony validated by 20 live onboardings). Continuous evidence
  beats a one-shot scratch pass; S2-is-just-rows holds on their side too.

## S2 exercise plan (Q5, original commitment — superseded by execution above)

AFT builds this week: an S2-shaped manifest fixture (N repo rows, one
agent_id) + a live probe on the e2e scratch org. FAILURE ARMS ARE FIRST-CLASS
FIXTURES (SUBC rider): an untrusted-key manifest, a stale/foreign binding row,
and a status-vs-verb agreement assertion on every broken fixture (the
must-disagree twin of the happy-path positive control) — all three arms
occurred in production within one hour of fleet rollout. Riders confirmed:
- ALF: probe window doubles as the armed worktree->shim->holder->wire
  end-to-end verify when a mason drives it.
- PLEX: rides the same probe for conformance drift against the S2 manifest +
  one scratch write ceremony, proving the whole governed path under one-App
  shape. Holder needs ZERO changes (row-shape agnostic, resolves per
  agent_id at call time) — S2 is rows on every seat.

## Errata

### Authority vs identity on admin verbs (2026-08-27, from SUBC's first bound-seat merge)

The identity-room fold ("all Alfonsos merge their own PRs") and the admin tier
("merges require GH_SHIM_BYPASS=operator") read as contradictory until split:
the fold governs AUTHORITY (the agent decides and executes, no human gate);
the tier governs IDENTITY (a merge signs as the operator - the Apps lack
contents:write by design). Pre-binding merges already obeyed both: passthrough
executed them under ambient operator credentials. The sanctioned autonomous
path on a bound seat is the agent setting GH_SHIM_BYPASS=operator itself -
the deliberate, audit-visible identity switch. Verification inverts for admin:
assert the OPERATOR is the actor (a bot merger would be the bug), the mirror
of bot-author verification on speech.
