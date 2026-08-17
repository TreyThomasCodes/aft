# gh routing shim caller audit, v1

Date: 2026-08-17

This audit is the activation fence for `gh-routing-manifest` v1. A release build
has no production manifest trust root, so it cannot reach R3 until this audit is
re-run against the deployed manifest and every blocking form below is resolved.
The empty release trust set in `crates/aft/src/gh_shim.rs` is intentional: adding
a production key before this table is closed would turn the list into an
unenforced warning.

## Classified callers

The retained AFT-owned forms that run in the shim's agent PATH are:

| Caller | Form | v1 disposition |
| --- | --- | --- |
| `scripts/release.sh` | `gh run list` | mechanical |
| `scripts/watch-ci.sh` | `gh run list`, `gh run view` | mechanical |
| `scripts/wait-release.sh` | `gh run list`, `gh run view` | mechanical |
| `.github/workflows/bump-opencode.yml` | `gh api` default GET | mechanical |
| `crates/aft/src/bash_background/registry.rs` test fixtures | `gh issue list`, `gh pr view` | mechanical fixtures only |

The shim's v1 mechanical declarations cover the listed `run`, `issue`, `pr`, and
GET API forms. These callers retain their original argv and upstream `gh` process
behavior; no caller is migrated to an `api` substitute.

## Activation blockers requiring a separately reviewed manifest release

The following retained forms are intentionally **not** declared by v1. They must
be migrated, retired, or added by a signed manifest release with an accompanying
route capability review before a production trust root is installed:

| Caller | Unclassified form | Required disposition |
| --- | --- | --- |
| `packages/aft-cli/src/lib/github.ts` | `gh issue create` | governed action and seam capability, or retire interactive filing |
| `scripts/wait-release.sh` | `gh auth status`, `gh run cancel` | mechanical/admin declaration after operator review |
| `scripts/fetch-subc-core.sh` | `gh release download` | mechanical declaration after supply-chain review |
| `.github/workflows/bump-opencode.yml` | `gh pr create` | governed/admin declaration for the CI App identity |
| `.github/workflows/discord-release.yml` and `release.yml` | `gh release view`, `gh workflow run` | mechanical/governed declaration after release-flow review |

Workflow-only forms execute in GitHub Actions rather than the fleet agent PATH,
but they are recorded here so a future runner that installs the shim cannot make
them silently fail closed. The audit must be repeated for any new bare `gh`
caller, alias, wrapper, or absolute-path invocation before it is placed on an R3
seat.
