---
title: "gh routing shim: SEAM governed routing and execution"
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

Scope shape: this campaign builds AFT's half only — dispatch, rung
determination, manifest client, classification, passthrough, governed
transport, self-report, and governed-response rendering — coding against PLEX's
governance seam and CKCRED's custody as CONSUMED contracts whose absence must
degrade DOWN the ladder (no seam = R2; no custody = R3 never activates) rather
than fail obscurely or silently downgrade a governed verb into an ungoverned
exec. The shim holds zero credentials by architectural exclusion: tokens are
minted by custody, permissions-scoped per tier, and never enter the shim or any
shim-owned descendant.

Read-side output compression is OUT of scope for this campaign entirely (chair
amendment A1). Mechanical passthrough is always exec of real gh with argv
unmodified; the shim never execs `gh api` on an agent's behalf, never
substitutes porcelain with api calls, and never rewrites mechanical output
bytes. The ONLY rendering this campaign ships is for governed-verb responses,
which arrive as seam structured data and have no raw gh output to preserve —
rendering them is a necessity, not a feature. Porcelain read compression (issue
view / pr view / run list / checks) moves to a follow-on campaign (owner note
#2168) with its own corpus fixtures and thresholds.

Counterparty honesty (A7): PLEX's `gh.route` holder and CKCRED custody minting
do not exist in this repository. Acceptance cases requiring them are marked
`counterparty:PLEX` or `counterparty:CKCRED` and are satisfiable here only
against the spec-pinned mock holder. The campaign's exit criterion is all
non-counterparty cases green, mock-holder cases green, and the consumed contract
ids published for the counterparties; live-fleet acceptance is a post-campaign
milestone gated on PLEX shipping `gh.route`.

## glossary
- **seam** — the governance entry point on PLEX's side through which governed
  operations are forwarded, executed, and where identity assertions are
  double-validated against project bindings.

- **`gh.route` / `prefrontal-core`** — the consumed governance operation id and
  the module id serving it; capability detection is `catalog.list` advertising
  `gh.route`. Minted and fixed by this campaign (A6) and published for the
  counterparties; the shim codes against these ids and nothing wider.

- **`gh_route_request_v1` / `gh_route_result_v1`** — the consumed request and
  response schema ids. Request = {verb_tuple, target, body, asserted_identity,
  manifest_version}, with target and body in CR9 canonical form; response =
  {outcome: result|refusal, refusal_code?, http_status?, body?}. Neither
  direction ever carries a credential.

- **`gh_route_schema=1`** — the version field the request carries. A holder
  answering with a higher schema MAJOR is refused with
  `gh_shim_seam_schema_mismatch` rather than parsed hopefully.

- **mock holder (spec-pinned)** — the fixture module answering `gh.route` with
  canned results and refusals under the schema ids above. It is how every
  counterparty-dependent case is satisfied in-campaign (A7).

- **counterparty tag** — the acceptance marker `counterparty:PLEX` or
  `counterparty:CKCRED` on a case that requires a party absent from this
  repository. The campaign exit criterion is all non-counterparty cases green +
  mock-holder cases green + the A6 contract ids published; live-fleet acceptance
  is a post-campaign milestone gated on PLEX shipping `gh.route`.

- **verify_identity_assertion** — the named seam function. Ships consistency-
  only in this campaign; prefrontal-signed assertion tokens are scheduled
  closure behind the same seam, built as one upgrade point now with signature
  verification dropped in later without moving the call site.

- **identity assertion** — the calling agent's identity, derived from session
  binding config (project root -> agent id) and asserted explicitly in forwarded
  requests; unbound is an R2 determination input failure and, if reached later
  on the governed path, refuses with `gh_shim_unbound_identity`. Never a
  model-typed parameter — no tool argument, no env override, no flag by which a
  caller names who it is.

- **assert-plus-double-validate** — the uniform assertion tier: the shim asserts
  the agent identity and PLEX independently validates it against project
  bindings; merge ships alongside comments under this same tier.

- **subc daemon / bind** — the local supervisor daemon and the connection the
  shim opens ITSELF, per governed call, over loopback. A bind is a transport
  FACT; a shim-originated marker inside someone else's channel is merely data
  any route caller could assert.

- **Principal::Direct** — the daemon-side principal class the shim's bind
  presents as, coming from the user's shell; it maps first-party at AFT's gate.
  Acceptable BECAUSE the agent identity assertion is still consistency-checked
  at the seam and custody scopes the minted token arithmetically. Also the
  origin of PLEX's finding that identity-by-assertion inside an existing channel
  is the weak form.

- **bind triple / session namespace `gh-shim:<agent-id>`** — the bind identity
  `(project_root, "runner", "gh-shim:<agent-id>")`, whose session namespace
  marks shim origin so daemon-side observability separates shim binds from
  plugin binds. Per CR7 it needs no pre-registration or allowlisting: the daemon
  treats it as an ordinary consumer session identity and rate accounting rides
  the standard per-consumer limits. The separation is proven by reading the
  namespace back out of daemon logs or status output, so it holds in
  observability and not merely in the transport.

- **CKCRED / custody** — the seat and mechanism that mints GitHub tokens
  permissions-scoped per tier (arithmetic enforcement). Tokens never touch the
  shim: the shim holds ZERO credentials by architectural exclusion, and no
  shim-owned process or descendant ever carries one.

- **process-tree assertion / environ inspection** — the two acceptance
  instruments that make the credential exclusion literal rather than
  aspirational: on a governed classification the shim spawns NO gh child
  process (process-tree assertion), and neither the shim nor any descendant
  carries a GitHub token in its process environment during the call (environ
  inspection). Assertions, not conventions. Grounds: process environment is
  same-user-readable during the exec window and every agent process on the seat
  IS that user.

- **GitHub App identity / bot handle** — the per-agent App (handle + avatar) each
  fleet Alfonso now has; the precondition whose firing makes R3 activatable and
  machine-account PATs retirable. The bot App scope lacks closeIssue on
  non-authored issues, so those closes are declared tier `admin` unless the
  permission set grows.

- **governed-response rendering** — the v1-by-necessity renderer for governed
  verbs, which execute seam-side and return `gh_route_result_v1` structured data
  with no raw gh output to fall back on: result to stdout, refusal as
  `gh_shim_seam_refusal` wrapping the seam's own stable code on stderr with a
  shim-band exit status. It is the ONLY rendering this campaign ships.

## constraints
Builds on the shipped CORE campaign's manifest, rung, and classification contracts; CORE identifiers are normative here by reference.

- Room-fold decisions are FIXED inputs, not open questions: (1) uniform
  assertion tier — merge ships alongside comments with assert-plus-
  double-validate (Ufuk's practice ruling: twenty Alfonsos already merge their
  own PRs ungated); prefrontal-signed assertion tokens are SCHEDULED CLOSURE
  behind the same `verify_identity_assertion` seam, built as one upgrade point
  now, signature verification dropped in later. (3) The shim holds ZERO
  credentials by architectural exclusion — tokens are minted by custody
  (CKCRED, permissions-scoped per tier: arithmetic enforcement) and never touch
  the shim.

- GOVERNED EXECUTION PATH IS FIXED: the shim opens its OWN subc daemon bind per
  governed call. Grounds: the second-consumer cost evaporated once the shim
  became an aft subcommand (the binary already links the subc client stack twice
  over — the --subc module path and the fleet-status SubcConsumer — so an
  own-bind reuses linked code); per-call connect+handshake+bind on loopback is
  milliseconds against a human-paced operation class, far below rebind storms
  the post-hardening daemon already absorbs; and a bind is a transport FACT
  whereas a shim-originated marker is data any route caller could assert (PLEX's
  Principal::Direct finding: identity-by-assertion inside an existing channel is
  the weak form). The module-proxy alternative is secretly more machinery: no
  shim-to-module channel exists today, so proxying means building and then
  defending a new local IPC surface.

- The shim's bind presents as Principal::Direct from the user's shell and maps
  first-party at AFT's gate. The spec must say this explicitly rather than
  leaving the trust mapping implicit: it is acceptable BECAUSE the agent
  identity assertion is still consistency-checked at the seam and custody scopes
  the minted token arithmetically. The bind triple is (project_root, "runner",
  "gh-shim:<agent-id>"); the session namespace marks shim origin so daemon-side
  observability separates shim binds from plugin binds, and per CR7 that
  namespace needs no pre-registration and rides standard per-consumer rate
  accounting.

- Governed verbs are performed AT THE SEAM and never exec real gh, so exit-code
  fidelity with real gh is NOT a goal on that path: seam refusals map into the
  shim's own reserved exit band with the stable refusal code on stderr, while
  real gh's statuses pass through unmapped on the passthrough path ONLY. This is
  deliberate — a seam-side call returns a structured refusal or result over the
  bind, so no half-state exists in which gh executed but the audit line is
  ambiguous about which identity fired.

- Identity assertion comes from session binding config (project root ->
  agent id), asserted explicitly in forwarded requests, refused when unbound;
  PLEX double-validates against project bindings at their seam. No model-typed
  identity parameter anywhere.

### CR5 - admin-tier refusal and operator-bypass seam interaction

Ops the App identity cannot perform (closeIssue on non-authored issues today)
are declared in the manifest as tier `admin`. At R3 the shim refuses them with
stable code `gh_shim_admin_tier` and text naming the operator bypass. There is
no dual "governed and also administrative" description anywhere in the spec.

Governed verbs remain seam-refused for agent
identities server-side regardless of the variable.

### CR7 - namespace and accounting

`gh-shim:<agent-id>` requires no pre-registration: the daemon treats it as an
ordinary consumer session identity, and rate accounting rides the standard
per-consumer limits. One sentence in interfaces; the audit-separation claim in
acceptance keys on the session prefix, which needs no new daemon feature.

### A6 - literal identifiers (SEAM clauses)

- Consumed governance operation (catalog + route target): operation id
  `gh.route`, served by module id `prefrontal-core`; capability detection =
  catalog.list advertising `gh.route`. Request schema id `gh_route_request_v1`
  = {verb_tuple, target, body, asserted_identity, manifest_version}; response
  schema id `gh_route_result_v1` = {outcome: result|refusal, refusal_code?,
  http_status?, body?}. Version negotiation: request carries
  `gh_route_schema=1`; a holder answering with a higher major refuses with
  `gh_shim_seam_schema_mismatch`.

- Bind identity: the shim binds with harness label `runner` and session id
  `gh-shim:<agent-id>` under the invoking project root — the triple is
  (project_root, "runner", "gh-shim:<agent-id>"); no daemon-side registration
  is required and rate accounting rides the standard per-session limits.

### A7 - acceptance honesty about absent counterparties

PLEX's `gh.route` holder and CKCRED custody minting do not exist in this
repository; every acceptance case that requires them is marked
`counterparty:PLEX` or `counterparty:CKCRED` and is satisfiable in this
campaign ONLY against the spec-pinned mock holder (fixture module answering
gh.route with canned verdicts). The campaign's exit criterion is: all
non-counterparty cases green + mock-holder cases green + the consumed contract
ids of A6 published for the counterparties. Live-fleet acceptance is a
post-campaign milestone gated on PLEX shipping gh.route.

## design
### Governed execution path

On a governed classification the shim opens its OWN subc daemon bind, per call,
over loopback. It does not proxy through the AFT module and does not carry a
shim-originated marker inside someone else's channel.

Grounds, carried here because they constrain the implementation: the shim is an
aft subcommand and the binary already links the subc client stack twice over
(the `--subc` module path and the fleet-status SubcConsumer), so an own-bind
reuses linked code rather than adding a dependency; per-call
connect+handshake+bind on loopback is milliseconds against a human-paced
operation class; and a bind is a transport FACT, whereas a marker is data any
route caller could assert (PLEX's Principal::Direct finding: identity-by-
assertion inside an existing channel is the weak form). The audit line's value
is "which CONNECTION asserted", and only an own-bind yields that.

**Consumed contract, literally (A6).** The governed operation is `gh.route`,
served by module id `prefrontal-core`, detected by `catalog.list` advertising
it. The request schema is `gh_route_request_v1` =
`{verb_tuple, target, body, asserted_identity, manifest_version}`, with `target`
and `body` in the CR9 canonical form; the response schema is `gh_route_result_v1`
= `{outcome: result|refusal, refusal_code?, http_status?, body?}`. The request
carries `gh_route_schema=1`; a holder answering with a higher major is refused
with `gh_shim_seam_schema_mismatch` rather than being parsed hopefully. These
ids are minted and fixed by this campaign and published for the counterparties;
the shim codes against them and against nothing wider.

**Execution locus.** Governed verbs are performed AT THE SEAM. The shim never
execs real gh on a governed classification, and no GitHub credential ever enters
a process owned by the agent user on that path. The shim forwards, then renders
what comes back. Because the governed verb set is deliberately tiny (comments,
reviews, reactions, closes, merges — the tier system's whole point), the seam
implements a handful of GitHub REST calls rather than reimplementing gh;
mechanical operations still exec real gh untouched and authenticate with
whatever the machine already has (CR8).

**Exit-code fidelity is NOT a goal for governed verbs.** The shim maps seam
refusals onto nonzero exits in its own reserved shim-originated band, with the
stable refusal code on stderr — a seam-returned refusal rendered as
`gh_shim_seam_refusal` wrapping the seam's own code. Real gh's statuses continue
to pass through unmapped on the passthrough path only. This is deliberate: a
seam-side call returns a structured refusal or result over the bind, so there is
no half-state in which gh executed but the audit line is ambiguous about which
identity fired.

**Bind identity.** The bind presents as `Principal::Direct` from the user's
shell and maps first-party at AFT's gate. This is stated rather than left
implicit: it is acceptable BECAUSE the agent identity assertion is still
consistency-checked at the seam and custody scopes the minted token
arithmetically. The bind triple is `(project_root, "runner", "gh-shim:<agent-id>")`;
the session namespace marks shim origin so daemon-side observability separates
shim binds from plugin binds, and per CR7 it needs no pre-registration or
allowlisting — the daemon treats it as an ordinary consumer session identity and
rate accounting rides the standard per-consumer limits. The audit line sources
its assertion record from the shim's own bind, and the separation is proven by
reading the namespace back out of daemon logs or status output, so it holds in
observability and not merely in the transport.

**Identity assertion.** Derived from session binding config (project root ->
agent id) and asserted explicitly in the forwarded request. Unbound is an R2
input failure at determination time and, if reached later on the governed path,
refuses with `gh_shim_unbound_identity`. Never a model-typed parameter anywhere
in the path. PLEX double-validates the assertion against project bindings at the
seam — assert-plus-double-validate, the uniform tier under which merge ships
alongside comments.

**`verify_identity_assertion`.** One named seam function, built now as the
single upgrade point, shipping consistency-only. Prefrontal-signed assertion
tokens are scheduled closure behind this same function: signature verification
drops in later without moving the call site.

**Credentials.** The shim holds zero credentials by architectural exclusion, and
the exclusion is literal rather than aspirational: no token transits the shim's
process environment, because process environment is same-user-readable during an
exec window (`/proc/<pid>/environ` on Linux, `ps eww` on macOS) and every agent
process on the seat IS that user — exactly the population custody exists to
exclude. A short-TTL scoped token would bound the damage without changing the
class. Tokens are minted by custody (CKCRED), permissions-scoped per tier, and
the custody token API stays entirely inside the PLEX/CKCRED boundary; the shim's
bind carries the request, never a credential in either direction. The renderer
is consequently written against a response shape rather than against a
subprocess.

**Counterparty status (A7).** Neither the `gh.route` holder nor CKCRED's minting
exists in this repository. Every part of this path that requires them is
exercised in-campaign against a spec-pinned MOCK HOLDER — a fixture module
answering `gh.route` with canned results and refusals under the schema ids
above — and the corresponding acceptance cases carry `counterparty:PLEX` or
`counterparty:CKCRED`. The campaign exits on non-counterparty cases green,
mock-holder cases green, and the A6 contract ids published; live-fleet
acceptance is a post-campaign milestone gated on PLEX shipping `gh.route`. This
is stated in the design rather than only in acceptance because it determines
what the transport is built against.

### Admin-tier seam interaction

**Admin tier (CR5, as corrected by A4).** Ops the App identity cannot perform —
closeIssue on non-authored issues today — are declared tier `admin` and refused
at R3 with `gh_shim_admin_tier`, the refusal text naming the operator bypass.
Nothing in this spec describes an op as both governed and administrative.

Governed verbs remain seam-refused for agent identities server-side
regardless of the variable.

### Rendering

One renderer, one surface, and a deliberate exclusion.

**Governed-response rendering (v1, by necessity).** Governed verbs execute
seam-side, so their results and refusals arrive as `gh_route_result_v1`
structured data with no raw gh output to fall back on. The minimal structured
render therefore ships with the identity slices: result rendering to stdout,
plus stable refusal-code rendering to stderr with a shim-band exit status. This
is not a compression feature; it is the only way a governed verb can produce
output at all. The per-verb table inside the renderer module is the single edit
point, so movement within the governed set is a table edit rather than a
rewrite.

**No read compression in this campaign (A1).** Mechanical passthrough is exec of
real gh with argv unmodified: the shim never execs `gh api` on an agent's
behalf, never substitutes porcelain with api calls, and never rewrites
mechanical output bytes. All non-governed output is byte-identical to real gh's
across the acceptance corpus, and R1/R2 output stays byte-identical permanently
by construction (CR3). There are no compression slices, no corpus byte budgets,
and no weak-model parse gates in this campaign's plan.

Porcelain read compression (issue view / pr view / run list / checks) moves to a
follow-on campaign under owner note #2168, with its own corpus fixtures and
thresholds. The reason it is separable rather than merely deferred: read
rendering changes what every agent SEES — a prompt-cache and weak-model-parsing
surface — and this fleet's own history (hint-parameter removal, TOON refusal)
says output-shape changes earn fixture-verified corpus comparisons on their own
schedule rather than riding an identity campaign's slices. The renderer module
and its per-verb table are the seam that follow-on work extends; nothing in this
design has to move to accommodate it.

### Legacy-account retirement gate

Legacy machine-account removal is a post-campaign milestone, not a slice. After
a proving window of App-handle posting with no attribution regressions, ALF, as
registry owner, queues one explicit owner ask to Ufuk; Ufuk performs the
irreversible organization-level deletions by hand. Nothing in this campaign
automates account deletion.

## interfaces
This section fixes the surfaces this campaign OWNS and names the surfaces it
only CONSUMES. Owned surfaces are normative here; consumed surfaces are
recorded as the contract shape this build codes against, with degradation
behaviour stated for each, because PLEX's seam and CKCRED's custody ship on
their own seats' schedules. Rungs are named R1/R2/R3 throughout (CR10).

### Owned: governed transport (shim's own daemon bind)

Governed classifications open the shim's OWN subc daemon bind, per call, over
loopback. There is no shim-to-module channel and none is built: no module proxy,
no shim-originated marker inside another caller's channel.

- **Execution locus.** Governed verbs are performed AT THE SEAM. The shim never
  execs real gh on a governed classification and spawns no gh child process at
  all on that path — a process-tree assertion, not a convention; it forwards,
  then renders what comes back. Because the governed verb set is deliberately
  tiny (comments, reviews, reactions, closes, merges), the seam implements a
  handful of GitHub REST calls rather than reimplementing gh.
- **Consumed contract, literally (A6).** The operation id is `gh.route`, served
  by module id `prefrontal-core`, detected by `catalog.list` advertising it.
  Request schema `gh_route_request_v1` =
  `{verb_tuple, target, body, asserted_identity, manifest_version}`, with
  `target` and `body` in the CR9 canonical form; response schema
  `gh_route_result_v1` = `{outcome: result|refusal, refusal_code?, http_status?,
  body?}`. The request carries `gh_route_schema=1`; a holder answering with a
  higher major is refused with `gh_shim_seam_schema_mismatch` rather than parsed
  hopefully. The request carries no credential in either direction.
- **Response rendering.** Result to stdout; refusal as `gh_shim_seam_refusal`
  wrapping the seam's own stable code, on stderr, with a shim-band exit status.
- **Principal.** The bind presents as `Principal::Direct` from the user's shell
  and maps first-party at AFT's gate. Stated explicitly rather than left
  implicit: this is acceptable BECAUSE the agent identity assertion is still
  consistency-checked at the seam and custody scopes the minted token
  arithmetically.
- **Bind identity and session namespace (CR7, A6).** The bind triple is
  `(project_root, "runner", "gh-shim:<agent-id>")`, so daemon-side observability
  separates shim binds from plugin binds. It requires NO pre-registration or
  allowlisting: the daemon treats it as an ordinary consumer session identity
  and rate accounting rides the standard per-consumer limits.
- **Audit line.** The record of WHICH CONNECTION asserted is sourced from this
  bind, because a bind is a transport fact whereas a marker is data any route
  caller could assert. The separation is proven by reading the namespace back
  out of daemon logs or status output, so it holds in observability and not
  merely in the transport.

### Owned: identity assertion surface

The assertion is derived from session binding config (project root -> agent id)
and asserted explicitly in the forwarded request. Unbound is an R2 input failure
at determination time and, if reached later on the governed path, refuses with
`gh_shim_unbound_identity`. There is no model-typed identity parameter anywhere
in the path — no tool argument, no env override, no flag by which a caller names
who it is.

`verify_identity_assertion` is the single named seam function and the single
upgrade point. It ships consistency-only in this campaign; prefrontal-signed
assertion tokens are scheduled closure behind this same function, with signature
verification dropped in later without moving the call site. PLEX double-validates
the assertion against project bindings at the seam — assert-plus-double-validate,
the uniform tier under which merge ships alongside comments.

### Owned: credential surface (empty by construction, and literally empty)

The shim exposes no interface for accepting, storing, forwarding, or caching a
GitHub credential — and, per the seam-side execution ruling, no interface for
handing one to a CHILD process either. No token enters any process owned by the
agent user at any point on the governed path; the acceptance suite inspects the
environ of the shim and of every descendant during a governed call rather than
taking the exclusion on faith.

Grounds carried here because they bound the interface: process environment is
same-user-readable during the exec window (`/proc/<pid>/environ` on Linux,
`ps eww` on macOS) and every agent process on the seat IS that user — exactly the
population custody exists to exclude. A short-TTL scoped token would bound the
damage without changing the class, so `GH_TOKEN`-in-env passthrough is not an
available design.

Consequently the custody (CKCRED) token API stays entirely inside the PLEX/CKCRED
boundary. The shim's bind carries a request in one direction and a
result-or-refusal in the other, and never a credential in either.

### Admin-tier refusal and operator bypass

Ops the App identity cannot perform — closeIssue on non-authored issues today —
are declared tier `admin` and refused at R3 with `gh_shim_admin_tier`, the
refusal text naming the operator bypass. Nothing in this spec describes an op as
both governed and administrative.

Governed verbs remain seam-refused for agent
identities server-side regardless of the variable.

### Owned: renderer

One module, one per-verb table, ONE surface in this campaign.

**Governed-response rendering — v1, by necessity.** Governed verbs execute
seam-side and return `gh_route_result_v1` structured data, so there is no raw gh
output to fall back on. The minimal structured render (result rendering to
stdout plus refusal-code rendering to stderr with a shim-band exit status) ships
with the identity slices. This is not a compression feature; it is the only way
a governed verb produces output at all. The per-verb table is the single edit
point, so movement inside the governed set is a table edit rather than a
rewrite.

**No read compression in this campaign (A1).** Mechanical reads are
exec-passthrough with argv unmodified and their output bytes are never
rewritten; ALL non-governed output is byte-identical to real gh's across the
acceptance corpus, and R1/R2 output stays byte-identical permanently by
construction (CR3). There is no compression renderer, no byte budget, no
weak-model parse gate, and no default-on knob in this campaign's interface
surface. Porcelain read compression (issue view / pr view / run list / checks)
moves to a follow-on campaign under owner note #2168 with its own corpus
fixtures and thresholds; the renderer module and its per-verb table are the seam
that follow-on work extends, and nothing here has to move to accommodate it.

### Consumed: PLEX seam and CKCRED custody, and the mock holder (A7)

This campaign builds against their contracts and implements neither. The seam
owes `gh_route_request_v1` / `gh_route_result_v1` under `gh_route_schema=1`,
stable machine refusal codes the shim can render verbatim inside
`gh_shim_seam_refusal`, and the GitHub-side execution of the tiny governed verb
set; custody owes per-tier permission-scoped minting entirely within its own
boundary. Absence of either must degrade cleanly along the ladder rather than
fail obscurely: no seam means R2 (byte-transparent passthrough, zero
per-invocation output, determination recorded in the rung cache); no custody
means R3 does not activate. Contract drift on either side surfaces as a loud
named refusal with a stable code, never as a silent downgrade of a governed verb
into an ungoverned exec.

Neither counterparty exists in this repository. Every surface above that
requires them is exercised in-campaign against a spec-pinned MOCK HOLDER — a
fixture module answering `gh.route` with canned results and refusals under the
schema ids above — and the corresponding acceptance cases carry
`counterparty:PLEX` or `counterparty:CKCRED`. The campaign exits on
non-counterparty cases green, mock-holder cases green, and the A6 contract ids
published; live-fleet acceptance is a post-campaign milestone gated on PLEX
shipping `gh.route`.

### Legacy-account retirement gate

**Legacy machine accounts** are a post-campaign milestone, not a slice: after a
proving window of App-handle posting with no attribution regressions, ALF (the
registry owner) queues a single explicit owner ask to Ufuk, who executes the
org-level deletions by hand. Account deletion is irreversible and stays behind a
human gate; nothing here automates it.

## acceptance sketch
Cases are tagged `counterparty:PLEX` or `counterparty:CKCRED` where they require
a party absent from this repository; those are satisfiable in-campaign ONLY
against the spec-pinned mock holder (fixture module answering `gh.route` with
canned verdicts under `gh_route_request_v1` / `gh_route_result_v1`). The
campaign exit criterion is: all non-counterparty cases green + mock-holder cases
green + the A6 contract ids published. Live-fleet acceptance is a post-campaign
milestone gated on PLEX shipping `gh.route` (A7).

- On a governed (R3) seat [counterparty:PLEX, counterparty:CKCRED]:
  `gh issue comment N --body X` in bash routes through the seam and lands AS the
  calling agent's bot handle (avatar renders); the audit line records which
  connection asserted, sourced from the shim's OWN daemon bind under the triple
  `(project_root, "runner", "gh-shim:<agent-id>")` so daemon-side observability
  separates it from plugin binds — asserted with NO daemon-side registration or
  allowlisting step (CR7), and asserted by READING THE NAMESPACE BACK out of
  daemon logs or status output, so separation is proven in observability and not
  only in the transport; `gh api repos/.../issues/N/comments -f body=X` routes
  identically (api_match closure); `gh run list` and `gh api
  repos/.../actions/runs` pass through byte-transparent off the mechanical
  allowlist.

- Seam-side execution, negative form
  [counterparty:PLEX, counterparty:CKCRED]: on a governed classification the shim
  spawns NO gh child process (process-tree assertion) and no shim-owned process
  — the shim itself or any descendant — ever carries a GitHub token in its
  environment (environ inspection during the call). The test asserts the
  governed result and any refusal arrive over the bind as structured data.

- Consumed-contract fidelity [counterparty:PLEX]: the forwarded request is
  `gh_route_request_v1` = {verb_tuple, target, body, asserted_identity,
  manifest_version} carrying `gh_route_schema=1` against operation `gh.route`
  on module `prefrontal-core`, detected by `catalog.list`; a mock holder
  answering with a higher schema major is refused with
  `gh_shim_seam_schema_mismatch` rather than parsed hopefully.

- Governed response rendering [counterparty:PLEX]: a governed verb's result
  renders from the seam's
  `gh_route_result_v1` structured response with no raw gh output available; a
  seam refusal renders as `gh_shim_seam_refusal` wrapping the seam's own stable
  code on stderr and exits in the shim's reserved band. Exit-code fidelity with
  real gh is NOT asserted for governed verbs; it IS asserted, unmapped, for
  every passthrough verb.

- Governed-set feasibility [counterparty:PLEX, counterparty:CKCRED]: each social
  verb the manifest
  declares governed (comment, review, reaction, close, merge) is exercised once
  end-to-end under a real App installation before its tier is locked. A verb the
  App scope cannot perform is a finding that moves it to tier `admin` (CR5), not
  a governed verb that fails in the field.

- The 2026-08-17 bypass shape is dead on GOVERNED (R3) seats
  [counterparty:PLEX, counterparty:CKCRED]: bare `gh` social
  verbs cannot land as the operator identity from an agent session. The claim is
  scoped to R3 and the suite states the R1/R2 residue explicitly rather than
  overclaiming (CR8).

## non-goals
- No prefrontal-signed token VERIFICATION in this campaign (the
  `verify_identity_assertion` seam function ships consistency-only; signature
  drop-in is the scheduled follow-up owned with ALF's decision plane).

- No PLEX-side seam implementation, no CKCRED custody implementation, no App
  minting flows — those are their seats' halves; this campaign builds against
  their contracts and must degrade cleanly while any of them is absent.

- No live-fleet acceptance as a campaign exit criterion (A7): the `gh.route`
  holder and custody minting do not exist in this repository, so
  counterparty-tagged cases are satisfied ONLY against the spec-pinned mock
  holder. The campaign exits on non-counterparty cases green, mock-holder cases
  green, and the A6 contract ids published; live-fleet acceptance is a
  post-campaign milestone gated on PLEX shipping `gh.route`.

- No credential transit through the shim in any form: the governed path does
  NOT exec real gh with a custody-minted token in env, and no shim-owned
  process or descendant ever holds a GitHub token. Process environment is
  same-user-readable during the exec window, and a short-TTL scoped token would
  bound the damage without changing the class.

- No gh exit-code fidelity for governed verbs, and no emulation of gh's output
  formatting on the governed path: governed verbs execute seam-side and return
  structured data, which the shim renders with its own reserved refusal band.
  Fidelity IS asserted, unmapped, on the passthrough path.

- No new shim-to-module IPC surface: governed calls use the shim's own daemon
  bind, not a module proxy carrying a shim-originated marker.

- No daemon-side registration, allowlisting, or new rate-accounting feature for
  the `gh-shim:<agent-id>` session namespace (CR7): it rides the standard
  per-consumer accounting as an ordinary consumer session identity.

- No automated deletion of the legacy machine accounts: that is a post-campaign
  milestone — ALF proposes after a proving window, Ufuk executes by hand, since
  account deletion is irreversible and stays behind a human gate.

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

- No merge/close policy engine: tier enforcement + custody scoping only;
  content policy stays at PLEX's seam.

- No dual "governed and also administrative" description of any op:
  App-incapable ops carry the single tier `admin` (CR5), with
  `GH_SHIM_BYPASS=operator` as an audited operator convenience — required-write
  before exec (A4), never a silent or agent-reachable hole.

## open_assumptions
This section records beliefs this campaign TAKES AS TRUE without having
verified them, distinct from `open_questions` (dispositions not yet decided;
currently empty). Each entry names the assumption, what breaks if it is false,
and the cheap check that would settle it. Several of the FIXED decisions —
room-fold rulings and chair rulings CR1-CR10 and amendments A1-A7 alike — rest
on assumptions listed here; naming them is not reopening those decisions, it is
recording precisely what would have to be false for one to need revisiting.
Rungs are named R1/R2/R3 throughout (CR10).

### Governed transport

- **The subc client stack is already linked into the binary and reusable from a
  subcommand entry point.** The fixed governed-path ruling's cost argument
  depends on the `--subc` module path and the fleet-status SubcConsumer already
  linking what an own-bind needs. If an own-bind in fact pulls new code or new
  initialisation, the cost argument weakens — but the ruling still stands on
  its provenance ground (a bind is a transport fact), and the dependency-free
  passthrough path must remain untouched either way. Cheap check: build the
  shim's bind path and diff binary size and cold-start cost.
- **Per-call connect+handshake+bind on loopback is milliseconds, and the daemon
  does not meter per-bind in a way that penalises one bind per governed verb.**
  CR7 fixes that `gh-shim:<agent-id>` rides the standard per-consumer
  accounting; the assumption is that those standard limits are sized for one
  bind per human-paced social verb. If binds are metered tightly or are
  materially slower than assumed, governed social verbs feel slow and a
  persistent or pooled bind becomes a real question rather than a rejected
  optimisation.
- **`Principal::Direct` from the shim's bind maps first-party at AFT's gate
  today, with no daemon-side change.** If a shim-originated Direct bind is
  currently gated or requires registration, this campaign acquires a daemon
  dependency it does not currently plan for, and R3 cannot activate anywhere
  until that dependency is met.
- **The `gh-shim:<agent-id>` session namespace is accepted as an ordinary
  consumer session identity with no pre-registration or allowlisting (CR7).**
  This is a fixed ruling resting on an unverified belief about current daemon
  behaviour; the acceptance suite asserts the no-registration property, so a
  daemon that rejects or requires registration of the namespace surfaces as a
  failing acceptance case rather than as a silent downgrade. Cheap check:
  open a bind under the namespace against a stock daemon before slicing the
  governed path.
- **Daemon-side observability actually surfaces the session namespace.** The
  audit-separation criterion keys on the `gh-shim:` prefix, which assumes the
  daemon's audit records carry the consumer session identity somewhere an
  operator can read it. If they do not, shim binds are separable in the
  transport but not in observability, and the separation claim needs a daemon
  feature this campaign did not plan for. Cheap check: open a bind under the
  namespace and read it back out of daemon logs or status output.

### Seam-side execution

- **The seam can perform the whole governed verb set over GitHub REST.** The
  ruling's cost argument rests on the governed set staying tiny — comments,
  reviews, reactions, closes, merges. If a governed verb needs behaviour that
  only gh implements (multi-step review submission flows, editor-driven bodies,
  interactive confirmation), the seam either grows a gh-shaped surface or that
  verb is declared tier `admin` (CR5). Cheap check: enumerate the governed set
  against the REST endpoints that serve it before locking the manifest tiers.
- **The seam returns a structured shape, not an opaque stream.** The
  governed-response renderer is written against a response shape deliberately,
  so it is indifferent to how the seam reaches GitHub; this assumes
  `gh_route_result_v1` does not hand back raw gh-shaped bytes that the shim
  would have to re-parse.
- **The seam's refusals carry stable machine codes the shim can render
  verbatim.** The refusal-code enumeration criterion treats seam-returned
  refusals as first-class members of the closed A6 set (wrapped as
  `gh_shim_seam_refusal`); if the seam returns prose only, the shim would have
  to synthesise codes, which is exactly the message-text coupling PLEX's
  enumeration suite is meant to avoid.
- **The spec-pinned mock holder is faithful enough to stand in for the real
  one (A7).** Every counterparty-tagged case is satisfied against canned
  verdicts under `gh_route_request_v1` / `gh_route_result_v1`, so the campaign
  proves the shim against OUR reading of the contract rather than against
  PLEX's implementation. If the shipped holder diverges, the divergence lands
  as a post-campaign live-fleet finding, and the A6 ids are the surface it
  argues over. Cheap check: publish the A6 ids to the counterparties early and
  have PLEX confirm the request/response shapes before the mock is frozen.
- **No governed verb needs a TTY.** Since the governed path never execs gh,
  anything that depended on gh's interactive behaviour (prompts, pagers, editor
  invocation) is gone by construction. Assumed nobody's workflow relies on it;
  if one does, the verb is declared tier `admin` rather than governed.
- **Governed latency is acceptable at human pace.** Bind plus seam plus GitHub
  round trip replaces a direct gh call. Assumed within the human-paced operation
  class the transport ruling already invoked.

### Identity and credentials

- **Session binding config resolves in every place a governed verb is issued.**
  Project-root -> agent-id resolution is assumed to work from worktrees,
  submodules, nested checkouts, and the cwd a bash tool call actually starts
  in. Unbound is an R2 determination-input failure by design; the assumption is
  that unbound is RARE on governed seats, not that it is impossible. If it is
  common, R2 becomes the normal rung on seats that believe they are governed,
  and agents learn to reach for bare gh.

- **The bot App scope covers the social set the uniform assertion tier ships.**
  closeIssue on non-authored issues is a KNOWN gap and the manifest expresses
  it as tier `admin` (CR5). It is assumed that comment, review, reaction, and
  MERGE are inside the App's permission set. If merge is not, the fixed
  uniform-tier ruling ships a governed verb the bot identity cannot perform,
  and merge joins closes at tier `admin` until the permission set grows. Cheap
  check: exercise each social verb once under a real App installation before
  declaring the tier.

- **Custody can mint per-tier permission-scoped tokens on the schedule this
  campaign needs.** CKCRED is a consumed contract; if scoping arrives coarser
  than per-tier, the arithmetic-enforcement backstop is weaker than the design
  assumes, though the shim's empty credential surface is unaffected — the token
  never leaves the PLEX/CKCRED boundary under either scoping.

- **Environ inspection is a sufficient instrument for the credential
  exclusion.** The acceptance suite proves "no token in any shim-owned process"
  by inspecting the environ of the shim and its descendants during a governed
  call. Assumed a credential could not reach a child by some other channel
  (inherited fd, temp file, keychain handle) that the environ check would miss;
  the process-tree assertion (no gh child at all on the governed path) is the
  complementary instrument that makes the gap narrow rather than open.

## open questions
- none: closed by chair rulings CR1-CR10 and fold-v4 chair amendments A1-A7 (constraints). The porcelain read-compression follow-on is deliberately out of scope (A1).
