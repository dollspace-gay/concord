# Egress implementation

Keep implementation subdivisions cohesive with the public ../egress.rs entry point.

Every destination and redirect must retain DNS/IP/origin restrictions, bounded bodies, timeouts, admission, and credential scoping.

Run the owning unit tests and strict server checks; use the corresponding integration target when runtime behavior is touched.
