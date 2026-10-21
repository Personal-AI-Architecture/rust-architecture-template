# Rust PAA Template Instructions

This template is a public starter implementation of the Personal AI
Architecture (PAA). Keep the vendored contracts, `specs/` mirror, and
`paa.lock.json` synchronized. Run `cargo run --bin paa-validate` before
submitting changes.

Provider-specific protocol details belong under `src/adapters/`; the gateway,
agent loop, memory, and public contracts must remain provider-neutral.
