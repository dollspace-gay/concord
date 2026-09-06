# Incoming event reducers

Each module handles a typed event family; ../events.ts selects the family. Keep unrelated domain cases in their owner.

Preserve version ordering, tombstone redaction, pending request ownership, and protected-generation checks. Early returns and switch breaks must retain their original meaning.

Run all browser replay/store tests and frontend lint/build. Do not weaken generated event types with any or broad assertions.
