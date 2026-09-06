# Browser application

src/ contains React views, API adapters, and state owners. tests/browser uses a controlled harness; tests/authenticated exercises a real fixture server.

Preserve keyboard accessibility, stable selectors, account isolation, correlated command outcomes, and private-state invalidation. Generated contracts are maintained by the generator, not hand edits.

Run npm --prefix concord/web run lint, npm --prefix concord/web run build, and relevant Playwright tests. Run scripts/check-contract.sh when protocol representations change.
