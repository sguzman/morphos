//! Canonical workspace crate for Morphos project state.
//!
//! M00 intentionally keeps this crate minimal. M01 will introduce the durable
//! workspace model and lifecycle APIs that the rest of the project depends on.

/// Marker type establishing the dedicated workspace crate anticipated by M01.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkspaceCrate;

#[cfg(test)]
mod tests {
    use super::WorkspaceCrate;

    #[test]
    fn marker_type_is_constructible() {
        let marker = WorkspaceCrate;
        assert_eq!(marker, WorkspaceCrate);
    }
}
