# Parser implementation

Keep implementation subdivisions cohesive with the public ../parser.rs entry point.

Retain the parent module interface, ownership boundaries, cancellation, and error behavior. Shared helpers stay narrowly visible to this implementation.

Run the owning unit tests and strict server checks; use the corresponding integration target when runtime behavior is touched.
