# Client transport contracts

client.ts and websocket.ts adapt HTTP/socket traffic. types.ts is the compatibility entry point for handwritten domain models; generated/ comes from the server schema.

Do not edit generated contracts or validators manually. Preserve strict decoding, safe errors, credentials, correlation IDs, and module import compatibility. Generated files inherit this guidance and are excluded from the handwritten size budget.

Run contract regeneration/fixtures, frontend build, and transport browser tests.
