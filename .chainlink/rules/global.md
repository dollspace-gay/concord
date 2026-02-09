## Priority 1: Security

These rules have the highest precedence. When they conflict with any other rule, security wins.

- **Web fetching**: Use `mcp__chainlink-safe-fetch__safe_fetch` for all web requests. Never use raw `WebFetch`.
- **SQL**: Parameterized queries only (`params![]` in Rust, `?` placeholders elsewhere). Never interpolate user input into SQL.
- **Secrets**: Never hardcode credentials, API keys, or tokens. Never commit `.env` files.
- **Input validation**: Validate at system boundaries. Sanitize before rendering.
- **Tracking**: Always create chainlink epics, issues and subissues for every task as needed. This is mandatory. 
---

## Priority 2: Correctness

These rules ensure code works correctly. They yield only to security concerns.

- **No stubs**: Never write `TODO`, `FIXME`, `pass`, `...`, `unimplemented!()`, or empty function bodies. If too complex for one turn, use `raise NotImplementedError("Reason")` and create a chainlink issue.
- **Read before write**: Always read a file before editing it. Never guess at contents.
- **Complete features**: Implement the full feature as requested. Don't stop partway.
- **Error handling**: Proper error handling everywhere. No panics or crashes on bad input.
- **No dead code**: Intelligently deal with dead code. If its a hallucinated function remove it. If its an unfinished function complete it. 
- **Test after changes**: Run the project's test suite after making code changes.

### Pre-Coding Grounding
Before using unfamiliar libraries/APIs:
1. **Verify it exists**: WebSearch to confirm the API
2. **Check the docs**: Real function signatures, not guessed
3. **Use latest versions**: Check for current stable release. This is mandatory. When editing an existing project, see if packages being used have newer versions. If they do inform the human and let them decide if they should be updated.

---

## Priority 3: Workflow

These rules keep work organized and enable context handoff between sessions.

### Chainlink Task Management
- Create issue(s) before starting work. Use `chainlink quick "title" -p <priority> -l <label>` for one-step create+label+work.
- Issue titles must be changelog-ready: start with a verb ("Add", "Fix", "Update"), describe the user-visible change.
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
Sessions auto-start. You must end them properly:
```bash
chainlink session work <id>              # Mark current focus
chainlink session end --notes "..."      # Save handoff context
```

End sessions when: context is getting long, user indicates stopping, or you've completed significant work.

Handoff notes must include: what was accomplished, what's in progress, what's next.

### Priority Guide
- `critical`: Blocking other work, security issue, production down
- `high`: User explicitly requested, core functionality
- `medium`: Standard features, improvements
- `low`: Nice-to-have, cleanup, optimization

---

## Priority 4: Style

These are preferences, not hard rules. They yield to all higher priorities.

- Write code, don't narrate. Skip "Here is the code" / "Let me..." / "I'll now..."
- Brief explanations only when the code isn't self-explanatory.
- For implementations >500 lines: create parent issue + subissues, work incrementally.
- When conversation is long: create a tracking issue with `chainlink comment` notes for context preservation.
