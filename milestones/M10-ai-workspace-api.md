# M10 — AI Workspace API

## Goal

Give AI agents a first-class, structured, deterministic interface to inspect and interact with a Morphos workspace.

A completed milestone means an agent can understand scene structure, inspect selected details, request previews/diagnostics, and propose structured mutations without scraping the GUI or blindly rewriting files.

## Scope

Owns AI/tool-facing read APIs, context summaries, capability discovery, and bounded structured mutation entry points.

## Subgoals

### Read API

- [ ] Expose `get_workspace_summary`.
- [ ] Expose `get_scene_tree`.
- [ ] Expose `get_node`.
- [ ] Expose `get_parameters`.
- [ ] Expose `get_diagnostics`.
- [ ] Expose `get_recent_history`.
- [ ] Expose geometry bounds/statistics for nodes/outputs.
- [ ] Keep responses structured and size-bounded.

### Context shaping

- [ ] Create a concise workspace summary designed for model context.
- [ ] Add selective expansion so an agent can fetch details only for relevant nodes.
- [ ] Include schema/capability information sufficient to construct valid operations.
- [ ] Include current workspace revision in every mutable/read context response.
- [ ] Provide source snippets only when requested/needed.

### Mutation boundary

- [ ] Let agents submit proposed `WorkspaceOp` transactions.
- [ ] Validate expected base revision to prevent stale edits.
- [ ] Support dry-run validation and structured diff return.
- [ ] Return affected nodes, diagnostics, and expected rebuild scope.
- [ ] Reject unsupported/raw arbitrary filesystem mutation through this API.

### Preview/tool support

- [ ] Expose a way to request a preview image or preview artifact reference when available.
- [ ] Expose a way to evaluate a selected node/output.
- [ ] Expose optional geometry statistics useful for reasoning about edits.
- [ ] Keep all preview tooling usable independently of an LLM provider.

### Transport boundary

- [ ] Define the API first as Rust interfaces/data structures.
- [ ] Add one external transport suitable for local agents, such as JSON-RPC, stdio, or a small local service.
- [ ] Version the external protocol.
- [ ] Add capability discovery so clients can adapt to optional features.
- [ ] Avoid coupling the workspace API to a specific AI vendor SDK.

### Tests

- [ ] Add read-API snapshot tests.
- [ ] Add stale-revision mutation tests.
- [ ] Add dry-run diff tests.
- [ ] Add protocol serialization/version tests.
- [ ] Add an end-to-end fake-agent test using only the public AI workspace API.

## Completion criteria

- An AI can reason about and modify a workspace without accessing GUI internals.
- The interface is useful to non-AI automation as well.
- Agent edits are expressed through the same transaction machinery used elsewhere.

