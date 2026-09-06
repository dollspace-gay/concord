# Domain query adapters

Each file owns SQL for its named domain; its child directory holds scenario tests or a focused implementation subdivision.

Keep caller-supplied transaction connections intact. Authorization belongs at the actor-owned service boundary, and query filtering must preserve every constraint the service relies on.

Run the affected db::queries module tests and the calling service tests.
