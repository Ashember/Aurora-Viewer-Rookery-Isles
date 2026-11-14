//! The Rookery body pack (tools/makehuman/rookery_body/pack.py in the
//! HomeWorldz tree; docs/ROOKERY_BODY.md): MakeHuman's body at rest (its
//! A-pose, in viewer space), every slider target, MakeHuman's skeleton with
//! its weights and the direction each limb turns to for the standard
//! skeleton's T-pose, the standard skeleton's skin, and the content
//! library. Read once, shared by every body built from it; each shape is
//! posed by `build`, as the HomeWorldz converter (body.py) poses it.

use glam::Vec3;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

const BODY_MAGIC: &[u8; 4] = b"RKBD";
const PROXY_MAGIC: &[u8; 4] = b"RKPX";
pub const VERSION: u32 = 2;

/// One target's offsets: (base vertex, delta in `delta_step` units).
pub struct Target {
    pub key: String,
    pub variables: Vec<String>,
    pub entries: Vec<(u16, [i16; 3])>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TargetInfo {
    pub key: String,
    pub variables: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Modifier {
    pub name: String,
    pub group: String,
    pub kind: String,
    #[serde(default)]
    pub left: Option<String>,
    #[serde(default)]
    pub right: Option<String>,
    #[serde(default)]
    pub center: Option<String>,
    #[serde(default)]
    pub default: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BodyPart {
    pub vertices: Vec<u32>,
    pub uvs: Vec<[f32; 2]>,
    pub triangles: Vec<[u32; 3]>,
    /// per vertex: (joint index, weight) pairs
    pub skin: Vec<Vec<(u32, f32)>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Material {
    pub texture: Option<String>,
    pub color: [f32; 3],
    pub alpha: String,
    #[serde(default)]
    pub two_sided: bool,
}

/// A worn item: eyes, eyebrows, eyelashes, hair, clothes.
#[derive(Debug, Clone, Deserialize)]
pub struct Proxy {
    pub id: String,
    pub kind: String,
    pub name: String,
    #[serde(default)]
    pub licence: String,
    /// Who made it (CC-BY items are credited).
    #[serde(default)]
    pub author: String,
    /// The Character Creator's shelf for clothes ("Tops", "Shoes", ...).
    #[serde(default)]
    pub category: String,
    pub mesh: String,
    /// viewer axis ("0".."2") -> (vertex a, vertex b, reference distance m)
    #[serde(default)]
    pub scales: BTreeMap<String, (u32, u32, f32)>,
    pub material: Material,
    #[serde(default)]
    pub z_depth: i32,
    pub thumbnail: Option<String>,
}

/// A texture only choice: skins and eye colours.
#[derive(Debug, Clone, Deserialize)]
pub struct Swatch {
    pub id: String,
    pub name: String,
    pub texture: String,
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub licence: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Content {
    #[serde(default)]
    pub skin: Vec<Swatch>,
    #[serde(default)]
    pub eyecolour: Vec<Swatch>,
    #[serde(default)]
    pub eyes: Vec<Proxy>,
    #[serde(default)]
    pub eyebrows: Vec<Proxy>,
    #[serde(default)]
    pub eyelashes: Vec<Proxy>,
    #[serde(default)]
    pub hair: Vec<Proxy>,
    #[serde(default)]
    pub clothes: Vec<Proxy>,
}

impl Content {
    pub fn proxy(&self, id: &str) -> Option<&Proxy> {
        [&self.eyes, &self.eyebrows, &self.eyelashes, &self.hair, &self.clothes]
            .into_iter()
            .flatten()
            .find(|p| p.id == id)
    }

    pub fn swatch(&self, id: &str) -> Option<&Swatch> {
        self.skin.iter().chain(&self.eyecolour).find(|s| s.id == id)
    }
}

/// One bone of MakeHuman's skeleton (parents before children).
#[derive(Debug, Clone)]
pub struct MhBone {
    /// Joint groups (`Pack::mh_groups`) of its head and tail.
    pub head: usize,
    pub tail: usize,
    pub parent: Option<usize>,
    /// The direction it turns to for the T-pose (limb bones only).
    pub target: Option<Vec3>,
}

#[derive(Debug, Clone, Deserialize)]
struct BoneEntry {
    head: String,
    tail: String,
    parent: i64,
    #[serde(default)]
    target: Option<[f32; 3]>,
}

#[derive(Debug, Clone, Deserialize)]
struct Document {
    version: u32,
    delta_step: f32,
    vertex_count: usize,
    joints: Vec<String>,
    joint_sources: HashMap<String, Vec<String>>,
    joint_groups: HashMap<String, Vec<u32>>,
    mh_bones: Vec<BoneEntry>,
    posed_joint_rule: HashMap<String, (usize, u8)>,
    mh_weights: Vec<Vec<(usize, f32)>>,
    pelvis_group: String,
    targets: Vec<TargetInfo>,
    modifiers: Vec<Modifier>,
    #[serde(default)]
    sliders: serde_json::Value,
    body: BodyPart,
    #[serde(default)]
    content: Content,
}

/// The loaded pack.
pub struct Pack {
    pub folder: PathBuf,
    pub delta_step: f32,
    /// Rest (A-pose) positions of every MakeHuman vertex, helpers included,
    /// in viewer space; a shape adds its targets' offsets to these.
    pub base: Vec<Vec3>,
    pub targets: Vec<Target>,
    pub modifiers: Vec<Modifier>,
    /// Target indices by key (MakeHuman's Targets.groups).
    pub groups: HashMap<String, Vec<usize>>,
    /// Standard bones the body is rigged to, in skin index order.
    pub joints: Vec<String>,
    /// Per standard joint: the MakeHuman joint groups whose posed joints
    /// it is the mean of.
    pub joint_sources: Vec<Vec<usize>>,
    /// MakeHuman's joint groups: the vertices whose mean is each joint.
    pub mh_groups: Vec<Vec<u32>>,
    pub mh_bones: Vec<MhBone>,
    /// Per joint group: the bone whose posed head (false) or tail (true)
    /// it is.
    pub posed_rule: Vec<Option<(usize, bool)>>,
    /// MakeHuman's weights per vertex: (bone, weight), positive.
    pub mh_weights: Vec<Vec<(u16, f32)>>,
    /// The joint group the body is centred on (MakeHuman's pelvis).
    pub pelvis_group: usize,
    pub sliders: serde_json::Value,
    pub body: BodyPart,
    pub content: Content,
}

/// A worn item's mesh, read from its own file when it is first worn.
pub struct ProxyMesh {
    pub refs: Vec<[u16; 3]>,
    pub bary: Vec<[f32; 3]>,
    pub offsets: Vec<Vec3>,
    pub uvs: Vec<[f32; 2]>,
    pub joints: Vec<[u8; 4]>,
    pub weights: Vec<[u16; 4]>,
    pub triangles: Vec<[u32; 3]>,
    /// Body vertices this item covers and hides.
    pub hides: Vec<u16>,
}

fn take<'a>(data: &'a [u8], at: &mut usize, n: usize, what: &str) -> Result<&'a [u8], String> {
    let end = at.checked_add(n).filter(|e| *e <= data.len()).ok_or_else(|| format!("truncated {what}"))?;
    let slice = &data[*at..end];
    *at = end;
    Ok(slice)
}

fn u32_at(data: &[u8], at: &mut usize, what: &str) -> Result<u32, String> {
    let b = take(data, at, 4, what)?;
    Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn f32s(bytes: &[u8]) -> impl Iterator<Item = f32> + '_ {
    bytes.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
}

impl Pack {
    /// Reads body.json and body.bin from `folder`.
    pub fn load(folder: &Path) -> Result<Pack, String> {
        let text = std::fs::read(folder.join("body.json")).map_err(|e| format!("body.json: {e}"))?;
        let document: Document = serde_json::from_slice(&text).map_err(|e| format!("body.json: {e}"))?;
        if document.version != VERSION {
            return Err(format!("body pack version {} (this viewer reads {VERSION})", document.version));
        }
        let data = std::fs::read(folder.join("body.bin")).map_err(|e| format!("body.bin: {e}"))?;
        let mut at = 0;
        if take(&data, &mut at, 4, "magic")? != BODY_MAGIC {
            return Err("body.bin is not a Rookery body".into());
        }
        let version = u32_at(&data, &mut at, "version")?;
        if version != VERSION {
            return Err(format!("body.bin version {version}"));
        }
        let count = u32_at(&data, &mut at, "vertex count")? as usize;
        if count != document.vertex_count || count > u16::MAX as usize + 1 {
            return Err("body.bin vertex count does not match body.json".into());
        }
        let base_bytes = take(&data, &mut at, count * 12, "base")?;
        let floats: Vec<f32> = f32s(base_bytes).collect();
        let base: Vec<Vec3> = floats.chunks_exact(3).map(|c| Vec3::new(c[0], c[1], c[2])).collect();
        let target_count = u32_at(&data, &mut at, "target count")? as usize;
        if target_count != document.targets.len() {
            return Err("body.bin target count does not match body.json".into());
        }
        let mut targets = Vec::with_capacity(target_count);
        for info in &document.targets {
            let n = u32_at(&data, &mut at, "target entries")? as usize;
            let bytes = take(&data, &mut at, n * 8, "target")?;
            let entries = bytes
                .chunks_exact(8)
                .map(|c| {
                    let vertex = u16::from_le_bytes([c[0], c[1]]);
                    let d = |k: usize| i16::from_le_bytes([c[2 + 2 * k], c[3 + 2 * k]]);
                    (vertex, [d(0), d(1), d(2)])
                })
                .filter(|(v, _)| (*v as usize) < count)
                .collect();
            targets.push(Target {
                key: info.key.clone(),
                variables: info.variables.clone(),
                entries,
            });
        }
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, t) in targets.iter().enumerate() {
            groups.entry(t.key.clone()).or_default().push(i);
        }
        // MakeHuman's joint groups by index
        let mut group_names: Vec<&String> = document.joint_groups.keys().collect();
        group_names.sort();
        let group_index: HashMap<&str, usize> = group_names.iter().enumerate().map(|(i, n)| (n.as_str(), i)).collect();
        let mh_groups: Vec<Vec<u32>> = group_names
            .iter()
            .map(|n| document.joint_groups[*n].iter().copied().filter(|v| (*v as usize) < count).collect())
            .collect();
        let group = |name: &str| group_index.get(name).copied().ok_or_else(|| format!("joint group {name} is not in the pack"));
        let mut mh_bones = Vec::with_capacity(document.mh_bones.len());
        for (i, b) in document.mh_bones.iter().enumerate() {
            let parent = usize::try_from(b.parent).ok();
            if parent.is_some_and(|p| p >= i) {
                return Err("MakeHuman's bones are not parents first".into());
            }
            mh_bones.push(MhBone {
                head: group(&b.head)?,
                tail: group(&b.tail)?,
                parent,
                target: b.target.map(Vec3::from_array),
            });
        }
        let mut posed_rule = vec![None; mh_groups.len()];
        for (name, (bone, tail)) in &document.posed_joint_rule {
            if *bone >= mh_bones.len() {
                return Err("posed joint rule names a bone the pack has not".into());
            }
            posed_rule[group(name)?] = Some((*bone, *tail != 0));
        }
        if document.mh_weights.len() != count {
            return Err("MakeHuman's weights do not match the base".into());
        }
        let mh_weights = document
            .mh_weights
            .iter()
            .map(|pairs| pairs.iter().filter(|(b, w)| *b < mh_bones.len() && *w > 0.0).map(|(b, w)| (*b as u16, *w)).collect())
            .collect();
        let joint_sources = document
            .joints
            .iter()
            .map(|j| document.joint_sources.get(j).into_iter().flatten().filter_map(|g| group_index.get(g.as_str()).copied()).collect())
            .collect();
        let pelvis_group = group(&document.pelvis_group)?;
        if document.body.vertices.iter().any(|v| *v as usize >= count) || document.body.skin.len() != document.body.vertices.len() {
            return Err("body part does not match the base".into());
        }
        Ok(Pack {
            folder: folder.to_path_buf(),
            delta_step: document.delta_step,
            base,
            targets,
            modifiers: document.modifiers,
            groups,
            joints: document.joints,
            joint_sources,
            mh_groups,
            mh_bones,
            posed_rule,
            mh_weights,
            pelvis_group,
            sliders: document.sliders,
            body: document.body,
            content: document.content,
        })
    }

    /// A worn item's mesh file.
    pub fn proxy_mesh(&self, proxy: &Proxy) -> Result<ProxyMesh, String> {
        let data = std::fs::read(self.folder.join(&proxy.mesh)).map_err(|e| format!("{}: {e}", proxy.mesh))?;
        parse_proxy(&data, self.base.len())
    }

}

pub fn parse_proxy(data: &[u8], base_count: usize) -> Result<ProxyMesh, String> {
    let mut at = 0;
    if take(data, &mut at, 4, "magic")? != PROXY_MAGIC {
        return Err("not a Rookery content mesh".into());
    }
    let version = u32_at(data, &mut at, "version")?;
    if version != VERSION {
        return Err(format!("content mesh version {version}"));
    }
    let vertices = u32_at(data, &mut at, "vertices")? as usize;
    let triangles = u32_at(data, &mut at, "triangles")? as usize;
    let hides = u32_at(data, &mut at, "hides")? as usize;
    const RECORD: usize = 6 + 12 + 12 + 8 + 4 + 8;
    let records = take(data, &mut at, vertices * RECORD, "vertices")?;
    let mut mesh = ProxyMesh {
        refs: Vec::with_capacity(vertices),
        bary: Vec::with_capacity(vertices),
        offsets: Vec::with_capacity(vertices),
        uvs: Vec::with_capacity(vertices),
        joints: Vec::with_capacity(vertices),
        weights: Vec::with_capacity(vertices),
        triangles: Vec::with_capacity(triangles),
        hides: Vec::with_capacity(hides),
    };
    for r in records.chunks_exact(RECORD) {
        let u16_at = |o: usize| u16::from_le_bytes([r[o], r[o + 1]]);
        let f = |o: usize| f32::from_le_bytes([r[o], r[o + 1], r[o + 2], r[o + 3]]);
        let refs = [u16_at(0), u16_at(2), u16_at(4)];
        if refs.iter().any(|v| *v as usize >= base_count) {
            return Err("content mesh ties to a vertex the body has not".into());
        }
        mesh.refs.push(refs);
        mesh.bary.push([f(6), f(10), f(14)]);
        mesh.offsets.push(Vec3::new(f(18), f(22), f(26)));
        mesh.uvs.push([f(30), f(34)]);
        mesh.joints.push([r[38], r[39], r[40], r[41]]);
        mesh.weights.push([u16_at(42), u16_at(44), u16_at(46), u16_at(48)]);
    }
    let tri = take(data, &mut at, triangles * 12, "triangles")?;
    for c in tri.chunks_exact(12) {
        let t = |k: usize| u32::from_le_bytes([c[4 * k], c[4 * k + 1], c[4 * k + 2], c[4 * k + 3]]);
        let triangle = [t(0), t(1), t(2)];
        if triangle.iter().any(|i| *i as usize >= vertices) {
            return Err("content mesh triangle out of range".into());
        }
        mesh.triangles.push(triangle);
    }
    let hidden = take(data, &mut at, hides * 2, "hides")?;
    mesh.hides = hidden.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Ok(mesh)
}
