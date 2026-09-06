# Server wire events

protocol.rs holds the flat ChatEvent enum; adjacent modules own payload models. ../events.rs preserves the public import path.

Serde tags, defaults, field types, and nullability are wire behavior. The protocol enum has a reviewed size exception; move model definitions out instead of changing its shape just to shorten it.

Run scripts/check-contract.sh and scripts/check-contract-fixtures.sh, plus event serialization tests.
