# Handwritten domain models

Group API-facing interfaces by domain and preserve exports through ../types.ts. Server/client unions remain separate from payload model definitions.

Keep handwritten compatibility types aligned with generated contracts. Type moves must preserve field names, optionality, literal tags, and helper behavior.

Run frontend build and contract fixture checks.
