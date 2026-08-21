use bevy::asset::RenderAssetUsages;
use bevy::mesh::Indices;
use bevy::prelude::Mesh;
use bevy::render::render_resource::PrimitiveTopology;
use geom_geometry::Mesh as MorphosMesh;
use std::error::Error;
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct MeshAdapterError {
    message: String,
}

impl MeshAdapterError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for MeshAdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(f)
    }
}

impl Error for MeshAdapterError {}

/// Converts Morphos mesh geometry into a renderable Bevy mesh with smooth normals.
pub fn adapt_morphos_mesh(mesh: &MorphosMesh) -> Result<Mesh, MeshAdapterError> {
    let positions = mesh.positions();
    let indices = mesh.triangle_indices();
    if positions.is_empty() || indices.is_empty() {
        return Err(MeshAdapterError::new("cannot render an empty Morphos mesh"));
    }

    let mut render_positions = Vec::with_capacity(positions.len());
    for position in positions {
        render_positions.push([position[0] as f32, position[1] as f32, position[2] as f32]);
    }

    let mut render_normals = vec![[0.0_f32; 3]; render_positions.len()];
    for triangle in indices {
        let a = render_positions[triangle[0] as usize];
        let b = render_positions[triangle[1] as usize];
        let c = render_positions[triangle[2] as usize];

        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let ac = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let cross = [
            ab[1] * ac[2] - ab[2] * ac[1],
            ab[2] * ac[0] - ab[0] * ac[2],
            ab[0] * ac[1] - ab[1] * ac[0],
        ];

        for vertex_index in triangle {
            let normal = &mut render_normals[*vertex_index as usize];
            normal[0] += cross[0];
            normal[1] += cross[1];
            normal[2] += cross[2];
        }
    }

    for normal in &mut render_normals {
        let length = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        if length > 1e-6 {
            normal[0] /= length;
            normal[1] /= length;
            normal[2] /= length;
        } else {
            *normal = [0.0, 1.0, 0.0];
        }
    }

    let mut flat_indices = Vec::with_capacity(indices.len() * 3);
    for triangle in indices {
        flat_indices.extend_from_slice(triangle);
    }

    let mut bevy_mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    bevy_mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, render_positions);
    bevy_mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, render_normals);
    bevy_mesh.insert_indices(Indices::U32(flat_indices));
    Ok(bevy_mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_rejects_empty_meshes() {
        let mesh = MorphosMesh::new(vec![], vec![]).expect("empty mesh is structurally valid");
        let error = adapt_morphos_mesh(&mesh).expect_err("empty mesh should fail");
        assert!(error.to_string().contains("empty"));
    }

    #[test]
    fn adapter_creates_renderable_normals() {
        let mesh = MorphosMesh::new(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .expect("mesh");

        let adapted = adapt_morphos_mesh(&mesh).expect("adapt mesh");
        assert!(adapted.attribute(Mesh::ATTRIBUTE_NORMAL).is_some());
    }
}
