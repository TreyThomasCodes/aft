#!/usr/bin/env bash
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

OWNER_REPO=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
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
UNRESOLVED=$(gh api graphql -f query='
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
HEAD_SHA=$(gh pr view "$PR" --json headRefOid --jq .headRefOid)
HEAD_TS=$(gh api "repos/$OWNER_REPO/commits/$HEAD_SHA" --jq .commit.committer.date)
NEWER=$(
  {
    gh api "repos/$OWNER_REPO/pulls/$PR/comments" --paginate --jq '.[].created_at'
    gh api "repos/$OWNER_REPO/pulls/$PR/reviews" --paginate --jq '.[] | select(.state != "APPROVED" and .state != "PENDING") | .submitted_at'
  } | awk -v head="$HEAD_TS" '$0 > head' | wc -l | tr -d ' '
)
[ "$NEWER" -eq 0 ] || refuse "$NEWER review comment(s)/review(s) are newer than the head commit - the author has not pushed over the latest finding"

# --- 3. Author mid-iteration (head pushed too recently) --------------------
NOW_EPOCH=$(date -u +%s)
HEAD_EPOCH=$(python3 -c "import datetime;print(int(datetime.datetime.fromisoformat('$HEAD_TS'.replace('Z','+00:00')).timestamp()))")
AGE_MIN=$(( (NOW_EPOCH - HEAD_EPOCH) / 60 ))
[ "$AGE_MIN" -ge "$SETTLE_MIN" ] || refuse "head commit is ${AGE_MIN}m old (< ${SETTLE_MIN}m settle window) - author may be mid-iteration"

# --- 4. Merge (gh enforces its own check-state gate) -----------------------
echo "merge-pr: gates green (threads=0, newer-findings=0, head ${AGE_MIN}m settled) - merging #$PR at $HEAD_SHA"
gh pr merge "$PR" --squash --match-head-commit "$HEAD_SHA"
