# Source architecture inspection

Own parsing support for repository boundary and maintainability checks. Treat source as text to inspect, never as executable input.

Follow declared production modules and exclude cfg(test) subtrees without dropping later production code. Mask literals/comments without losing offsets; handle lifetimes and character literals distinctly.

Run scripts/test-actor-service-boundaries.py and scripts/test-maintainability.py after changing scanner or policy behavior.
