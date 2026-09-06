# Evidence validation

Separate structural validation, workload/results, resources, supporting artifacts, orchestration, and negative fixtures. The CLI is a thin adapter.

Fail closed on missing/invalid evidence. Keep atomic output, exact boolean/type checks, provenance, resource thresholds, and the distinction between smoke and full claims.

Run python3 scripts/analyze-load-recovery-evidence.py --self-test and provenance tests; add a negative fixture for a new acceptance condition.
