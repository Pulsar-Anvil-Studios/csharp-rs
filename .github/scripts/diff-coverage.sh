#!/usr/bin/env bash
set -euo pipefail

# diff-coverage.sh — Compute diff coverage from lcov and post PR comment
# Usage: diff-coverage.sh <lcov-file> <base-sha> <pr-number> [threshold]

LCOV_FILE="${1:?Usage: diff-coverage.sh <lcov-file> <base-sha> <pr-number> [threshold]}"
BASE_SHA="${2:?Usage: diff-coverage.sh <lcov-file> <base-sha> <pr-number> [threshold]}"
PR_NUMBER="${3:?Usage: diff-coverage.sh <lcov-file> <base-sha> <pr-number> [threshold]}"
THRESHOLD="${4:-98}"

COMMENT_MARKER="<!-- coverage-report -->"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

# Group consecutive line numbers: "1 2 3 5 7 8 9" → "L1-3, L5, L7-9"
group_lines() {
    local nums=("$@")
    local result="" start="" prev=""

    for n in "${nums[@]}"; do
        if [[ -z "$start" ]]; then
            start=$n; prev=$n
        elif (( n == prev + 1 )); then
            prev=$n
        else
            if (( start == prev )); then
                result="${result:+$result, }L$start"
            else
                result="${result:+$result, }L$start-$prev"
            fi
            start=$n; prev=$n
        fi
    done

    if [[ -n "$start" ]]; then
        if (( start == prev )); then
            result="${result:+$result, }L$start"
        else
            result="${result:+$result, }L$start-$prev"
        fi
    fi

    echo "$result"
}

post_or_update_comment() {
    local body="$1"

    local existing_id
    existing_id=$(gh api "repos/${GITHUB_REPOSITORY}/issues/${PR_NUMBER}/comments" \
        --paginate \
        --jq '.[] | select(.body | startswith("<!-- coverage-report -->")) | .id' \
        2>/dev/null | head -1 || true)

    if [[ -n "$existing_id" ]]; then
        gh api "repos/${GITHUB_REPOSITORY}/issues/comments/${existing_id}" \
            -X PATCH -f body="$body" > /dev/null
        echo "Updated existing coverage comment."
    else
        gh pr comment "$PR_NUMBER" --body "$body"
        echo "Posted new coverage comment."
    fi
}

# ---------------------------------------------------------------------------
# Collect changed Rust files
# ---------------------------------------------------------------------------

CHANGED_RS=$(git diff --name-only "$BASE_SHA"...HEAD -- '*.rs' 2>/dev/null || true)

if [[ -z "$CHANGED_RS" ]]; then
    echo "No Rust files changed."
    BODY="${COMMENT_MARKER}
## Coverage Report

No Rust files changed — diff coverage check skipped."
    post_or_update_comment "$BODY"
    exit 0
fi

# ---------------------------------------------------------------------------
# Parse lcov for changed files
# ---------------------------------------------------------------------------

TOTAL_LINES=0
TOTAL_HIT=0
TABLE_ROWS=""

while IFS= read -r changed_file; do
    [[ -z "$changed_file" ]] && continue

    in_file=false
    file_lines=0
    file_hit=0
    uncovered_nums=()

    while IFS= read -r line; do
        case "$line" in
            SF:*)
                sf="${line#SF:}"
                if [[ "$sf" == *"$changed_file" ]]; then
                    in_file=true
                    file_lines=0; file_hit=0; uncovered_nums=()
                else
                    in_file=false
                fi
                ;;
            DA:*)
                if $in_file; then
                    da="${line#DA:}"
                    ln="${da%%,*}"
                    cnt="${da#*,}"
                    file_lines=$((file_lines + 1))
                    if (( cnt > 0 )); then
                        file_hit=$((file_hit + 1))
                    else
                        uncovered_nums+=("$ln")
                    fi
                fi
                ;;
            end_of_record)
                if $in_file && (( file_lines > 0 )); then
                    TOTAL_LINES=$((TOTAL_LINES + file_lines))
                    TOTAL_HIT=$((TOTAL_HIT + file_hit))

                    pct=$(awk -v h="$file_hit" -v t="$file_lines" \
                        'BEGIN { printf "%.1f", (h/t)*100 }')

                    if (( ${#uncovered_nums[@]} > 0 )); then
                        grouped=$(group_lines "${uncovered_nums[@]}")
                    else
                        grouped="-"
                    fi

                    TABLE_ROWS="${TABLE_ROWS}| \`${changed_file}\` | ${pct}% (${file_hit}/${file_lines}) | ${grouped} |
"
                fi
                in_file=false
                ;;
        esac
    done < "$LCOV_FILE"
done <<< "$CHANGED_RS"

# ---------------------------------------------------------------------------
# Compute diff coverage percentage
# ---------------------------------------------------------------------------

if (( TOTAL_LINES == 0 )); then
    DIFF_PCT="100.0"
else
    DIFF_PCT=$(awk -v h="$TOTAL_HIT" -v t="$TOTAL_LINES" \
        'BEGIN { printf "%.1f", (h/t)*100 }')
fi

DIFF_PASS=$(awk -v p="$DIFF_PCT" -v t="$THRESHOLD" \
    'BEGIN { print (p >= t) ? "1" : "0" }')

if (( DIFF_PASS )); then
    DIFF_STATUS="Pass"
    EXIT_CODE=0
else
    DIFF_STATUS="Fail"
    EXIT_CODE=1
fi

# ---------------------------------------------------------------------------
# Read global coverage from lcov
# ---------------------------------------------------------------------------

GLOBAL_LF=0
GLOBAL_LH=0
while IFS= read -r line; do
    case "$line" in
        LF:*) GLOBAL_LF=$((GLOBAL_LF + ${line#LF:})) ;;
        LH:*) GLOBAL_LH=$((GLOBAL_LH + ${line#LH:})) ;;
    esac
done < "$LCOV_FILE"

if (( GLOBAL_LF > 0 )); then
    GLOBAL_PCT=$(awk -v h="$GLOBAL_LH" -v t="$GLOBAL_LF" \
        'BEGIN { printf "%.1f", (h/t)*100 }')
else
    GLOBAL_PCT="100.0"
fi

GLOBAL_PASS=$(awk -v p="$GLOBAL_PCT" -v t="$THRESHOLD" \
    'BEGIN { print (p >= t) ? "1" : "0" }')

if (( GLOBAL_PASS )); then
    GLOBAL_STATUS="Pass"
else
    GLOBAL_STATUS="Fail"
fi

# ---------------------------------------------------------------------------
# Build and post PR comment
# ---------------------------------------------------------------------------

BODY="${COMMENT_MARKER}
## Coverage Report

| Metric | Value | Threshold | Status |
|--------|-------|-----------|--------|
| Global | ${GLOBAL_PCT}% | ${THRESHOLD}% | ${GLOBAL_STATUS} |
| Diff | ${DIFF_PCT}% | ${THRESHOLD}% | ${DIFF_STATUS} |

### Changed Files

| File | Coverage | Uncovered Lines |
|------|----------|-----------------|
${TABLE_ROWS}"

post_or_update_comment "$BODY"

echo "Diff coverage: ${DIFF_PCT}% (threshold: ${THRESHOLD}%)"
exit "$EXIT_CODE"
