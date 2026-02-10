## Chainlink Task Management (MANDATORY — ABSOLUTE RULE)

**You MUST use chainlink to track ALL work. This is NOT optional. This is NOT a suggestion. This is an ABSOLUTE REQUIREMENT.**

**YOU MUST CREATE A CHAINLINK ISSUE BEFORE WRITING ANY CODE. NO EXCEPTIONS.**

Before your FIRST Write, Edit, or Bash tool call that modifies code:
1. Run `chainlink quick "title" -p <priority> -l <label>` to create an issue AND start working on it
2. The PreToolUse hook WILL BLOCK your tool calls if no issue is active
3. NEVER skip this step. NEVER proceed without an issue. NEVER treat this as optional.

### On Every User Request
1. Create issue(s) BEFORE starting work — ON ALL ISSUES NO MATTER WHAT. THIS IS A CRITICAL OVERRIDE.
2. When creating the issue, add as much detail as possible with comments so any new agent instance will understand the plan.

### Issue Title Requirements (CHANGELOG-READY)
Issue titles are automatically added to CHANGELOG.md when closed. Write titles that:
- Describe the user-visible change (not implementation details)
- Start with a verb: "Add", "Fix", "Update", "Remove", "Improve"
- Are complete sentences (but no period)

**GOOD titles** (changelog-ready):
- "Add dark mode toggle to settings page"
- "Fix authentication timeout on slow connections"
- "Update password requirements to include special characters"

**BAD titles** (implementation-focused):
- "auth.ts changes"
- "Fix bug"
- "Update code"
- "WIP feature"

### Labels for Changelog Categories
Add labels to control CHANGELOG.md section:
- `bug`, `fix` → **Fixed**
- `feature`, `enhancement` → **Added**
- `breaking`, `breaking-change` → **Changed**
- `security` → **Security**
- `deprecated` → **Deprecated**
- `removed` → **Removed**
- (no label) → **Changed** (default)

### Task Breakdown Rules
```bash
# Single task — use quick for create + label + work in one step
chainlink quick "Fix login validation error on empty email" -p medium -l bug

# Or use create with flags
chainlink create "Fix login validation error on empty email" -p medium --label bug --work

# Multi-part feature → Epic with subissues
chainlink create "Add user authentication system" -p high --label feature
chainlink subissue 1 "Add user registration endpoint"
chainlink subissue 1 "Add login endpoint with JWT tokens"
chainlink subissue 1 "Add session middleware for protected routes"

# Mark what you're working on
chainlink session work 1

# Add context as you discover things
chainlink comment 1 "Found existing auth helper in utils/auth.ts"

# Close when done — auto-updates CHANGELOG.md
chainlink close 1

# Skip changelog for internal/refactor work
chainlink close 1 --no-changelog

# Batch close
chainlink close-all --no-changelog

# Quiet mode for scripting
chainlink -q create "Fix bug" -p high  # Outputs just the ID number
```

### Memory-Driven Planning (CRITICAL)

Your auto-memory directory (`~/.claude/projects/.../memory/`) contains plans, architecture notes, and context from prior sessions. **You MUST consult memory before creating issues.**

1. **Read memory first**: At session start, read `MEMORY.md` and any linked topic files. These contain the current plan of record.
2. **Translate plans to issues**: Break memory plans into small, concrete chainlink issues/epics/subissues. Each subissue should be completable in a single focused session.
3. **Verbose comments are mandatory**: When creating issues from a memory plan, add comments that quote or reference the specific plan section, rationale, and acceptance criteria so any new agent instance can pick up the work without re-reading memory.
4. **Stay on track**: Before starting new work, check if it aligns with the plan in memory. If the user's request diverges from the plan, update memory AND issues together — never let them drift apart.
5. **Close the loop**: When closing an issue, update memory to reflect what was completed and what changed from the original plan.

```bash
# Example: translating a memory plan into tracked work
chainlink create "Implement webhook retry system" -p high --label feature
chainlink comment 1 "Per memory/architecture.md: retry with exponential backoff, max 5 attempts, dead-letter queue after exhaustion. See 'Webhook Reliability' section."
chainlink subissue 1 "Add retry queue with exponential backoff (max 5 attempts)"
chainlink comment 2 "Backoff schedule: 1s, 5s, 25s, 125s, 625s. Store attempt count in webhook_deliveries table."
chainlink subissue 1 "Add dead-letter queue for exhausted retries"
chainlink comment 3 "Failed webhooks go to dead_letter_webhooks table with full payload + error history for manual inspection."
chainlink subissue 1 "Add webhook delivery dashboard endpoint"
```

### When to Create Issues
| Scenario | Action |
|----------|--------|
| User asks for a feature | Create epic + subissues if >2 components |
| User reports a bug | Create issue, investigate, add comments |
| Task has multiple steps | Create subissues for each step |
| Work will span sessions | Create issue with detailed comments |
| You discover related work | Create linked issue |
| Memory contains a plan | Translate plan into epic + subissues with verbose comments |

### Session Management (MANDATORY)

Sessions are auto-started by the SessionStart hook. **You MUST end sessions properly.**

```bash
chainlink session work <id>          # Mark current focus — ALWAYS
chainlink session end --notes "..."  # REQUIRED before stopping — ALWAYS
```

**You MUST run `chainlink session end --notes "..."` when:**
- Context is getting long (conversation > 30-40 messages)
- User says goodbye, done, thanks, or indicates stopping
- Before any natural stopping point
- You've completed a significant piece of work

**Handoff notes MUST include:**
- What was accomplished this session
- What's in progress or blocked
- What should be done next

### Priority Guide
- `critical`: Blocking other work, security issue, production down
- `high`: User explicitly requested, core functionality
- `medium`: Standard features, improvements
- `low`: Nice-to-have, cleanup, optimization

### Dependencies
```bash
chainlink block 2 1     # Issue 2 blocked by issue 1
chainlink ready         # Show unblocked work
```

### Large Implementations (500+ lines)
1. Create parent issue: `chainlink create "<feature>" -p high`
2. Break into subissues: `chainlink subissue <id> "<component>"`
3. Work one subissue at a time, close each when done

### Context Window Management
When conversation is long or task needs many steps:
1. Create tracking issue: `chainlink create "Continue: <summary>" -p high`
2. Add notes: `chainlink comment <id> "<what's done, what's next>"`
