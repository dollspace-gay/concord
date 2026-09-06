# Workspace architecture

The Rust workspace is in this directory; the browser application is in web/. Preserve public imports, wire contracts, persistent data, and observable behavior during structural changes.

Use a soft 500-line budget for handwritten source. Split by responsibility; an exception needs a specific reason and maximum in .design/maintainability/size-exceptions.json. Never use numbered chunks, include files, compressed formatting, or warning suppression to meet a count.

From the repository root run python3 scripts/check-maintainability.py. Follow the narrower server or web guidance for validation.
