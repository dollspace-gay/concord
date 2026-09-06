# Active workload generator

state.py owns one process configuration, shared collections, locks, and worker counters. Transports, steady workers, stress probes, orchestration, and reporting are separate.

Collections/events are shared by identity. Access reassigned query counters via state, never from-imported copies. Keep startup/stop order, exact fanout checks, bounded retries, and configured full-scale thresholds.

Run the inert import/provenance tests and bounded load/recovery smoke. Changes to any generator module must change generator_fingerprint.
