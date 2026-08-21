# M04 — Live Viewport Shell

## Goal

Build the smallest useful desktop application for inhabiting and inspecting a Morphos workspace.

A completed milestone means the app can open a workspace, display its evaluated geometry, navigate around it comfortably, and expose basic workspace/build state.

## Scope

Owns desktop shell, renderer integration, camera controls, selection groundwork, and minimal status UI.

## Subgoals

### Application shell

- [ ] Create `geom_app` using Bevy.
- [ ] Integrate egui/bevy_egui for tool UI.
- [ ] Open a workspace from a command-line path.
- [ ] Add recent/reopen behavior only if it does not complicate the core shell.
- [ ] Keep application startup functional even when the scene currently contains errors.

### 3D viewport

- [ ] Display evaluated workspace mesh data in the viewport.
- [ ] Implement orbit camera.
- [ ] Implement pan and zoom.
- [ ] Add grid/axis orientation aids.
- [ ] Add shaded and wireframe/debug display modes.
- [ ] Frame all geometry and frame selected geometry commands.
- [ ] Display empty-scene and build-error states gracefully.

### View synchronization

- [ ] Replace rendered geometry when a new evaluation result arrives.
- [ ] Avoid recreating unrelated application state during geometry refreshes.
- [ ] Preserve camera state across scene rebuilds.
- [ ] Show current workspace revision and geometry revision.

### Minimal status surface

- [ ] Add top-level workspace dirty/clean status.
- [ ] Add last rebuild duration.
- [ ] Add success/error build indicator.
- [ ] Add current output/selection name where available.

### Tests

- [ ] Add non-rendering tests for app/workspace state synchronization.
- [ ] Add a smoke-test/example workspace that launches in the app.
- [ ] Document manual verification steps for camera and viewport behavior.

## Completion criteria

- Opening a valid workspace presents navigable 3D geometry.
- Geometry can refresh without resetting the user's camera.
- Workspace/build state is visible without opening a console.

