# Database row models

Group row and parameter types by domain while preserving re-exports from ../models.rs. Keep SQL field names and nullability aligned with their queries.

These are persistence representations. Avoid importing transport handlers or adding domain side effects to a model.

Run Rust checks and the affected query/migration tests; a type move must not change stored data.
