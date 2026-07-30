#!/bin/bash
# PreToolUse hook (Bash) — technically enforces SOUL.md's Tier 2 gate for the
# obvious, direct forms of the three actions it names as never-autonomous for
# this repo: pushing to a remote, merging into a default/production branch,
# and publishing a package. This blocks the direct command; it does not
# inspect scripts written to disk and executed later, so it raises the bar
# rather than making bypass impossible — see ARCHITECTURE.md and
# HOW_THE_COMPANY_WORKS.md §8 for the honest scope of this guarantee.
set -euo pipefail

COMMAND=$(jq -r '.tool_input.command // empty')
HOOK_CWD=$(jq -r '.cwd // empty')

deny() {
  jq -n --arg reason "$1" '{
    hookSpecificOutput: {
      hookEventName: "PreToolUse",
      permissionDecision: "deny",
      permissionDecisionReason: $reason
    }
  }'
  exit 0
}

if echo "$COMMAND" | grep -qE '(^|[;&|]\s*)git\s+push\b'; then
  deny "SOUL.md Tier 2: pushing to a remote requires human approval. Ask the operator to run this themselves."
fi

# Text matching alone misses the realistic case: "main"/"master" is the
# *source* text in `git merge main`, but never appears at all in the far more
# common risky form `git merge feature-branch` run *while already on* main.
# So also check actual git state: deny any merge/rebase attempted while the
# current branch is a default/production one, regardless of wording.
if echo "$COMMAND" | grep -qE '(^|[;&|]\s*)git\s+(merge|rebase)\b'; then
  CURRENT_BRANCH=$(git -C "${HOOK_CWD:-.}" branch --show-current 2>/dev/null || echo "")
  if echo "$COMMAND" | grep -qE '\b(main|master)\b' || [ "$CURRENT_BRANCH" = "main" ] || [ "$CURRENT_BRANCH" = "master" ]; then
    deny "SOUL.md Tier 2: merging/rebasing into a default/production branch requires human approval (current branch: ${CURRENT_BRANCH:-unknown})."
  fi
fi

if echo "$COMMAND" | grep -qE '\b(npm|pnpm|yarn)\s+publish\b|cargo\s+publish\b'; then
  deny "SOUL.md Tier 2/3: publishing a package is not an autonomous action."
fi

# SOUL.md Tier 2 also names "installing new deps." "add" subcommands always
# introduce a new dependency; "install"/"i" only do when given a bare package
# name (a flags-only or bare invocation is just a lockfile sync, which is
# routine and not gated). pip's -r/--requirement form is the lockfile-sync
# equivalent and stays allowed for the same reason.
if echo "$COMMAND" | grep -qE '(^|[;&|]\s*)(npm|pnpm|yarn)\s+add\b'; then
  deny "SOUL.md Tier 2: installing a new dependency requires human approval."
fi
if echo "$COMMAND" | grep -qE '(^|[;&|]\s*)(npm|pnpm)\s+(install|i)\s+[a-zA-Z@]'; then
  deny "SOUL.md Tier 2: installing a new dependency requires human approval."
fi
if echo "$COMMAND" | grep -qE '(^|[;&|]\s*)cargo\s+(add|install)\b'; then
  deny "SOUL.md Tier 2: installing a new dependency requires human approval."
fi
if echo "$COMMAND" | grep -qE '(^|[;&|]\s*)pip[3]?\s+install\b' && ! echo "$COMMAND" | grep -qE '\s(-r|--requirement)\b'; then
  deny "SOUL.md Tier 2: installing a new dependency requires human approval."
fi

exit 0
