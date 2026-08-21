# Morphos Scene Schema (M02)

Morphos scene documents live in `source/scene.toml` and use an explicit schema version distinct
from the workspace format version:

```toml
schema_version = 1
root = "body"
```

## IDs and References

- Node IDs come from `[nodes.<id>]` table names.
- Parameter IDs come from `[params.<id>]` table names.
- IDs must start with an ASCII letter and may then use ASCII letters, digits, `_`, or `-`.
- IDs are stable independent of TOML ordering and are the canonical reference handles for future
  geometry, GUI, CLI, and AI consumers.
- Node references use string IDs in arrays such as `children = ["left_arm", "right_arm"]`.
- Parameter references use typed scalar references such as `{ param = "body_width" }`.

## Coordinate and Transform Conventions

- Coordinate system: right-handed.
- Up axis: `+Y`.
- Translation units: abstract Morphos scene units.
- Rotation representation: Euler XYZ stored in `rotate_deg`.
- Rotation units: degrees.
- Local transform application order: scale, then rotate, then translate.
- Scale components must be strictly positive.

## Parameter Model

M02 intentionally keeps the parameter system small. The initial parameter type set contains one
typed kind:

- `scalar`

Parameters are declared as:

```toml
[params.body_width]
type = "scalar"
value = 1.4
```

Scalar-consuming fields can reference them directly:

```toml
size = { x = { param = "body_width" }, y = 1.8, z = 0.7 }
```

This is a typed reference model, not text substitution and not a general expression language.

## Node Grammar by Example

Primitives:

```toml
[nodes.cube]
kind = "box"
size = { x = 1.0, y = 1.0, z = 1.0 }
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }

[nodes.ball]
kind = "sphere"
radius = 0.5
transform = { translate = { x = 0.0, y = 1.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
```

Supported primitive declarations in M02:

- `box`
- `sphere`
- `cylinder`
- `capsule`
- `plane`
- `profile`

Composition:

```toml
[nodes.body]
kind = "union"
children = ["torso", "head"]
transform = { translate = { x = 0.0, y = 0.0, z = 0.0 }, rotate_deg = { x = 0.0, y = 0.0, z = 0.0 }, scale = { x = 1.0, y = 1.0, z = 1.0 } }
```

Supported composition operators in M02:

- `union`
- `difference`
- `intersection`

The `root` field must name a valid node ID so future M03 evaluation can resolve one explicit
output node without inspecting TOML structure directly.

## Source-Preserving Editing

`geom_scene::SceneSource` owns the source-preserving editable representation. It parses TOML with
`toml_edit::DocumentMut`, performs targeted mutations on existing syntax nodes, and revalidates
the result into a `SceneDocument`.

This means:

- unrelated comments should survive targeted edits
- unrelated key ordering should survive targeted edits
- unrelated layout should survive targeted edits where `toml_edit` can preserve it
- Morphos does not create a second hidden canonical scene state

`SceneDocument` is the typed semantic model. It does not expose TOML editing types as its public
scene API.

## Temporary Invalid Source Contract

Parsing invalid TOML returns a structured `SceneError` with source span information.

M02 does not implement file watching or last-good viewport behavior. Instead it provides the
contract M05 will need:

- invalid source can fail cleanly
- callers can retain the previous valid `SceneDocument`
- callers can keep the invalid raw text separately
- a failed parse/validation never partially mutates a previously valid typed scene

## Extensions and Unknown Fields

- Unknown structural fields are rejected so likely typos do not silently become schema.
- A designated `extensions` area is preserved at the top level and within parameter/node tables.
- Extension metadata is stored as Morphos-owned typed values and survives targeted edits and
  canonical semantic round trips.

## Workspace Relationship

`geom_workspace` remains the owner of durable workspace/source text only.

Typical composition:

```rust
let workspace = geom_workspace::Workspace::open(path)?;
let mut scene_source = geom_scene::SceneSource::parse(workspace.source_text())?;
let scene = scene_source.validate()?;

scene_source.set_parameter_scalar(&geom_scene::ParamId::new("body_width")?, 1.8)?;
let updated_source = scene_source.into_text();

let mut workspace = workspace;
workspace.replace_source(updated_source);
workspace.save()?;
```

That separation is deliberate: workspaces remain openable even when scene TOML is temporarily
invalid.
