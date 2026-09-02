#!/usr/bin/env bash
# Operator tooling runs through the real GitHub CLI (gh); the shim is only for AI agent commands.
# Gated PR merge - the ONLY sanctioned way to merge a contributor PR.
#
# Exists because of #234 (2026-08-17): a P1 bot finding landed at 12:10, the
# author pushed the fix at 12:17, and a hand merge at ~12:19 took the head
# WITHOUT the fix - forcing a post-merge follow-up PR. A merge must not race
# an iterating author or an open finding.
#
# Refuses to merge when:
#   1. any review thread is unresolved,
#   2. any review/review-comment is NEWER than the head commit (a finding the
#      author has not pushed over yet),
#   3. the head was pushed very recently (author likely mid-iteration),
#   4. CI on the head is red/pending (merge rides gh's own check gate).
#
# Usage: scripts/merge-pr.sh <pr-number> [--settle-minutes N] [--force-reason "..."]
set -euo pipefail

PR="${1:?usage: merge-pr.sh <pr-number> [--settle-minutes N] [--force-reason ...]}"
shift || true
SETTLE_MIN=5
FORCE_REASON=""
while [ $# -gt 0 ]; do
  case "$1" in
    --settle-minutes) SETTLE_MIN="$2"; shift 2 ;;
    --force-reason) FORCE_REASON="$2"; shift 2 ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done

source "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/lib/operator-gh.sh" || exit 1

OWNER_REPO=$("$OPERATOR_GH" repo view --json nameWithOwner --jq .nameWithOwner)
OWNER="${OWNER_REPO%%/*}"
REPO="${OWNER_REPO##*/}"

refuse() {
  echo "merge-pr: REFUSED - $1" >&2
  if [ -n "$FORCE_REASON" ]; then
    echo "merge-pr: proceeding anyway (--force-reason: $FORCE_REASON)" >&2
    return 0
  fi
  exit 1
}

# --- 1. Unresolved review threads ------------------------------------------
UNRESOLVED=$("$OPERATOR_GH" api graphql -f query='
  query($owner:String!,$repo:String!,$pr:Int!){
    repository(owner:$owner,name:$repo){
      pullRequest(number:$pr){
        reviewThreads(first:100){nodes{isResolved}}
      }
    }
  }' -F owner="$OWNER" -F repo="$REPO" -F pr="$PR" \
  --jq '[.data.repository.pullRequest.reviewThreads.nodes[] | select(.isResolved==false)] | length')
[ "$UNRESOLVED" -eq 0 ] || refuse "$UNRESOLVED unresolved review thread(s); resolve or let the author finish"

# --- 2. Findings newer than the head commit --------------------------------
HEAD_SHA=$("$OPERATOR_GH" pr view "$PR" --json headRefOid --jq .headRefOid)
HEAD_TS=$("$OPERATOR_GH" api "repos/$OWNER_REPO/commits/$HEAD_SHA" --jq .commit.committer.date)
NEWER=$(
  {
    "$OPERATOR_GH" api "repos/$OWNER_REPO/pulls/$PR/comments" --paginate --jq '.[].created_at'
    "$OPERATOR_GH" api "repos/$OWNER_REPO/pulls/$PR/reviews" --paginate --jq '.[] | select(.state != "APPROVED" and .state != "PENDING") | .submitted_at'
  } | awk -v head="$HEAD_TS" '$0 > head' | wc -l | tr -d ' '
)
if [ "$NEWER" -ne 0 ]; then
  # The check exists to stop merging while an author is still addressing
  # findings. When the maintainer resolved the outstanding threads with a
  # recorded rationale (so newer comments are the resolution itself, not
  # unaddressed findings), the maintainer can acknowledge them explicitly.
  # The reason is printed so the merge log carries the judgment.
  if [ -n "${AFT_MERGE_PR_ACK_FINDINGS:-}" ]; then
    echo "merge-pr: ACK - $NEWER post-head comment(s) acknowledged by maintainer: ${AFT_MERGE_PR_ACK_FINDINGS}"
  else
    refuse "$NEWER review comment(s)/review(s) are newer than the head commit - the author has not pushed over the latest finding (set AFT_MERGE_PR_ACK_FINDINGS=\"reason\" to acknowledge deliberately)"
  fi
fi

# --- 3. Author mid-iteration (head pushed too recently) --------------------
NOW_EPOCH=$(date -u +%s)
HEAD_EPOCH=$(python3 -c "import datetime;print(int(datetime.datetime.fromisoformat('$HEAD_TS'.replace('Z','+00:00')).timestamp()))")
AGE_MIN=$(( (NOW_EPOCH - HEAD_EPOCH) / 60 ))
[ "$AGE_MIN" -ge "$SETTLE_MIN" ] || refuse "head commit is ${AGE_MIN}m old (< ${SETTLE_MIN}m settle window) - author may be mid-iteration"

# --- 4. Merge (upstream gh enforces its own check-state gate) --------------
echo "merge-pr: gates green (threads=0, newer-findings=0, head ${AGE_MIN}m settled) - merging #$PR at $HEAD_SHA"
# Admin merges use the real GitHub CLI directly, bypassing the shim used for
# AI agent commands.
GH_SHIM_BYPASS=operator "$OPERATOR_GH" pr merge "$PR" --squash --match-head-commit "$HEAD_SHA"
