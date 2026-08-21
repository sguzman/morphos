# M04 — Live Viewport Shell

## Goal

Build the smallest useful desktop application for inhabiting and inspecting a Morphos workspace.

A completed milestone means the app can open a workspace, display its evaluated geometry, navigate around it comfortably, and expose basic workspace/build state.

## Scope

Owns desktop shell, renderer integration, camera controls, selection groundwork, and minimal status UI.

## Subgoals

### Application shell

- [x] Create `geom_app` using Bevy.
- [x] Integrate egui/bevy_egui for tool UI.
- [x] Open a workspace from a command-line path.
- [x] Add recent/reopen behavior only if it does not complicate the core shell.
- [x] Keep application startup functional even when the scene currently contains errors.

### 3D viewport

- [x] Display evaluated workspace mesh data in the viewport.
- [x] Implement orbit camera.
- [x] Implement pan and zoom.
- [x] Add grid/axis orientation aids.
- [x] Add shaded and wireframe/debug display modes.
- [x] Frame all geometry and frame selected geometry commands.
- [x] Display empty-scene and build-error states gracefully.

### View synchronization

- [x] Replace rendered geometry when a new evaluation result arrives.
- [x] Avoid recreating unrelated application state during geometry refreshes.
- [x] Preserve camera state across scene rebuilds.
- [x] Show current workspace revision and geometry revision.

### Minimal status surface

- [x] Add top-level workspace dirty/clean status.
- [x] Add last rebuild duration.
- [x] Add success/error build indicator.
- [x] Add current output/selection name where available.

### Tests

- [x] Add non-rendering tests for app/workspace state synchronization.
- [x] Add a smoke-test/example workspace that launches in the app.
- [x] Document manual verification steps for camera and viewport behavior.

## Completion criteria

- Opening a valid workspace presents navigable 3D geometry.
- Geometry can refresh without resetting the user's camera.
- Workspace/build state is visible without opening a console.

## Verification Notes

- `geom_app` is implemented as a dedicated Bevy 0.19.1 / bevy_egui 0.41.1 crate with a library
  target, binary target, mesh adapter, orbit camera model, manual rebuild/reopen actions, grid
  and axis gizmos, and shaded or wireframe display.
- Non-rendering verification is covered by `cargo test -p geom_app`, including successful build
  state, failed rebuild preserving last-good displayed geometry, camera preservation, frame math,
  and display-mode isolation.
- Manual viewport verification steps are recorded in `docs/viewport.md`.
- On Friday, August 21, 2026, the coding-agent environment compiled and tested the shell but did
  not claim visual desktop inspection of the running window. Interactive viewport checks remain a
  manual follow-up in a local desktop session.

