# Session and projection facade

mod.rs owns shared ChatEngine state and public re-exports. Child modules group session orchestration and domain command/projection responsibilities. Use the corresponding service for authoritative state changes.

Do not turn the facade back into a second persistence layer. Keep delivery guards, account identity, bounded queues, and disconnect cleanup attached to the operation they protect. legacy_send.rs is a cfg(test)-only compatibility oracle.

Run engine::chat_engine::tests, the architecture guard, and the browser socket suite for transport-facing changes.
