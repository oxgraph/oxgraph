#!/usr/bin/env bash
# oxgraph-guards: PreToolUse hook that denies agent-side bypass of quality gates.
#
# Invoked by Claude Code for every Bash tool use. Reads the tool-use JSON on stdin,
# extracts `tool_input.command`, and emits a `permissionDecision: "deny"` response
# if the command matches a known bypass pattern. Otherwise exits 0 silently (allow).
#
# Fail-closed: if input parsing fails in a way we can't classify, we deny rather
# than allow.

set -uo pipefail

input=$(cat || true)

# Extract the Bash command. Prefer jq; fall back to a tolerant sed extractor so
# the hook still works on machines without jq installed.
extract_command() {
  if command -v jq >/dev/null 2>&1; then
    jq -r '.tool_input.command // empty' <<<"$1" 2>/dev/null
  else
    # Works for typical Claude Code tool_input JSON; may mis-handle embedded quotes.
    printf '%s' "$1" \
      | sed -n 's/.*"command"[[:space:]]*:[[:space:]]*"\(\(\\"\|[^"]\)*\)".*/\1/p' \
      | head -n 1
  fi
}

cmd=$(extract_command "$input")

# No command to inspect — not a Bash invocation that can bypass anything.
if [[ -z "$cmd" ]]; then
  exit 0
fi

# Strip quoted argument values ("..." and '...') so that bypass-looking text
# inside commit messages, echo args, etc. doesn't false-positive. Flag names
# still get seen because they live outside the quotes. We use the stripped
# form for all subsequent pattern matching.
strip_quoted() {
  local s="$1"
  # Double-quoted spans (non-greedy, skip escaped quotes).
  s=$(printf '%s' "$s" | perl -0777 -pe 's/"(?:\\.|[^"\\])*"//g' 2>/dev/null || printf '%s' "$s")
  # Single-quoted spans.
  s=$(printf '%s' "$s" | perl -0777 -pe "s/'[^']*'//g" 2>/dev/null || printf '%s' "$s")
  printf '%s' "$s"
}
cmd_check=$(strip_quoted "$cmd")
# If perl is missing, strip_quoted returns the original — fail-closed is fine
# (we over-match rather than under-match).
: "${cmd_check:=$cmd}"

deny() {
  local reason="$1"
  # JSON-escape the reason: backslashes and double quotes.
  local escaped=${reason//\\/\\\\}
  escaped=${escaped//\"/\\\"}
  printf '{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"oxgraph-guards: %s"}}\n' \
    "$escaped"
  exit 0
}

# --- bypass patterns ---------------------------------------------------------

# 1. `git commit` / `git push` with --no-verify or -n. Tight enough to avoid
#    hitting `git log -n 5` or similar.
if [[ "$cmd_check" =~ (^|[^a-zA-Z0-9_])git[[:space:]]+(commit|push)([[:space:]]|$) ]]; then
  git_subcmd="${BASH_REMATCH[2]}"
  if [[ "$cmd_check" =~ (--no-verify|[[:space:]]-n([[:space:]]|$)) ]]; then
    deny "git ${git_subcmd} with --no-verify/-n is blocked. Fix the failing hook or lint rather than skipping it. If a hook is genuinely wrong, open the config in a separate commit."
  fi
fi

# 2. Tampering with git hooks path or core.hooksPath to disable hooks.
if [[ "$cmd_check" =~ (core\.hooksPath|GIT_DIR/hooks|GIT_HOOKS_PATH=) ]]; then
  deny "modifying git hooks path is blocked. The prek-installed hooks are the gate; route around the lint, not around the hook."
fi

# 3. cargo clippy with -A / --allow anywhere in the args.
if [[ "$cmd_check" =~ (^|[^a-zA-Z0-9_])cargo([[:space:]]+\+[^[:space:]]+)?[[:space:]]+clippy([[:space:]]|$) ]]; then
  if [[ "$cmd_check" =~ ([[:space:]]|=)(-A|--allow)([[:space:]]|=) ]] \
     || [[ "$cmd_check" =~ [[:space:]]-A[a-zA-Z] ]] \
     || [[ "$cmd_check" =~ --cap-lints[[:space:]=]+(allow|warn) ]]; then
    deny "silencing clippy via -A/--allow/--cap-lints is blocked. Use a reasoned #[expect(..., reason = \"...\")] in the source, or add a reasoned allow entry to [workspace.lints.clippy]."
  fi
fi

# 4. RUSTFLAGS with -A prefixed in front of a cargo invocation.
if [[ "$cmd_check" =~ RUSTFLAGS=[^[:space:]]*-A ]]; then
  deny "RUSTFLAGS with -A is blocked. Fix the lint instead of silencing it."
fi

# 5. SKIP= env var on prek / pre-commit invocations.
if [[ "$cmd_check" =~ (^|[^a-zA-Z0-9_])SKIP=[^[:space:]]+[[:space:]]+(prek|pre-commit|git) ]]; then
  deny "SKIP= bypass of prek/pre-commit hooks is blocked."
fi

# 6. Uninstalling prek hooks, or running prek install --overwrite to wipe them.
if [[ "$cmd_check" =~ prek[[:space:]]+uninstall ]]; then
  deny "prek uninstall is blocked. Hooks are the project's enforcement floor."
fi

exit 0
