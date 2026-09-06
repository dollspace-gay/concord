# Remediation design verification

Verified 2026-09-05. This verifies the requested **design document**, not implementation or deployment.

Document: [Concord full remediation](../../concord-remediation.md).

Document SHA-256: `0cf133b63ae185351ed8000c82b8e3f969885399f8e2a47144f845833136b040`.

## Completion audit

| Obligation | Evidence and result |
| --- | --- |
| Ground the full scope in the current project | Current commit and source/call-path reads match the baseline manifest; all 132 recorded source/config-example/manifest files retain their captured hashes. |
| Address the review rather than narrow it | E01–E12 map to explicit requirements and gates; additional thread/privacy, notification, configuration, and migration findings are included. |
| Preserve north-star ambition and existing features | All 26 F journeys cover the README/review feature families; live voice/federation/encryption extensions are explicitly separate. |
| Make requirements observable | 24 unique R requirements map one-to-one to 24 unique G acceptance gates; every F journey names its gates. |
| Specify the design and tradeoffs | Ownership/dependency diagram, 13 proposed-design subsections, decision/alternative table, access and failure matrices, wire/schema and privacy/durability contracts were reviewed. |
| Specify compatibility and data recovery | Eight migration groups, historical 1–16 fixture/repair policy, stable ID/content handling, PDS import provenance, staged activation and rollback/restore are explicit. |
| Make delivery concrete | Ten stages S0–S9 specify files/ownership, dependencies, data transition and evidence. Partial stage evidence is distinguished from a completed broad gate. |
| Include verification and operational targets | Current commands are distinguished from proposed suites/commands; workload/hardware/budgets are targets, not present performance claims. |
| Resolve architecture choices without hiding assumptions | Recommended defaults are explicit; no unresolved decision prevents planning. Generator/library selection is assigned a bounded implementation proof, not claimed to exist. |
| Preserve reproducible review evidence | Probe sources/results and source hashes are stored beside the document; experiments intentionally characterize defects and are not claimed as repaired regression results. |
| Make the artifact usable | All required design-skill sections are present; local Markdown links exist; fenced blocks balance; no unfinished marker tokens. Crosslink's parser reads 24 requirements and 24 acceptance criteria and retains full design/migration/rollout sections. |
| Preserve workspace boundaries | Only .design artifacts were added by this design task; existing application sources, lockfiles, configuration, data, and unrelated worktree changes are preserved. |

## Checks performed

- Parsed all required Markdown section headings and unique R/G/F/S/M identifiers; checked requirement/gate correspondence and feature/gate coverage.
- Checked local Markdown link destinations and fenced-block balance; inspected the stage and baseline sections after revision.
- Compared current SHA-256 values with the source and copied-probe manifest.
- Compiled an isolated check using the installed tooling checkout's actual `design_doc.rs` parser; it preserved the expected title, requirements, gates, proposed design, data compatibility, and rollout sections.
- Reviewed authority, publication, transaction/replay, legacy repair, rollback and stage dependencies for contradictions. Corrected stage evidence scope and clarified opaque global cursors, migration provenance, and backup media retention.
- Ran Git whitespace checking and inspected the exact added-file inventory. Saved result text as .txt so repository-wide .log ignores do not omit the evidence from a later intentional commit.

## Limits

No remediation gate G01–G24 is claimed passed by this document review. Application checks cited in the baseline belong to the reviewed source; they were not rerun as if documentation changed application behavior. Full release verification, external provider canary, visual/accessibility review, and load/storage fault tests remain implementation work explicitly specified by the design.
