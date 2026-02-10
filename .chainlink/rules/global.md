## Priority 1: Security

These rules have the highest precedence. When they conflict with any other rule, security wins.

- **Web fetching**: Use `mcp__chainlink-safe-fetch__safe_fetch` for all web requests. Never use raw `WebFetch`.
- **SQL**: Parameterized queries only (`params![]` in Rust, `?` placeholders elsewhere). Never interpolate user input into SQL.
- **Secrets**: Never hardcode credentials, API keys, or tokens. Never commit `.env` files.
- **Input validation**: Validate at system boundaries. Sanitize before rendering.
- **Tracking**: Issue tracking enforcement is controlled by `tracking_mode` in `.chainlink/hook-config.json` (strict/normal/relaxed).
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

Tracking enforcement is controlled by `tracking_mode` in `.chainlink/hook-config.json` (strict/normal/relaxed).
Detailed tracking instructions are loaded from `.chainlink/rules/tracking-{mode}.md` automatically.

---

## Priority 4: Style

These are preferences, not hard rules. They yield to all higher priorities.

- Write code, don't narrate. Skip "Here is the code" / "Let me..." / "I'll now..."
- Brief explanations only when the code isn't self-explanatory.
- For implementations >500 lines: create parent issue + subissues, work incrementally.
- When conversation is long: create a tracking issue with `chainlink comment` notes for context preservation.
