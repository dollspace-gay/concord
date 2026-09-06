# Config implementation

Keep implementation subdivisions cohesive with the public ../config.rs entry point.

Retain private-file checks, environment precedence, path resolution, recovery-mode allowances, and startup validation.

Run the owning unit tests and strict server checks; use the corresponding integration target when runtime behavior is touched.
