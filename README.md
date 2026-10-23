# Personal AI Architecture Rust Template

A small, runnable Rust starter for building a Personal AI Architecture (PAA)
implementation. It demonstrates the public Gateway, Agent Loop, model-adapter,
memory, streaming, configuration, and contract-validation boundaries in code
you can read and replace.

The current contract checks cover per-request model and provider selection,
provider-profile routing, saved reply provenance, context limits, and notices
for omitted complete turns. This starter's small text journey is not a full
implementation of every PAA tool and product conformance case.

This is an educational and development template. It uses a deterministic mock
adapter by default, so the demo and tests run offline. The included
OpenAI-compatible adapter supports HTTPS and streamed responses; add your
authentication, persistence, policy enforcement, observability, and deployment
controls before using it with real users or data.

## Quick start

Requirements: Rust 1.80 or newer, Cargo, and Node.js only if you want to run the
shared language-neutral validator.

```bash
cargo run --bin paa-demo
cargo test
cargo run --bin paa-server
```

In another terminal:

```bash
curl http://127.0.0.1:8000/health
curl -N -X POST http://127.0.0.1:8000/conversations/messages \
  -H 'Content-Type: application/json' \
  -H 'X-Actor-ID: demo-user' \
  -H 'X-Actor-Permissions: memory:read,memory:write' \
  -d '{"content":[{"type":"text","text":"Explain what PAA is."}]}'
```

The demo server intentionally uses the Rust standard library for local HTTP
hosting. Replace it with your chosen production HTTP stack when you need TLS,
graceful shutdown, limits, middleware, and a mature request parser.

## Validate the template

Run the complete native suite from this directory:

```bash
cargo run --bin paa-validate \
  -- --source-checkout ../../personal-ai-architecture \
  --shared-validator ../paa-conformance-validator/bin/paa-validate.js
```

The native checks cover formatting, Clippy warnings, contract hashes and source
comparison, architecture boundaries, unit/integration/conformance tests, and
acceptance behavior. The shared validator additionally executes the commands in
`paa.target.json` and detects contract drift.

The included GitHub Actions workflow checks out the exact pinned PAA source and
runs the same source-comparison gate, so contract drift is visible in CI.

The template is pinned to PAA commit
`28e9a275a0f5e062aa4452892e9ec70644f88c69`. `paa.lock.json` records that pin,
the contract file set, and SHA-256 digests. Contracts are mirrored under
`contracts/` for the starter's local examples and vendored under
`vendor/paa-contracts/` for reproducible validation.

The executable checks cover this starter's text-message journey and declared component boundaries. Pinning the full PAA contract set does not by itself certify unimplemented product-specific paths such as production authentication, approval policy, or multimodal provider handling.

## PAA flow

```text
HTTP Gateway
    -> public request validation and actor headers
Agent Loop
    -> provider-neutral completion request
Model Adapter
    -> mock by default; OpenAI-compatible HTTPS adapter also included
SSE stream
    -> text-delta and done events
Memory Store
    -> conversation and message history boundary
```

Canonical routes demonstrated by the starter:

| Method | Route | Purpose |
| --- | --- | --- |
| `GET` | `/health` | Local liveness check |
| `POST` | `/conversations/messages` | Start a conversation and stream a reply |
| `POST` | `/conversations/{id}/messages` | Continue a conversation |
| `GET` | `/conversations` | List conversations |
| `GET` | `/conversations/{id}` | Read conversation history |

The public message shape uses content parts: `{ "content": [{"type": "text", "text": "..."}], "metadata": {} }`.
Actor identity and permissions arrive through `X-Actor-ID` and
`X-Actor-Permissions`; provider selection is runtime configuration, not a public
engine request field.

## Project layout

```text
src/types/       PAA-facing request, message, tool, and stream contracts
src/gateway/     route handling, public validation, streaming, persistence hook
src/engine/      provider-neutral Agent Loop
src/adapters/    mock and OpenAI-compatible adapter boundaries
src/memory/      replaceable memory persistence boundary
src/config/      runtime configuration loading
src/checks.rs    contract, boundary, and lock-in checks
src/bin/         demo, server, native checks, acceptance, and full validation
tests/           integration and PAA conformance tests
contracts/           local contract mirror
vendor/          pinned contract source used by validation
```

## Extending it

1. Keep `src/types/` aligned with the pinned PAA contracts.
2. Add provider protocols only under `src/adapters/` and normalize them into
   `ModelStreamChunk` values.
3. Keep authorization, policy, tool permissions, and persistence explicit at
   the Gateway/Agent Loop boundaries.
4. Replace the in-memory store and mock HTTP server with production components.
5. Add tests at the Gateway, Agent Loop, adapter, and persistence boundaries.
6. Run `cargo run --bin paa-validate` and the shared validator before release.

See `AGENTS.md` for the template-specific implementation rules.
