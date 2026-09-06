# Load and recovery qualification

Generator workers/probes and evidence analysis have separate packages. Shell setup owns smoke/full prerequisites; the top-level runner owns cleanup and restore orchestration.

Full acceptance thresholds must remain strict. generator_fingerprint covers the entry point and local generator sources, including this provenance implementation; preserve that transitive coverage.

Run analyzer self-tests, provenance tests, and scripts/run-load-recovery-qualification.sh in smoke mode. A local smoke never proves dedicated-host one-hour qualification.
