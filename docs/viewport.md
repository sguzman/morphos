# Morphos Viewport Verification (M04)

Launch command:

```powershell
cargo run -p geom_app -- examples/workspaces/viewport-smoke
```

Expected control mapping:

- Right mouse drag: orbit
- Middle mouse drag: pan
- Mouse wheel: zoom

Manual verification checklist:

1. Launch the app with the smoke workspace and confirm a window opens.
2. Confirm the asymmetric smoke shape is visible on startup.
3. Confirm orbit works with right-drag.
4. Confirm pan works with middle-drag.
5. Confirm zoom works with the mouse wheel.
6. Confirm interacting with egui controls does not move the camera.
7. Confirm the grid lies on the XZ plane and +Y is up.
8. Confirm the world-axis lines make +X, +Y, and +Z visually distinct.
9. Confirm shaded mode is the default and displays lit solid geometry.
10. Confirm wireframe mode toggles on and off from the top status UI.
11. Confirm `Frame All` reframes the full displayed geometry.
12. Confirm `Frame Selected` reframes the selected/current output geometry.
13. Confirm external saves to `source/scene.toml` refresh geometry automatically without pressing `Reload / Rebuild`.
14. Confirm `Reload / Rebuild` still works and routes through the same common rebuild path.
15. Confirm the camera does not reset across successful reactive geometry rebuilds.
16. Confirm parse or geometry errors appear in the UI without closing the app and that the last good mesh remains visible.

Environment note:

- This document records the required manual procedure.
- In the current coding-agent environment, compilation and non-rendering tests can be executed,
  but visual inspection of the desktop window is not treated as completed unless explicitly
  observed in an interactive desktop session.
