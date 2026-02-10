## Chainlink Task Management

Create issues before starting work to keep things organized and enable context handoff between sessions.

### Creating Issues
- Use `chainlink quick "title" -p <priority> -l <label>` for one-step create+label+work.
- Issue titles should be changelog-ready: start with a verb ("Add", "Fix", "Update"), describe the user-visible change.
- Add labels for changelog categories: `bug`/`fix` → Fixed, `feature`/`enhancement` → Added, `breaking` → Changed, `security` → Security.
- For multi-part features: create parent issue + subissues. Work one at a time.
- Add context as you discover things: `chainlink comment <id> "..."`

### Labels for Changelog Categories
- `bug`, `fix` → **Fixed**
- `feature`, `enhancement` → **Added**
- `breaking`, `breaking-change` → **Changed**
- `security` → **Security**
- `deprecated` → **Deprecated**
- `removed` → **Removed**
- (no label) → **Changed** (default)

### Quick Reference
```bash
# One-step create + label + start working
chainlink quick "Fix auth timeout" -p high -l bug

# Or use create with flags
chainlink create "Add dark mode" -p medium --label feature --work

# Multi-part feature
chainlink create "Add user auth" -p high --label feature
chainlink subissue 1 "Add registration endpoint"
chainlink subissue 1 "Add login endpoint"

# Track progress
chainlink session work <id>
chainlink comment <id> "Found existing helper in utils/"

# Close (auto-updates CHANGELOG.md)
chainlink close <id>
chainlink close <id> --no-changelog    # Skip changelog for internal work
chainlink close-all --no-changelog     # Batch close

# Quiet mode for scripting
chainlink -q create "Fix bug" -p high  # Outputs just the ID number
```

### Session Management
Sessions auto-start. End them properly when you can:
```bash
chainlink session work <id>              # Mark current focus
chainlink session end --notes "..."      # Save handoff context
```

End sessions when: context is getting long, user indicates stopping, or you've completed significant work.

Handoff notes should include: what was accomplished, what's in progress, what's next.

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
