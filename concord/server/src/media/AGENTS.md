# Media implementation

Keep implementation subdivisions cohesive with the public ../media.rs entry point.

Keep rooted filesystem operations, quotas, durable media states, bounded collection, and recoverable legacy import separate.

Run the owning unit tests and strict server checks; use the corresponding integration target when runtime behavior is touched.
