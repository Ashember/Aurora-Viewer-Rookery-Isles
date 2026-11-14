//! Diagnostic for the Rookery body (docs/ROOKERY_BODY.md): imports a .glb
//! through the viewer's own glTF importer and reports what an upload would
//! get: models, faces and triangles, skin joints, warnings, and the joint
//! positions the skin overrides. Usage: body_check <body.glb> <avatar_skeleton.xml>
fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("body.glb");
    let skeleton_path = args.next().expect("avatar_skeleton.xml");
    let skeleton = aurora_assets::Skeleton::parse(&std::fs::read(&skeleton_path).expect("skeleton file")).expect("skeleton");
    let data = std::fs::read(&path).expect("glb file");
    let document = aurora_assets::import_gltf::Document::parse(&data, |uri| {
        Err(aurora_assets::AssetError::invalid(format!("external file {uri} not expected")))
    })
    .expect("glTF document");
    let scene = aurora_assets::import_mesh::gltf::parse(&document, &skeleton).expect("import");
    println!("models {}  instances {}  materials {}", scene.models.len(), scene.instances.len(), scene.materials.len());
    for model in &scene.models {
        let triangles: usize = model.faces.iter().map(|f| f.indices.len() / 3).sum();
        let vertices: usize = model.faces.iter().map(|f| f.vertices.len()).sum();
        println!("model {:?}: faces {} vertices {} triangles {}", model.name, model.faces.len(), vertices, triangles);
        if let Some(skin) = &model.skin {
            println!("  skin joints {}", skin.names.len());
            for name in ["mPelvis", "mHead", "mShoulderLeft", "mElbowLeft", "mWristLeft", "mKneeLeft", "mAnkleLeft", "mEyeLeft"] {
                if let Some(i) = skin.names.iter().position(|n| n == name) {
                    let position = skin.alternate_bind[i].w_axis.truncate();
                    println!("  {name:16} override {:+.3} {:+.3} {:+.3}", position.x, position.y, position.z);
                }
            }
        } else {
            println!("  NO SKIN");
        }
    }
    for warning in &scene.warnings {
        println!("warning: {warning}");
    }
}
