//! The Rookery body (docs/ROOKERY_BODY.md): MakeHuman's body as the avatar's
//! own body on the Rookery Isles grid, shaped by the Character Creator.
//!
//! - `pack`: the body pack the grid serves (built by the HomeWorldz tools)
//! - `shape`: MakeHuman's slider arithmetic
//! - `look`: a character's look, as saved on the grid
//! - `build`: a look turned into drawable, skinned parts and joints
//! - `fetch`: the pack downloaded from the grid and kept in the cache

pub mod build;
pub mod fetch;
pub mod look;
pub mod pack;
pub mod shape;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// The pack built by tools/makehuman/rookery_body/pack.py, when present
    /// on this machine (ROOKERY_BODY_PACK, else the HomeWorldz default).
    fn pack_folder() -> Option<PathBuf> {
        let folder = std::env::var_os("ROOKERY_BODY_PACK")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:/HomeWorldz/var/rookery-body/pack"));
        folder.join("body.json").exists().then_some(folder)
    }

    /// The Rust build and the Python converter make the same body: the
    /// default woman is 1.59 m and her joints sit where the converter put
    /// them (tools/makehuman/rookery_body/build_body.py report).
    #[test]
    fn the_viewer_builds_the_body_the_converter_built() {
        let Some(folder) = pack_folder() else {
            eprintln!("skipping: no Rookery body pack on this machine");
            return;
        };
        let pack = pack::Pack::load(&folder).expect("pack");
        let look = look::Look::default();
        let weights = shape::target_weights(&pack, &look.macros, &HashMap::new());
        assert_eq!(weights.len(), 4, "MakeHuman's default woman is four targets: {weights:?}");
        let body = build::build(&pack, &look, &HashMap::new());
        assert!((body.height - 1.59).abs() < 0.01, "height {}", body.height);
        let joint = |name: &str| body.joints[pack.joints.iter().position(|j| j == name).expect(name)];
        // the converter's report for this body (rookery-body-default.json)
        for (name, expected) in [
            ("mPelvis", [0.0, 0.0, 0.8249]),
            ("mShoulderLeft", [-0.003, 0.1448, 1.2274]),
            ("mElbowLeft", [-0.003, 0.3772, 1.2274]),
            ("mWristLeft", [-0.003, 0.5878, 1.2274]),
        ] {
            let got = joint(name);
            assert!(got.distance(glam::Vec3::from_array(expected)) < 0.004, "{name}: {got:?} != {expected:?}");
        }
        let skin_vertices: usize = body.parts.iter().map(|p| p.vertices.len()).sum();
        // the body wears the look's skin
        let skin = pack.content.swatch(&look.skin).map(|s| s.texture.clone());
        assert!(skin.is_some(), "the default skin is in the pack");
        assert_eq!(body.parts[0].item, "body");
        assert_eq!(body.parts[0].material.texture, skin);
        assert!(skin_vertices > 12000, "the body part is there: {skin_vertices}");
        // a heavier, older man is a different, taller body on the same rig
        let mut man = look::Look::default();
        man.macros.gender = 1.0;
        man.macros.weight = 0.9;
        let big = build::build(&pack, &man, &HashMap::new());
        assert!(big.height > body.height + 0.05, "{} vs {}", big.height, body.height);
        assert_eq!(big.joints.len(), body.joints.len());
    }

    /// Worn items read from their own files and placed on the body by their
    /// ties: the eyes land in the eye sockets.
    #[test]
    fn eyes_brows_and_lashes_sit_on_the_face() {
        let Some(folder) = pack_folder() else {
            eprintln!("skipping: no Rookery body pack on this machine");
            return;
        };
        let pack = pack::Pack::load(&folder).expect("pack");
        let look = look::Look::default();
        let mut meshes = HashMap::new();
        for id in [Some(&look.eyes), look.eyebrows.as_ref(), look.eyelashes.as_ref()].into_iter().flatten() {
            let proxy = pack.content.proxy(id).unwrap_or_else(|| panic!("{id} in the pack"));
            meshes.insert(id.clone(), std::sync::Arc::new(pack.proxy_mesh(proxy).expect("mesh")));
        }
        let body = build::build(&pack, &look, &meshes);
        let eye_left = body.joints[pack.joints.iter().position(|j| j == "mEyeLeft").expect("eye")];
        let eyes = body.parts.iter().find(|p| p.name == "eyes-highpoly").expect("eyes part");
        let centre = (eyes.min + eyes.max) / 2.0;
        assert!((centre.z - eye_left.z).abs() < 0.02, "eyes at {centre:?}, eye joint {eye_left:?}");
        assert!(body.parts.iter().any(|p| p.name.starts_with("eyebrows-")));
        assert!(body.parts.iter().any(|p| p.name.starts_with("eyelashes-")));
    }
}
