# M10 — AI Workspace API

## Goal

Give AI agents a first-class, structured, deterministic interface to inspect and interact with a Morphos workspace.

A completed milestone means an agent can understand scene structure, inspect selected details, request previews/diagnostics, and propose structured mutations without scraping the GUI or blindly rewriting files.

## Scope

Owns AI/tool-facing read APIs, context summaries, capability discovery, and bounded structured mutation entry points.

## Subgoals

### Read API

- [x] Expose `get_workspace_summary`.
- [x] Expose `get_scene_tree`.
- [x] Expose `get_node`.
- [x] Expose `get_parameters`.
- [x] Expose `get_diagnostics`.
- [x] Expose `get_recent_history`.
- [x] Expose geometry bounds/statistics for nodes/outputs.
- [x] Keep responses structured and size-bounded.

### Context shaping

- [x] Create a concise workspace summary designed for model context.
- [x] Add selective expansion so an agent can fetch details only for relevant nodes.
- [x] Include schema/capability information sufficient to construct valid operations.
- [x] Include current workspace revision in every mutable/read context response.
- [x] Provide source snippets only when requested/needed.

### Mutation boundary

- [x] Let agents submit proposed `WorkspaceOp` transactions.
- [x] Validate expected base revision to prevent stale edits.
- [x] Support dry-run validation and structured diff return.
- [x] Return affected nodes, diagnostics, and expected rebuild scope.
- [x] Reject unsupported/raw arbitrary filesystem mutation through this API.

### Preview/tool support

- [x] Expose a way to request a preview image or preview artifact reference when available.
- [x] Expose a way to evaluate a selected node/output.
- [x] Expose optional geometry statistics useful for reasoning about edits.
- [x] Keep all preview tooling usable independently of an LLM provider.

### Transport boundary

- [x] Define the API first as Rust interfaces/data structures.
- [x] Add one external transport suitable for local agents, such as JSON-RPC, stdio, or a small local service.
- [x] Version the external protocol.
- [x] Add capability discovery so clients can adapt to optional features.
- [x] Avoid coupling the workspace API to a specific AI vendor SDK.

### Tests

- [x] Add read-API snapshot tests.
- [x] Add stale-revision mutation tests.
- [x] Add dry-run diff tests.
- [x] Add protocol serialization/version tests.
- [x] Add an end-to-end fake-agent test using only the public AI workspace API.

## Completion criteria

- An AI can reason about and modify a workspace without accessing GUI internals.
- The interface is useful to non-AI automation as well.
- Agent edits are expressed through the same transaction machinery used elsewhere.

