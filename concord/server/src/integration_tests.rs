//! Integration tests for Concord — cross-layer tests that verify end-to-end flows,
//! migration correctness, and system-level behavior.
//!
//! Each test creates its own in-memory SQLite database so tests are fully isolated.

#[cfg(test)]
mod tests;
