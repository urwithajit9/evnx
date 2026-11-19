#!/usr/bin/env bash
# Trace every winget-pkgs PR for urwithajit9.evnx: was it blocked, why, how it was cleared.
# Usage: scripts/trace-winget-history.sh > winget-history.md   (see docs/RELEASING.md, "Winget validation")
# Needs: gh (logged in), jq. Read-only: it only lists PRs, events and comments.
#
# Why events and not labels: winget removes Validation-Defender-Error once a PR is
# cleared, so a merged PR's final labels look the same whether it was blocked or not.
# The label *events* keep the history (docs/RELEASING.md, "Winget validation").

# No `set -e`: one odd PR (a title without a version, an API hiccup) must not end the run.
set -u
REPO=microsoft/winget-pkgs
PKG=urwithajit9.evnx

prs=$(gh pr list -R "$REPO" --state all --limit 200 \
  --search "$PKG in:title" --json number,title,state,createdAt,mergedAt,closedAt,author \
  | jq -c 'sort_by(.createdAt)[]')

echo "# winget history for $PKG"
echo
echo "Generated $(date -u +%Y-%m-%dT%H:%MZ) from $REPO."
echo
echo "| PR | Version | Opened | Result | Blocked by | Cleared by | Days to merge |"
echo "|---|---|---|---|---|---|---|"

details=""
while IFS= read -r pr; do
  [ -z "$pr" ] && continue
  n=$(jq -r .number <<<"$pr")
  title=$(jq -r .title <<<"$pr")
  ver=$(grep -oE '[0-9]+\.[0-9]+\.[0-9]+' <<<"$title" | head -1 || true)
  [ -z "$ver" ] && ver="($title)"
  opened=$(jq -r '.createdAt[0:10]' <<<"$pr")
  merged=$(jq -r '.mergedAt // ""' <<<"$pr")
  state=$(jq -r .state <<<"$pr")

  events=$(gh api --paginate "repos/$REPO/issues/$n/events" 2>/dev/null \
    | jq -s -c 'add // [] | [.[] | select(.event=="labeled" or .event=="unlabeled") | {e:.event, l:.label.name, by:.actor.login, at:.created_at[0:16]}]' 2>/dev/null)
  comments=$(gh api --paginate "repos/$REPO/issues/$n/comments" 2>/dev/null \
    | jq -s -c 'add // [] | [.[] | {by:.user.login, at:.created_at[0:16], body:.body}]' 2>/dev/null)
  [ -z "$events" ] && events='[]' && echo "warning: could not read events of #$n" >&2
  [ -z "$comments" ] && comments='[]' && echo "warning: could not read comments of #$n" >&2

  added() { jq -r --arg l "$1" '[.[] | select(.e=="labeled" and .l==$l)] | length' <<<"$events"; }
  defender=$(added Validation-Defender-Error)
  feedback=$(added Needs-Author-Feedback)
  moderator=$(added Moderator-Approved)

  # Why it was blocked: look for the known texts in the comments.
  why=""
  jq -e '[.[] | select(.body|test("Sprisky";"i"))] | length > 0' <<<"$comments" >/dev/null && why="Defender ML verdict (Sprisky)"
  jq -e '[.[] | select(.body|test("Another scan already in progress|race condition";"i"))] | length > 0' <<<"$comments" >/dev/null && why="${why:+$why; }Defender race on the test VM"
  [ -z "$why" ] && [ "$defender" -gt 0 ] && why="Defender error (see comments)"
  [ -z "$why" ] && [ "$feedback" -gt 0 ] && why="Needs-Author-Feedback (see comments)"
  [ -z "$why" ] && why="-"

  # How it was cleared.
  how="-"
  if [ "$defender" -gt 0 ] || [ "$feedback" -gt 0 ]; then
    who=$(jq -r '[.[] | select(.e=="unlabeled" and (.l=="Validation-Defender-Error" or .l=="Needs-Author-Feedback"))][0].by // ""' <<<"$events")
    case "$who" in
      "" ) how="not cleared" ;;
      *bot*|*policy-service* ) how="re-validation passed ($who)" ;;
      * ) how="moderator $who" ;;
    esac
  fi
  # Moderator-Approved on a clean PR is the normal sign-off for community PRs, not a fix.
  [ "$moderator" -gt 0 ] && [ "$how" = "-" ] && how="- (normal moderator sign-off)"

  if [ -n "$merged" ]; then
    result="merged ${merged:0:10}"
    days=$(( ( $(date -d "$merged" +%s) - $(date -d "$(jq -r .createdAt <<<"$pr")" +%s) ) / 86400 )) || days="?"
  else
    result="$state (not merged)"; days="-"
  fi

  echo "| [#$n](https://github.com/$REPO/pull/$n) | ${ver:--} | $opened | $result | $why | $how | $days |"

  # Keep the label timeline of anything that was not a clean pass, for the record.
  if [ "$why" != "-" ] || [ -z "$merged" ]; then
    details+=$'\n'"### #$n ($ver)"$'\n\n'"\`\`\`text"$'\n'
    details+=$(jq -r '.[] | "\(.at)  \(.e) \(.l)  by \(.by)"' <<<"$events")
    details+=$'\n'"\`\`\`"$'\n\n'"What the bot and moderators said (first lines):"$'\n\n'"\`\`\`text"$'\n'
    details+=$(jq -r '.[] | select(((.by|test("policy-service|wingetbot|winget";"i"))|not) or (.body|test("Defender|Sprisky|scan|blocked|Validation";"i"))) | "\(.at) \(.by): \(.body | gsub("\r";"") | split("\n") | map(select(length>0)) | .[0:4] | join(" / ") | .[0:400])"' <<<"$comments")
    details+=$'\n'"\`\`\`"$'\n'
  fi
done <<<"$prs"

echo
echo "## Label timelines of the PRs that were held up"
echo "$details"
