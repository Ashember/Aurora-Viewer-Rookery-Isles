//! A body from a look: the shape at rest (MakeHuman's A-pose plus the
//! weighted targets), its MakeHuman skeleton from its joint groups, then the
//! T-pose of the standard skeleton by linear blend skinning with MakeHuman's
//! own weights (each limb bone turned to the standard segment's direction,
//! its children carried along), exactly as the HomeWorldz converter poses a
//! shape (tools/makehuman/rookery_body/body.py `Body._t_pose`); the standard
//! joints from the posed MakeHuman joints; the body part with what clothes
//! cover removed; every worn item fitted on the rest shape by its ties and
//! posed with its ties' weights. Pure data, built on a worker; the scene
//! uploads it (scene/rookery_body.rs).

use super::look::Look;
use super::pack::{Pack, ProxyMesh};
use super::shape::target_weights;
use aurora_render::{SkinVertex, Vertex};
use glam::{DMat3, DVec3, Vec3};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// How a built part is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct PartMaterial {
    /// Pack file of the texture (None: the colour alone).
    pub texture: Option<String>,
    pub color: [f32; 3],
    pub blend: bool,
    /// Cut out where the texture's alpha is under `cutoff` (hair cards,
    /// the eyes' clear cornea, lace), drawn with depth like a solid part,
    /// so overlapping layers never sort wrong. 2026-11-14.
    pub mask: bool,
    pub cutoff: f32,
    pub two_sided: bool,
}

/// The cutoff of a masked part whose pack gives none.
pub const DEFAULT_CUTOFF: f32 = 0.5;

/// One drawable face (at most 65,535 vertices).
pub struct BuiltPart {
    /// Its name ("body", "body 1" for a second face, or the item's id) and
    /// the material it was built with: read by the tests and diagnostics,
    /// the scene draws `material_of` of the look now.
    #[cfg_attr(not(test), allow(dead_code))]
    pub name: String,
    /// What it shows: "body", or the content id of the worn item. Its
    /// material comes from the look at draw time (`material_of`), so a
    /// colour or skin change redraws without building again.
    pub item: String,
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    pub skin: Vec<SkinVertex>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub material: PartMaterial,
    pub min: Vec3,
    pub max: Vec3,
}

/// A whole body ready to upload: its parts and its joints' rest positions
/// (skeleton space, feet at z = 0), in the pack's joint order.
pub struct BuiltBody {
    pub parts: Vec<BuiltPart>,
    pub joints: Vec<Vec3>,
    /// Height of the body, feet to crown, metres.
    pub height: f32,
}

/// The shape at rest (A-pose, viewer space, not centred) for every vertex.
pub fn positions(pack: &Pack, look: &Look) -> Vec<Vec3> {
    let sliders: HashMap<String, f32> = look.sliders.iter().map(|(k, v)| (k.clone(), *v)).collect();
    let mut out = pack.base.clone();
    let step = pack.delta_step;
    for (target, weight) in target_weights(pack, &look.macros, &sliders) {
        let w = weight * step;
        for (vertex, d) in &pack.targets[target].entries {
            out[*vertex as usize] += Vec3::new(d[0] as f32, d[1] as f32, d[2] as f32) * w;
        }
    }
    out
}

/// The rotation turning direction `a` onto `b` by the shortest arc
/// (body.py `_align`).
fn align(a: DVec3, b: DVec3) -> DMat3 {
    let a = a.normalize_or_zero();
    let b = b.normalize_or_zero();
    let v = a.cross(b);
    let c = a.dot(b);
    if c > 1.0 - 1e-9 {
        return DMat3::IDENTITY;
    }
    if c < -1.0 + 1e-9 {
        let mut axis = a.cross(DVec3::X);
        if axis.length() < 1e-6 {
            axis = a.cross(DVec3::Y);
        }
        let axis = axis.normalize();
        let outer = DMat3::from_cols(axis * axis.x, axis * axis.y, axis * axis.z);
        return outer * 2.0 - DMat3::IDENTITY;
    }
    // k = [v]x, column by column
    let k = DMat3::from_cols(DVec3::new(0.0, v.z, -v.y), DVec3::new(-v.z, 0.0, v.x), DVec3::new(v.y, -v.x, 0.0));
    DMat3::IDENTITY + k + k * k * (1.0 / (1.0 + c))
}

/// A shape's T-pose: each MakeHuman bone's rotation, rest head and posed
/// head, and the shift that stands the rest shape on the ground centred on
/// its pelvis.
pub struct Pose {
    shift: DVec3,
    rotation: Vec<DMat3>,
    head_rest: Vec<DVec3>,
    head_posed: Vec<DVec3>,
}

impl Pose {
    /// The pose of a shape at rest (`positions`).
    pub fn of(pack: &Pack, rest: &[Vec3]) -> Pose {
        let joint = |group: usize| -> DVec3 {
            let vertices = &pack.mh_groups[group];
            if vertices.is_empty() {
                return DVec3::ZERO;
            }
            vertices.iter().map(|v| rest[*v as usize].as_dvec3()).sum::<DVec3>() / vertices.len() as f64
        };
        let pelvis = joint(pack.pelvis_group);
        let floor = pack.body.vertices.iter().map(|v| f64::from(rest[*v as usize].z)).fold(f64::MAX, f64::min);
        let shift = DVec3::new(-pelvis.x, -pelvis.y, -floor);
        let n = pack.mh_bones.len();
        let mut pose = Pose {
            shift,
            rotation: vec![DMat3::IDENTITY; n],
            head_rest: vec![DVec3::ZERO; n],
            head_posed: vec![DVec3::ZERO; n],
        };
        for (i, bone) in pack.mh_bones.iter().enumerate() {
            let head = joint(bone.head) + shift;
            let tail = joint(bone.tail) + shift;
            let parent_rotation = bone.parent.map_or(DMat3::IDENTITY, |p| pose.rotation[p]);
            let head_posed = match bone.parent {
                Some(p) => pose.head_posed[p] + parent_rotation * (head - pose.head_rest[p]),
                None => head,
            };
            let own = match bone.target {
                Some(target) => align(parent_rotation * (tail - head), target.as_dvec3()),
                None => DMat3::IDENTITY,
            };
            pose.rotation[i] = own * parent_rotation;
            pose.head_rest[i] = head;
            pose.head_posed[i] = head_posed;
        }
        pose
    }

    /// A point of the rest shape (not centred) in the T-pose, by the blend
    /// of its MakeHuman bone weights (body.py `pose_points`).
    pub fn point(&self, rest: Vec3, weights: impl Iterator<Item = (u16, f32)>) -> Vec3 {
        let p = rest.as_dvec3() + self.shift;
        let mut value = DVec3::ZERO;
        let mut total = 0.0;
        for (bone, weight) in weights {
            let b = bone as usize;
            if b >= self.rotation.len() || weight <= 0.0 {
                continue;
            }
            let w = f64::from(weight);
            value += (self.rotation[b] * (p - self.head_rest[b]) + self.head_posed[b]) * w;
            total += w;
        }
        (if total > 1e-9 { value / total } else { p }).as_vec3()
    }

    /// The standard skeleton's joints of the posed shape, in the pack's
    /// joint order (body.py `_standard_joints`).
    pub fn joints(&self, pack: &Pack, rest: &[Vec3]) -> Vec<Vec3> {
        let joint = |group: usize| -> DVec3 {
            let vertices = &pack.mh_groups[group];
            if vertices.is_empty() {
                return DVec3::ZERO;
            }
            vertices.iter().map(|v| rest[*v as usize].as_dvec3()).sum::<DVec3>() / vertices.len() as f64 + self.shift
        };
        let posed = |group: usize| -> DVec3 {
            match pack.posed_rule.get(group).copied().flatten() {
                Some((bone, false)) => self.head_posed[bone],
                Some((bone, true)) => {
                    let tail = joint(pack.mh_bones[bone].tail);
                    self.head_posed[bone] + self.rotation[bone] * (tail - self.head_rest[bone])
                }
                None => joint(group),
            }
        };
        pack.joint_sources
            .iter()
            .map(|groups| {
                if groups.is_empty() {
                    return Vec3::ZERO;
                }
                (groups.iter().map(|g| posed(*g)).sum::<DVec3>() / groups.len() as f64).as_vec3()
            })
            .collect()
    }
}

fn smooth_normals(positions: &[Vec3], triangles: &[[u32; 3]], smooth: &[u32]) -> Vec<Vec3> {
    let mut by_id: HashMap<u32, Vec3> = HashMap::new();
    for t in triangles {
        let (a, b, c) = (positions[t[0] as usize], positions[t[1] as usize], positions[t[2] as usize]);
        let n = (b - a).cross(c - a);
        for &i in t {
            *by_id.entry(smooth[i as usize]).or_insert(Vec3::ZERO) += n;
        }
    }
    smooth
        .iter()
        .map(|id| by_id.get(id).copied().unwrap_or(Vec3::Z).try_normalize().unwrap_or(Vec3::Z))
        .collect()
}

fn skin_vertex(pairs: impl Iterator<Item = (u8, f32)>) -> SkinVertex {
    let mut joints = [0u8; 4];
    let mut weights = [0u16; 4];
    let list: Vec<(u8, f32)> = pairs.take(4).collect();
    let total: f32 = list.iter().map(|(_, w)| *w).sum::<f32>().max(1e-9);
    for (k, (j, w)) in list.into_iter().enumerate() {
        joints[k] = j;
        weights[k] = aurora_render::pack_skin_weight(w / total);
    }
    SkinVertex { joints, weights }
}

/// Parts over 65,535 vertices are cut into faces with their own vertices.
fn faces(
    name: &str,
    positions: &[Vec3],
    normals: &[Vec3],
    uvs: &[[f32; 2]],
    skin: &[SkinVertex],
    triangles: &[[u32; 3]],
    material: &PartMaterial,
) -> Vec<BuiltPart> {
    let mut out = Vec::new();
    let mut remap: HashMap<u32, u16> = HashMap::new();
    let mut part = new_part(name, 0, material);
    for t in triangles {
        let fresh = t.iter().filter(|i| !remap.contains_key(i)).count();
        if part.vertices.len() + fresh > u16::MAX as usize {
            out.push(std::mem::replace(&mut part, new_part(name, out.len() + 1, material)));
            remap.clear();
        }
        for &i in t {
            let index = *remap.entry(i).or_insert_with(|| {
                let p = positions[i as usize];
                part.min = part.min.min(p);
                part.max = part.max.max(p);
                // the pack's uvs are glTF's (v down, image rows top first);
                // the viewer's are v up, as its glTF importer turns them
                // (aurora-assets import_mesh/gltf.rs)
                let uv = uvs[i as usize];
                part.vertices.push(Vertex::new(p.to_array(), normals[i as usize].to_array(), [uv[0], 1.0 - uv[1]]));
                part.skin.push(skin[i as usize]);
                (part.vertices.len() - 1) as u16
            });
            part.indices.push(index);
        }
    }
    if !part.indices.is_empty() {
        out.push(part);
    }
    out
}

fn new_part(name: &str, n: usize, material: &PartMaterial) -> BuiltPart {
    BuiltPart {
        name: if n == 0 { name.to_owned() } else { format!("{name} {n}") },
        item: name.to_owned(),
        vertices: Vec::new(),
        indices: Vec::new(),
        skin: Vec::new(),
        material: material.clone(),
        min: Vec3::splat(f32::MAX),
        max: Vec3::splat(f32::MIN),
    }
}

fn tinted(texture: Option<String>, base: [f32; 3], tint: [f32; 3]) -> ([f32; 3], Option<String>) {
    ([base[0] * tint[0], base[1] * tint[1], base[2] * tint[2]], texture)
}

/// The tint the look gives a worn item of this kind.
fn tint_of(look: &Look, proxy: &super::pack::Proxy) -> [f32; 3] {
    match proxy.kind.as_str() {
        "eyes" => look.eye_tint,
        "eyebrows" | "eyelashes" => look.eyebrow_tint,
        "hair" => look.hair_tint,
        _ => look.clothes_tints.get(&proxy.id).copied().unwrap_or(super::look::WHITE),
    }
}

/// How the look draws an item ("body" or a content id): its texture in the
/// pack, its colour, blending and sides.
pub fn material_of(pack: &Pack, look: &Look, item: &str) -> PartMaterial {
    if item == "body" {
        let skin_texture = pack.content.swatch(&look.skin).map(|s| s.texture.clone());
        let (color, texture) = tinted(skin_texture, [1.0, 1.0, 1.0], look.skin_tint);
        return PartMaterial {
            texture,
            color,
            blend: false,
            mask: false,
            cutoff: DEFAULT_CUTOFF,
            two_sided: false,
        };
    }
    let Some(proxy) = pack.content.proxy(item) else {
        return PartMaterial {
            texture: None,
            color: super::look::WHITE,
            blend: false,
            mask: false,
            cutoff: DEFAULT_CUTOFF,
            two_sided: false,
        };
    };
    let eye_texture = (proxy.kind == "eyes")
        .then(|| pack.content.swatch(&look.eye_colour).map(|s| s.texture.clone()))
        .flatten();
    let texture = eye_texture.or_else(|| proxy.material.texture.clone());
    let base = if texture.is_some() { [1.0, 1.0, 1.0] } else { proxy.material.color };
    let (color, texture) = tinted(texture, base, tint_of(look, proxy));
    PartMaterial {
        texture,
        color,
        blend: proxy.material.alpha == "BLEND",
        mask: proxy.material.alpha == "MASK" || proxy.material.mask_cutoff.is_some(),
        cutoff: proxy.material.mask_cutoff.unwrap_or(DEFAULT_CUTOFF),
        two_sided: proxy.material.two_sided || proxy.kind != "eyes",
    }
}

/// The worn items of a look, in the order they cover the body (eyes,
/// brows, lashes, hair, then clothes in layer order).
pub fn worn_items(look: &Look) -> Vec<String> {
    let mut items = vec![look.eyes.clone()];
    items.extend(look.eyebrows.iter().cloned());
    items.extend(look.eyelashes.iter().cloned());
    items.extend(look.hair.iter().cloned());
    items.extend(look.clothes.iter().cloned());
    items
}

/// Every pack file a look needs: the worn items' meshes and every texture
/// drawn (skin, eye colour, items).
pub fn files_of(pack: &Pack, look: &Look) -> Vec<String> {
    let mut files = Vec::new();
    let mut push = |name: Option<String>| {
        if let Some(name) = name
            && !files.contains(&name)
        {
            files.push(name);
        }
    };
    push(material_of(pack, look, "body").texture);
    for item in worn_items(look) {
        if let Some(proxy) = pack.content.proxy(&item) {
            push(Some(proxy.mesh.clone()));
            push(material_of(pack, look, &item).texture);
        }
    }
    files
}

/// Builds the body for a look. `meshes` holds the worn items' meshes, read
/// from the pack (a missing one is left off, not the body).
pub fn build(pack: &Pack, look: &Look, meshes: &HashMap<String, Arc<ProxyMesh>>) -> BuiltBody {
    let shape = positions(pack, look);
    let pose = Pose::of(pack, &shape);
    let joint_positions = pose.joints(pack, &shape);

    // worn items, in the order they cover the body
    let worn: Vec<(&super::pack::Proxy, Arc<ProxyMesh>)> = worn_items(look)
        .iter()
        .filter_map(|id| Some((pack.content.proxy(id)?, meshes.get(id)?.clone())))
        .collect();

    // what the clothes cover is not drawn
    let hidden: HashSet<u16> = worn
        .iter()
        .filter(|(p, _)| p.kind == "clothes")
        .flat_map(|(_, m)| m.hides.iter().copied())
        .collect();

    let mut parts = Vec::new();
    // the body
    let body = &pack.body;
    let body_positions: Vec<Vec3> = body
        .vertices
        .iter()
        .map(|v| {
            let v = *v as usize;
            pose.point(shape[v], pack.mh_weights[v].iter().copied())
        })
        .collect();
    let triangles: Vec<[u32; 3]> = body
        .triangles
        .iter()
        .copied()
        .filter(|t| t.iter().all(|i| !hidden.contains(&(body.vertices[*i as usize] as u16))))
        .collect();
    let normals = smooth_normals(&body_positions, &body.triangles, &body.vertices);
    let skin: Vec<SkinVertex> = body
        .skin
        .iter()
        .map(|pairs| skin_vertex(pairs.iter().map(|(j, w)| (*j as u8, *w))))
        .collect();
    let material = material_of(pack, look, "body");
    parts.extend(faces("body", &body_positions, &normals, &body.uvs, &skin, &triangles, &material));

    // the worn items
    for (proxy, mesh) in &worn {
        let scale = Vec3::from_array(std::array::from_fn(|axis| {
            proxy.scales.get(&axis.to_string()).map_or(1.0, |(a, b, reference)| {
                let d = (shape[*a as usize][axis] - shape[*b as usize][axis]).abs();
                if *reference > 1e-9 { d / reference } else { 1.0 }
            })
        }));
        // fitted on the rest shape, posed with the blend of its ties'
        // weights (the positive part of each tie: body.py `proxy`)
        let positions: Vec<Vec3> = mesh
            .refs
            .iter()
            .zip(&mesh.bary)
            .zip(&mesh.offsets)
            .map(|((r, b), o)| {
                let rest = shape[r[0] as usize] * b[0] + shape[r[1] as usize] * b[1] + shape[r[2] as usize] * b[2] + *o * scale;
                let mut blend: Vec<(u16, f32)> = Vec::with_capacity(8);
                for (k, &vertex) in r.iter().enumerate() {
                    if b[k] <= 0.0 {
                        continue;
                    }
                    for &(bone, w) in &pack.mh_weights[vertex as usize] {
                        match blend.iter_mut().find(|(other, _)| *other == bone) {
                            Some(entry) => entry.1 += b[k] * w,
                            None => blend.push((bone, b[k] * w)),
                        }
                    }
                }
                pose.point(rest, blend.into_iter())
            })
            .collect();
        let smooth: Vec<u32> = (0..positions.len() as u32).collect();
        let normals = smooth_normals(&positions, &mesh.triangles, &smooth);
        let skin: Vec<SkinVertex> = mesh
            .joints
            .iter()
            .zip(&mesh.weights)
            .map(|(j, w)| SkinVertex { joints: *j, weights: *w })
            .collect();
        let material = material_of(pack, look, &proxy.id);
        parts.extend(faces(&proxy.id, &positions, &normals, &mesh.uvs, &skin, &mesh.triangles, &material));
    }

    let (low, high) = body_positions.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.z), hi.max(p.z)));
    BuiltBody {
        parts,
        joints: joint_positions,
        height: (high - low).max(0.0),
    }
}
