//! The Rookery body in the scene (docs/ROOKERY_BODY.md): on the Rookery Isles
//! grid every avatar with a saved look is drawn with the MakeHuman body that
//! look builds, in place of the system body, skinned to the same standard
//! skeleton so every animation, sit and gesture plays on it.
//!
//! - the body pack comes from the grid (`rookery_body::fetch`) and is read
//!   once
//! - the look of each avatar comes from the grid (POST /rookery/looks), or,
//!   for this viewer's own avatar in the Character Creator, from the creator
//!   as it changes (`set_local_look`)
//! - a body is built on a worker for each shape (`rookery_body::build`),
//!   uploaded as one geometry (`GeomKey::RookeryBody`) whose faces are its
//!   parts, and its joints move the avatar's skeleton (`apply_joints`, from
//!   `skeleton_of`)
//! - its textures are pack files, decoded on a worker and given texture
//!   slots of their own (`register_local_image`)

use super::*;
use crate::rookery_body::build::{self, BuiltBody};
use crate::rookery_body::fetch::{self, Manifest};
use crate::rookery_body::look::Look;
use crate::rookery_body::pack::{Pack, ProxyMesh};
use std::sync::Mutex;
use std::time::Duration;

/// How long an avatar's look is trusted before the grid is asked again.
const LOOKS_EVERY: Duration = Duration::from_secs(30);
/// How often the avatars around are looked over for looks to ask.
const SCAN_EVERY: Duration = Duration::from_secs(2);
/// A failed download or build is tried again after this.
const RETRY_AFTER: Duration = Duration::from_secs(30);
/// A body no avatar has drawn for this long is let go.
const UNUSED_FOR: Duration = Duration::from_secs(90);
/// Builds and texture decodes running at once.
const BUILDS_AT_ONCE: usize = 2;
const DECODES_AT_ONCE: usize = 4;
/// Ids per looks request (the grid takes up to 256).
const LOOKS_PER_REQUEST: usize = 200;

/// A body ready to draw: its geometry, skin bindings and joints.
pub(crate) struct BodyMeta {
    pub key: GeomKey,
    /// The skin bindings' id (`SkinBinds::mesh`).
    pub skin_id: Uuid,
    pub skin: Arc<aurora_assets::SkinInfo>,
    /// (rig joint, rest position in skeleton space) of every joint the body
    /// places.
    pub joints: Vec<(usize, Vec3)>,
    /// What each face of the geometry shows ("body" or a content id).
    pub items: Vec<String>,
    pub height: f32,
}

enum BodyState {
    Building,
    Ready(Arc<BodyMeta>),
    Failed(Instant),
}

enum TexState {
    Loading,
    Ready { id: Uuid, slot: u32 },
    Failed(Instant),
}

enum PackState {
    Off,
    Loading,
    Ready(Arc<Pack>, Arc<Manifest>),
    Failed(Instant, String),
}

struct Built {
    key: u64,
    result: Result<BuiltBody, String>,
}

struct Decoded {
    file: String,
    result: Result<(Vec<(u32, u32, Vec<u8>)>, AlphaKind, bool), String>,
}

/// What the Character Creator shows while the pack comes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PackStatus {
    /// Not the Rookery Isles grid.
    #[default]
    Off,
    Loading,
    Ready,
    Failed(String),
}

pub(crate) struct RookeryBodies {
    /// The grid's service address (Rookery Isles only).
    service: Option<String>,
    pack: PackState,
    pack_rx: Option<crossbeam_channel::Receiver<Result<(Pack, Manifest), String>>>,
    /// Looks the grid gave, by avatar id (None: that avatar has none).
    looks: HashMap<Uuid, (Option<Arc<Look>>, Instant)>,
    /// Looks this viewer sets itself (its own avatar in the creator).
    local: HashMap<Uuid, Arc<Look>>,
    looks_rx: Option<crossbeam_channel::Receiver<Result<HashMap<Uuid, Look>, String>>>,
    asked: Vec<Uuid>,
    scanned: Option<Instant>,
    bodies: HashMap<u64, BodyState>,
    used: HashMap<u64, Instant>,
    built_tx: crossbeam_channel::Sender<Built>,
    built_rx: crossbeam_channel::Receiver<Built>,
    building: usize,
    meshes: Arc<Mutex<HashMap<String, Arc<ProxyMesh>>>>,
    textures: HashMap<String, TexState>,
    decoded_tx: crossbeam_channel::Sender<Decoded>,
    decoded_rx: crossbeam_channel::Receiver<Decoded>,
    decoding: usize,
    /// Bumped by everything that changes how an avatar is drawn (a body
    /// built, a texture ready, a look changed): the avatars are synced
    /// again when it moves on from `touched` (a still avatar is not synced
    /// every frame).
    generation: u64,
    touched: u64,
    /// The body each avatar was last drawn with: kept while its next shape
    /// builds (a slider dragged), so it never falls back to the system body.
    shown: HashMap<Uuid, u64>,
    /// When the state was last written to the log.
    reported: Option<Instant>,
}

impl Default for RookeryBodies {
    fn default() -> Self {
        let (built_tx, built_rx) = crossbeam_channel::unbounded();
        let (decoded_tx, decoded_rx) = crossbeam_channel::unbounded();
        RookeryBodies {
            service: None,
            pack: PackState::Off,
            pack_rx: None,
            looks: HashMap::new(),
            local: HashMap::new(),
            looks_rx: None,
            asked: Vec::new(),
            scanned: None,
            bodies: HashMap::new(),
            used: HashMap::new(),
            built_tx,
            built_rx,
            building: 0,
            meshes: Arc::new(Mutex::new(HashMap::new())),
            textures: HashMap::new(),
            decoded_tx,
            decoded_rx,
            decoding: 0,
            generation: 0,
            touched: 0,
            shown: HashMap::new(),
            reported: None,
        }
    }
}

/// A stable id for a pack texture's slot.
fn texture_id(file: &str) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"Rookery body texture v1 ");
    hash.update(file.as_bytes());
    let digest = hash.finalize();
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    Uuid::from_bytes(bytes)
}

/// The geometry key of a shape on this pack.
fn body_key(pack: &Manifest, look: &Look) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    pack.entry("body.bin").map(|e| e.sha256.as_str()).unwrap_or("").hash(&mut h);
    look.shape_hash().hash(&mut h);
    h.finish()
}

/// The avatar's skeleton moved onto the body's joints: each joint the body
/// places is put where the body has it (its position relative to its
/// parent as the skeleton then stands), every other joint keeps its offset
/// from its parent; scales stay the skeleton's own (the body is shaped by
/// its sliders, not by the bone scales of a system shape).
pub(crate) fn apply_joints(rig: &anim::Rig, joints: &[(usize, Vec3)], local_pos: &mut [Vec3], scale: &mut [Vec3]) {
    let n = rig.len().min(local_pos.len()).min(scale.len());
    let mut target: Vec<Option<Vec3>> = vec![None; n];
    for &(j, p) in joints {
        if j < n && p.is_finite() {
            target[j] = Some(p);
        }
    }
    for j in 0..n {
        scale[j] = rig.scale[j];
    }
    let mut world = vec![Vec3::ZERO; n];
    for &j in &rig.order {
        if j >= n {
            continue;
        }
        let (base, parent_scale) = match rig.parent[j] {
            Some(p) if p < n => (world[p], scale[p]),
            _ => (Vec3::ZERO, rig.parent_scale[j]),
        };
        match target[j] {
            Some(p) => {
                let safe = Vec3::new(
                    if parent_scale.x.abs() > 1e-6 { parent_scale.x } else { 1.0 },
                    if parent_scale.y.abs() > 1e-6 { parent_scale.y } else { 1.0 },
                    if parent_scale.z.abs() > 1e-6 { parent_scale.z } else { 1.0 },
                );
                local_pos[j] = (p - base) / safe;
                world[j] = p;
            }
            None => world[j] = base + local_pos[j] * parent_scale,
        }
    }
}

impl RookeryBodies {
    /// The grid this viewer is on: the Rookery body only on Rookery Isles
    /// (`service` its address), nothing elsewhere.
    pub fn set_grid(&mut self, service: Option<String>) {
        if self.service == service {
            return;
        }
        let keep_pack = matches!(self.pack, PackState::Ready(..)) && service.is_some();
        self.service = service;
        if !keep_pack {
            self.pack = PackState::Off;
            self.pack_rx = None;
        }
        self.looks.clear();
        self.local.clear();
        self.looks_rx = None;
        self.asked.clear();
        self.generation += 1;
    }

    pub fn enabled(&self) -> bool {
        self.service.is_some()
    }

    pub fn status(&self) -> PackStatus {
        match &self.pack {
            PackState::Off if self.service.is_none() => PackStatus::Off,
            PackState::Off | PackState::Loading => PackStatus::Loading,
            PackState::Ready(..) => PackStatus::Ready,
            PackState::Failed(_, e) => PackStatus::Failed(e.clone()),
        }
    }

    /// The pack, once read (the Character Creator lists its content).
    pub fn pack(&self) -> Option<Arc<Pack>> {
        match &self.pack {
            PackState::Ready(pack, _) => Some(pack.clone()),
            _ => None,
        }
    }

    /// The grid's manifest of the pack (for thumbnails the creator fetches).
    pub fn manifest(&self) -> Option<Arc<Manifest>> {
        match &self.pack {
            PackState::Ready(_, manifest) => Some(manifest.clone()),
            _ => None,
        }
    }

    pub fn service(&self) -> Option<&str> {
        self.service.as_deref()
    }

    /// The look this viewer draws an avatar with while the creator edits it
    /// (None: the grid's again).
    pub fn set_local_look(&mut self, avatar: Uuid, look: Option<Look>) {
        match look {
            Some(look) => {
                if self.local.get(&avatar).is_none_or(|l| **l != look) {
                    self.local.insert(avatar, Arc::new(look));
                    self.generation += 1;
                }
            }
            None => {
                if self.local.remove(&avatar).is_some() {
                    self.generation += 1;
                }
            }
        }
    }

    /// A look saved on the grid: drawn at once by this viewer, as the grid
    /// will give it from now on.
    pub fn note_saved(&mut self, avatar: Uuid, look: Look) {
        self.looks.insert(avatar, (Some(Arc::new(look)), Instant::now()));
        self.generation += 1;
    }

    /// Forget the grid's look of an avatar: asked again at once.
    pub fn forget(&mut self, avatar: &Uuid) {
        self.looks.remove(avatar);
        self.scanned = None;
        self.generation += 1;
    }

    /// The look an avatar is drawn with, if any.
    pub fn look_of(&self, avatar: &Uuid) -> Option<Arc<Look>> {
        self.local
            .get(avatar)
            .cloned()
            .or_else(|| self.looks.get(avatar).and_then(|(l, _)| l.clone()))
    }


    /// The body an avatar is drawn with now, and its look: its look's body
    /// once built, the body it was drawn with before while that one builds;
    /// None while it has no look or no body was ever built for it.
    fn ready_body(&mut self, avatar: &Uuid) -> Option<(Arc<BodyMeta>, Arc<Look>)> {
        let Some(look) = self.look_of(avatar) else {
            self.shown.remove(avatar);
            return None;
        };
        let key = match &self.pack {
            PackState::Ready(_, manifest) => body_key(manifest, &look),
            _ => return None,
        };
        let key = match self.bodies.get(&key) {
            Some(BodyState::Ready(_)) => {
                self.shown.insert(*avatar, key);
                key
            }
            _ => *self.shown.get(avatar)?,
        };
        match self.bodies.get(&key) {
            Some(BodyState::Ready(meta)) => {
                self.used.insert(key, Instant::now());
                Some((meta.clone(), look))
            }
            _ => {
                self.shown.remove(avatar);
                None
            }
        }
    }

    /// The key of the body an avatar is drawn with (its look's, or the one
    /// before while that builds); 0: none.
    fn drawn_key(&self, avatar: &Uuid) -> u64 {
        let Some(look) = self.look_of(avatar) else {
            return 0;
        };
        let PackState::Ready(_, manifest) = &self.pack else {
            return 0;
        };
        let key = body_key(manifest, &look);
        if matches!(self.bodies.get(&key), Some(BodyState::Ready(_))) {
            return key;
        }
        match self.shown.get(avatar) {
            Some(shown) if matches!(self.bodies.get(shown), Some(BodyState::Ready(_))) => *shown,
            _ => 0,
        }
    }

    /// The skeleton key of an avatar's body (0: none), for `skeleton_of`.
    pub fn skeleton_key(&self, avatar: &Uuid) -> u64 {
        self.drawn_key(avatar)
    }

    /// The joints of the body an avatar is drawn with.
    pub fn joints_of(&self, avatar: &Uuid) -> Option<Arc<BodyMeta>> {
        match self.bodies.get(&self.drawn_key(avatar)) {
            Some(BodyState::Ready(meta)) => Some(meta.clone()),
            _ => None,
        }
    }

    /// Whether a body is being built for an avatar's current look.
    pub fn building_for(&self, avatar: &Uuid) -> bool {
        let (Some(look), PackState::Ready(_, manifest)) = (self.look_of(avatar), &self.pack) else {
            return false;
        };
        matches!(self.bodies.get(&body_key(manifest, &look)), Some(BodyState::Building) | None)
    }

    fn texture_slot(&self, file: &str) -> Option<(Uuid, u32)> {
        match self.textures.get(file) {
            Some(TexState::Ready { id, slot }) => Some((*id, *slot)),
            _ => None,
        }
    }

    fn start_pack(&mut self, runtime: &tokio::runtime::Handle) {
        let Some(service) = self.service.clone() else {
            return;
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.pack = PackState::Loading;
        self.pack_rx = Some(rx);
        if let Some(folder) = fetch::local_pack() {
            log::info!("Rookery body: reading the pack of {}", folder.display());
            std::thread::spawn(move || {
                let _ = tx.send(fetch::local_manifest(&folder).and_then(|manifest| Pack::load(&folder).map(|pack| (pack, manifest))));
            });
            return;
        }
        runtime.spawn(async move {
            let base = fetch::base_url(&service);
            let folder = fetch::folder();
            let result = async {
                let manifest = fetch::manifest(&base, &folder).await?;
                let core: Vec<String> = fetch::CORE.iter().map(|s| (*s).to_owned()).collect();
                fetch::ensure(&base, &folder, &manifest, &core).await?;
                Ok::<_, String>(manifest)
            }
            .await;
            std::thread::spawn(move || {
                let _ = tx.send(result.and_then(|manifest| Pack::load(&folder).map(|pack| (pack, manifest))));
            });
        });
    }

    fn start_looks(&mut self, runtime: &tokio::runtime::Handle, ids: Vec<Uuid>) {
        let Some(service) = self.service.clone() else {
            return;
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        self.looks_rx = Some(rx);
        self.asked = ids.clone();
        runtime.spawn(async move {
            let _ = tx.send(fetch_looks(&service, &ids).await);
        });
    }

    fn start_build(&mut self, runtime: &tokio::runtime::Handle, key: u64, look: Arc<Look>) {
        let (Some(service), PackState::Ready(pack, manifest)) = (self.service.clone(), &self.pack) else {
            return;
        };
        let (pack, manifest) = (pack.clone(), manifest.clone());
        let files: Vec<String> = build::files_of(&pack, &look)
            .into_iter()
            .filter(|f| f.starts_with("content/"))
            .collect();
        let meshes = self.meshes.clone();
        let tx = self.built_tx.clone();
        self.bodies.insert(key, BodyState::Building);
        self.building += 1;
        log::info!("Rookery body: building {key:016x} ({} files to check)", files.len());
        runtime.spawn(async move {
            let base = fetch::base_url(&service);
            let fetched = fetch::ensure(&base, &pack.folder, &manifest, &files).await;
            std::thread::spawn(move || {
                let started = Instant::now();
                let result = fetched.and_then(|()| {
                    let mut worn = HashMap::new();
                    for item in build::worn_items(&look) {
                        let Some(proxy) = pack.content.proxy(&item) else {
                            continue;
                        };
                        let cached = meshes.lock().ok().and_then(|m| m.get(&item).cloned());
                        let mesh = match cached {
                            Some(mesh) => mesh,
                            None => {
                                let mesh = Arc::new(pack.proxy_mesh(proxy)?);
                                if let Ok(mut m) = meshes.lock() {
                                    m.insert(item.clone(), mesh.clone());
                                }
                                mesh
                            }
                        };
                        worn.insert(item, mesh);
                    }
                    Ok(build::build(&pack, &look, &worn))
                });
                if let Ok(body) = &result {
                    log::info!(
                        "Rookery body: built {key:016x} in {} ms ({} parts, {:.2} m)",
                        started.elapsed().as_millis(),
                        body.parts.len(),
                        body.height
                    );
                }
                let _ = tx.send(Built { key, result });
            });
        });
    }

    fn start_texture(&mut self, runtime: &tokio::runtime::Handle, file: String) {
        let (Some(service), PackState::Ready(pack, manifest)) = (self.service.clone(), &self.pack) else {
            return;
        };
        let (folder, manifest) = (pack.folder.clone(), manifest.clone());
        let tx = self.decoded_tx.clone();
        self.textures.insert(file.clone(), TexState::Loading);
        self.decoding += 1;
        runtime.spawn(async move {
            let base = fetch::base_url(&service);
            let fetched = fetch::ensure(&base, &folder, &manifest, std::slice::from_ref(&file)).await;
            std::thread::spawn(move || {
                let result = fetched.and_then(|()| decode(&folder.join(&file)));
                let _ = tx.send(Decoded { file, result });
            });
        });
    }
}

/// A pack texture read and prepared for its slot: its mip chain, its alpha
/// class and whether it has an alpha channel at all.
fn decode(path: &std::path::Path) -> Result<(Vec<(u32, u32, Vec<u8>)>, AlphaKind, bool), String> {
    let image = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let has_alpha = image.color().has_alpha();
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    let data = rgba.into_raw();
    let alpha = if has_alpha {
        super::jobs::classify_alpha(&data)
    } else {
        AlphaKind::Opaque
    };
    let levels = if alpha == AlphaKind::Opaque {
        aurora_render::build_mips(width, height, data, 13)
    } else {
        aurora_render::build_mips_alpha_weighted(width, height, data, 13)
    };
    Ok((levels, alpha, has_alpha))
}

/// The looks of `ids` from the grid: {"looks": {id: look}}; an avatar left
/// out has none.
async fn fetch_looks(service: &str, ids: &[Uuid]) -> Result<HashMap<Uuid, Look>, String> {
    #[derive(serde::Deserialize)]
    struct Answer {
        #[serde(default)]
        looks: HashMap<String, serde_json::Value>,
    }
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(20))
        .user_agent(aurora_version::user_agent())
        .build()
        .map_err(|e| e.to_string())?;
    let body = serde_json::json!({ "ids": ids.iter().map(|id| id.to_string()).collect::<Vec<_>>() });
    let response = client
        .post(format!("{}/rookery/looks", service.trim_end_matches('/')))
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| format!("looks: {e}"))?;
    let bytes = response.bytes().await.map_err(|e| format!("looks: {e}"))?;
    let answer: Answer = serde_json::from_slice(&bytes).map_err(|e| format!("looks: {e}"))?;
    Ok(answer
        .looks
        .into_iter()
        .filter_map(|(id, value)| Some((id.parse::<Uuid>().ok()?, serde_json::from_value::<Look>(value).ok()?)))
        .collect())
}

impl Scene {
    /// The grid this viewer is on (the app, at each login).
    pub fn set_rookery_grid(&mut self, service: Option<String>) {
        self.rookery.set_grid(service);
    }

    pub fn rookery_status(&self) -> PackStatus {
        self.rookery.status()
    }

    pub fn rookery_pack(&self) -> Option<Arc<Pack>> {
        self.rookery.pack()
    }

    pub fn rookery_manifest(&self) -> Option<Arc<Manifest>> {
        self.rookery.manifest()
    }

    pub fn rookery_service(&self) -> Option<String> {
        self.rookery.service().map(str::to_owned)
    }

    pub fn set_rookery_local_look(&mut self, avatar: Uuid, look: Option<Look>) {
        self.rookery.set_local_look(avatar, look);
    }

    pub fn note_rookery_saved(&mut self, avatar: Uuid, look: Look) {
        self.rookery.note_saved(avatar, look);
    }


    pub fn forget_rookery_look(&mut self, avatar: Uuid) {
        self.rookery.forget(&avatar);
    }

    /// The height of the body an avatar is drawn with (metres), once built.
    pub fn rookery_height(&self, avatar: &Uuid) -> Option<f32> {
        self.rookery.joints_of(avatar).map(|meta| meta.height)
    }

    /// The creator's own avatar: whether its body is still on its way.
    pub fn rookery_building(&self, avatar: &Uuid) -> bool {
        self.rookery.building_for(avatar)
    }

    /// One frame of the Rookery bodies: the pack, the looks of the avatars
    /// around, the bodies and textures they need, and what came back.
    pub(super) fn rookery_frame(&mut self, renderer: &mut Renderer, net: &NetClient, world: &World) {
        if !self.rookery.enabled() {
            return;
        }
        let runtime = net.runtime().clone();
        // the pack
        if let Some(rx) = &self.rookery.pack_rx {
            match rx.try_recv() {
                Ok(Ok((pack, manifest))) => {
                    log::info!(
                        "Rookery body: pack read ({} targets, {} skins, {} hair, {} clothes)",
                        pack.targets.len(),
                        pack.content.skin.len(),
                        pack.content.hair.len(),
                        pack.content.clothes.len()
                    );
                    self.rookery.pack = PackState::Ready(Arc::new(pack), Arc::new(manifest));
                    self.rookery.pack_rx = None;
                }
                Ok(Err(error)) => {
                    log::warn!("Rookery body: the pack did not load: {error}");
                    self.rookery.pack = PackState::Failed(Instant::now(), error);
                    self.rookery.pack_rx = None;
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.rookery.pack = PackState::Failed(Instant::now(), "the pack download stopped".into());
                    self.rookery.pack_rx = None;
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        match &self.rookery.pack {
            PackState::Off => self.rookery.start_pack(&runtime),
            PackState::Failed(at, _) if at.elapsed() > RETRY_AFTER => self.rookery.start_pack(&runtime),
            _ => {}
        }
        // looks
        if let Some(rx) = &self.rookery.looks_rx {
            match rx.try_recv() {
                Ok(result) => {
                    let now = Instant::now();
                    let asked = std::mem::take(&mut self.rookery.asked);
                    match result {
                        Ok(mut looks) => {
                            for id in asked {
                                let look = looks.remove(&id).map(Arc::new);
                                // (an unchanged look keeps its Arc: nothing rebuilds)
                                let same = matches!((self.rookery.looks.get(&id), &look), (Some((Some(a), _)), Some(b)) if **a == **b);
                                if same {
                                    if let Some(entry) = self.rookery.looks.get_mut(&id) {
                                        entry.1 = now;
                                    }
                                } else {
                                    self.rookery.looks.insert(id, (look, now));
                                    self.rookery.generation += 1;
                                }
                            }
                        }
                        Err(error) => {
                            log::debug!("Rookery body: {error}");
                            // asked again after the usual wait
                            for id in asked {
                                let entry = self.rookery.looks.entry(id).or_insert((None, now));
                                entry.1 = now;
                            }
                        }
                    }
                    self.rookery.looks_rx = None;
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => self.rookery.looks_rx = None,
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        if self.rookery.looks_rx.is_none() && self.rookery.scanned.is_none_or(|t| t.elapsed() >= SCAN_EVERY) {
            self.rookery.scanned = Some(Instant::now());
            let ids: Vec<Uuid> = world
                .objects
                .avatars()
                .map(|(_, o)| o.full_id)
                .chain(std::iter::once(world.agent_id))
                .filter(|id| !id.is_nil())
                .filter(|id| self.rookery.looks.get(id).is_none_or(|(_, at)| at.elapsed() >= LOOKS_EVERY))
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .take(LOOKS_PER_REQUEST)
                .collect();
            if !ids.is_empty() {
                self.rookery.start_looks(&runtime, ids);
            }
        }
        let PackState::Ready(pack, manifest) = &self.rookery.pack else {
            self.touch_rookery_avatars(world);
            return;
        };
        let (pack, manifest) = (pack.clone(), manifest.clone());
        // finished builds: uploaded as one geometry
        while let Ok(built) = self.rookery.built_rx.try_recv() {
            self.rookery.building = self.rookery.building.saturating_sub(1);
            match built.result {
                Ok(body) => {
                    let meta = self.upload_rookery_body(renderer, &pack, built.key, body);
                    self.rookery.bodies.insert(built.key, BodyState::Ready(Arc::new(meta)));
                    // (counted as used from now: a body not drawn yet is not stale)
                    self.rookery.used.insert(built.key, Instant::now());
                    self.rookery.generation += 1;
                }
                Err(error) => {
                    log::warn!("Rookery body: a body did not build: {error}");
                    self.rookery.bodies.insert(built.key, BodyState::Failed(Instant::now()));
                }
            }
        }
        // decoded textures: their own slots
        while let Ok(decoded) = self.rookery.decoded_rx.try_recv() {
            self.rookery.decoding = self.rookery.decoding.saturating_sub(1);
            let state = match decoded.result {
                Ok((levels, alpha, has_alpha)) => {
                    let mips: Vec<aurora_render::MipLevel> = levels
                        .iter()
                        .map(|(width, height, data)| aurora_render::MipLevel {
                            width: *width,
                            height: *height,
                            data,
                        })
                        .collect();
                    match renderer.create_texture(&mips) {
                        Some(slot) => {
                            log::info!("Rookery body: texture {} in slot {slot}", decoded.file);
                            let id = texture_id(&decoded.file);
                            self.textures.register_local_image(id, slot, alpha, has_alpha);
                            TexState::Ready { id, slot }
                        }
                        None => TexState::Failed(Instant::now()),
                    }
                }
                Err(error) => {
                    log::warn!("Rookery body: texture {}: {error}", decoded.file);
                    TexState::Failed(Instant::now())
                }
            };
            self.rookery.textures.insert(decoded.file, state);
            self.rookery.generation += 1;
        }
        // what the avatars' looks need
        let mut looks: Vec<Arc<Look>> = self.rookery.local.values().cloned().collect();
        looks.extend(self.rookery.looks.values().filter_map(|(l, _)| l.clone()));
        for look in looks {
            let key = body_key(&manifest, &look);
            let wanted = match self.rookery.bodies.get(&key) {
                None => true,
                Some(BodyState::Failed(at)) => at.elapsed() > RETRY_AFTER,
                Some(BodyState::Ready(meta)) => {
                    // its geometry was let go (GPU recovery, collection)
                    let gone = self.geom_ready(&meta.key).is_none();
                    if gone {
                        self.rookery.bodies.remove(&key);
                    }
                    gone
                }
                Some(BodyState::Building) => false,
            };
            if wanted && self.rookery.building < BUILDS_AT_ONCE {
                self.rookery.start_build(&runtime, key, look.clone());
            }
            for file in build::files_of(&pack, &look).into_iter().filter(|f| f.starts_with("textures/")) {
                let wanted = match self.rookery.textures.get(&file) {
                    None => true,
                    Some(TexState::Failed(at)) => at.elapsed() > RETRY_AFTER,
                    _ => false,
                };
                if wanted && self.rookery.decoding < DECODES_AT_ONCE {
                    self.rookery.start_texture(&runtime, file);
                }
            }
        }
        // bodies nobody draws any more
        let stale: Vec<u64> = self
            .rookery
            .bodies
            .iter()
            .filter(|(key, state)| {
                matches!(state, BodyState::Ready(_)) && self.rookery.used.get(key).is_none_or(|t| t.elapsed() > UNUSED_FOR)
            })
            .map(|(key, _)| *key)
            .collect();
        for key in stale {
            if let Some(BodyState::Ready(meta)) = self.rookery.bodies.remove(&key) {
                self.skin_binds.release(&meta.skin_id);
            }
            self.rookery.used.remove(&key);
        }
        self.touch_rookery_avatars(world);
        if self.rookery.reported.is_none_or(|t| t.elapsed() > Duration::from_secs(10)) {
            self.rookery.reported = Some(Instant::now());
            let ready = self.rookery.bodies.values().filter(|b| matches!(b, BodyState::Ready(_))).count();
            let building = self.rookery.bodies.values().filter(|b| matches!(b, BodyState::Building)).count();
            let textures = self.rookery.textures.values().filter(|t| matches!(t, TexState::Ready { .. })).count();
            log::info!(
                "Rookery body: {} looks ({} local), bodies {ready} ready / {building} building, textures {textures} of {}, {} avatars drawn",
                self.rookery.looks.values().filter(|(l, _)| l.is_some()).count(),
                self.rookery.local.len(),
                self.rookery.textures.len(),
                self.rookery.shown.len()
            );
        }
    }

    /// Every avatar synced again after a change of the Rookery bodies.
    fn touch_rookery_avatars(&mut self, world: &World) {
        if self.rookery.generation == self.rookery.touched {
            return;
        }
        self.rookery.touched = self.rookery.generation;
        let avatars: Vec<usize> = world.objects.avatars().map(|(idx, _)| idx).collect();
        for idx in avatars {
            self.sets.touched(&world.objects, world.agent_id, idx, false);
        }
    }

    /// A built body into the arena: one geometry whose faces are its parts,
    /// its skin bindings, its joints by rig joint.
    fn upload_rookery_body(&mut self, renderer: &mut Renderer, pack: &Pack, key: u64, body: BuiltBody) -> BodyMeta {
        let rig = self.avatar_lib.rig.clone();
        let geom_key = GeomKey::RookeryBody { hash: key };
        let mut min = Vec3::splat(f32::MAX);
        let mut max = Vec3::splat(f32::MIN);
        let mut items = Vec::with_capacity(body.parts.len());
        let mut faces = Vec::with_capacity(body.parts.len());
        for part in body.parts {
            min = min.min(part.min);
            max = max.max(part.max);
            items.push(part.item);
            faces.push(Some(jobs::FaceGeom {
                vertices: part.vertices,
                indices: part.indices,
                skin: Some(part.skin),
            }));
        }
        if !self.geom_drawn(&geom_key) {
            self.upload_geom(renderer, geom_key, jobs::PreparedGeom::new(faces, min, max, false, None));
        }
        let mut bytes = [0u8; 16];
        bytes[..8].copy_from_slice(&key.to_le_bytes());
        bytes[8..].copy_from_slice(b"rkbody\x01\x00");
        let skin_id = Uuid::from_bytes(bytes);
        let skin = Arc::new(aurora_assets::SkinInfo {
            joint_names: pack.joints.clone(),
            inverse_bind: body.joints.iter().map(|p| Mat4::from_translation(-*p)).collect(),
            ..Default::default()
        });
        let joints = pack
            .joints
            .iter()
            .zip(&body.joints)
            .filter_map(|(name, p)| Some((*rig.names.get(name)?, *p)))
            .collect();
        BodyMeta {
            key: geom_key,
            skin_id,
            skin,
            joints,
            items,
            height: body.height,
        }
    }

    /// The GPU was lost: the body textures' slots went with it (the
    /// geometry and skin bindings are dropped by `forget_gpu` itself).
    pub(super) fn rookery_gpu_lost(&mut self) {
        let ids: Vec<Uuid> = self
            .rookery
            .textures
            .values()
            .filter_map(|t| match t {
                TexState::Ready { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        for id in ids {
            self.textures.unregister_local(&id);
        }
        self.rookery.textures.clear();
        self.rookery.bodies.clear();
        self.rookery.used.clear();
    }

    /// Draws an avatar with its Rookery body when it has one ready; false
    /// leaves it to the system body. The records follow `sync_avatar`'s.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn sync_rookery_avatar(
        &mut self,
        renderer: &mut Renderer,
        idx: usize,
        avatar: Uuid,
        record_model: [[f32; 4]; 4],
        anchor: Option<u32>,
        palette: u32,
    ) -> bool {
        let Some((meta, look)) = self.rookery.ready_body(&avatar) else {
            return false;
        };
        let Some(pack) = self.rookery.pack() else {
            return false;
        };
        let Some(geom) = self.geom_ready(&meta.key) else {
            return false;
        };
        let rig = self.avatar_lib.rig.clone();
        let binds = self.skin_binds.mesh(meta.skin_id, &meta.skin, &rig);
        // everything the records depend on: rebuilt when it changes
        let materials: Vec<build::PartMaterial> = meta.items.iter().map(|item| build::material_of(&pack, &look, item)).collect();
        let slots: Vec<Option<(Uuid, u32)>> = materials
            .iter()
            .map(|m| m.texture.as_deref().and_then(|t| self.rookery.texture_slot(t)))
            .collect();
        let signature = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            meta.key.hash(&mut h);
            binds.hash(&mut h);
            palette.hash(&mut h);
            for (m, s) in materials.iter().zip(&slots) {
                m.color.map(f32::to_bits).hash(&mut h);
                (m.blend, m.mask, m.cutoff.to_bits(), m.two_sided).hash(&mut h);
                s.map(|(_, slot)| slot).hash(&mut h);
            }
            h.finish() | 1
        };
        if self.gpu[idx].rookery == signature && self.gpu[idx].is_avatar {
            for f in &self.gpu[idx].faces {
                renderer.records.set_placement(f.record, record_model, anchor);
            }
            return true;
        }
        let mut g = std::mem::take(&mut self.gpu[idx]);
        self.drop_faces(renderer, idx, &mut g);
        let old_tex: Vec<Uuid> = std::mem::take(&mut g.tex_ids);
        let held = std::mem::take(&mut g.morph_geoms);
        g.morph_pending = false;
        self.geom_ref(meta.key);
        g.morph_geoms.push(meta.key);
        for (fi, alloc) in geom.faces.iter().enumerate() {
            let Some(alloc) = alloc else {
                continue;
            };
            let (Some(item), Some(material)) = (meta.items.get(fi), materials.get(fi)) else {
                continue;
            };
            let proxy = pack.content.proxy(item);
            let kind = proxy.map_or("body", |p| p.kind.as_str());
            let slot = slots.get(fi).copied().flatten();
            // a masked part is cut out at its own cutoff, every other part
            // at the avatar's minimum alpha, as before
            let cutoff = if material.mask { material.cutoff } else { AVATAR_MINIMUM_ALPHA };
            let mut rec = DrawRecord {
                model: record_model,
                params: [0.0, 0.0, 0.8, cutoff],
                flags: [flags::SKINNED, palette, binds, 0],
                base_color: [material.color[0], material.color[1], material.color[2], 1.0],
                ..Default::default()
            };
            if material.two_sided {
                rec.flags[0] |= flags::DOUBLE_SIDED;
            }
            let base_slot = slot.map_or(aurora_render::textures::WHITE, |(_, s)| s);
            rec.tex[0] = base_slot;
            rec.set_anchor(anchor);
            let record = renderer.records.alloc(rec);
            let pbr_alpha = match kind {
                "body" => None,
                _ if material.mask => Some(2),
                _ if material.blend => Some(1),
                _ => Some(0),
            };
            g.faces.push(FaceDraw {
                te_index: fi as u8,
                record,
                cmd: DrawCmd {
                    index_count: alloc.index_count,
                    first_index: alloc.index_offset,
                    base_vertex: alloc.vertex_offset as i32,
                    record,
                    bounds: [0.0; 4],
                },
                base_slot,
                tex_id: Uuid::nil(),
                aux_tex: [Uuid::nil(); 3],
                bump_height_source: Uuid::nil(),
                te_alpha: 1.0,
                pbr_alpha,
                legacy_alpha: None,
                legacy_glare: false,
                two_sided: material.two_sided,
                repeats: 1.0,
                glow: false,
                pass: Pass::Hidden,
                glow_pool: false,
                rez_glow: false,
                system: (kind == "body").then_some(SystemPart::Body),
                first_person_hidden: matches!(kind, "eyes" | "eyebrows" | "eyelashes" | "hair")
                    || proxy.is_some_and(|p| p.category == "Hats"),
            });
        }
        for t in old_tex {
            self.textures.release(&t);
        }
        for k in held {
            self.geom_unref(k);
        }
        g.is_avatar = true;
        g.rookery = signature;
        self.finish_faces(renderer, idx, &mut g, true);
        self.gpu[idx] = g;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_skeleton_stands_on_the_body_s_joints() {
        let lib = AvatarLibrary::load();
        let rig = &lib.rig;
        let mut local = rig.local_pos.clone();
        let mut scale = rig.scale.clone();
        let pelvis = rig.pelvis;
        let torso = *rig.names.get("mTorso").expect("torso");
        let hip = *rig.names.get("mHipLeft").expect("hip");
        let knee = *rig.names.get("mKneeLeft").expect("knee");
        let joints = vec![
            (pelvis, Vec3::new(0.0, 0.0, 0.8249)),
            (torso, Vec3::new(-0.01, 0.0, 0.93)),
            (hip, Vec3::new(0.0, 0.09, 0.80)),
        ];
        apply_joints(rig, &joints, &mut local, &mut scale);
        let world = rig.world_with(&vec![Quat::IDENTITY; rig.len()], Vec3::ZERO, &local, &scale);
        for (j, p) in &joints {
            let got = world[*j].w_axis.truncate();
            assert!(got.distance(*p) < 1e-4, "joint {j}: {got:?} != {p:?}");
        }
        // a joint the body leaves keeps its offset from its parent
        assert!((local[knee] - rig.local_pos[knee]).length() < 1e-6);
        let knee_world = world[knee].w_axis.truncate();
        assert!(knee_world.distance(Vec3::new(0.0, 0.09, 0.80) + rig.local_pos[knee]) < 1e-4);
        // its scales are the skeleton's own
        assert_eq!(scale, rig.scale);
    }

    #[test]
    fn texture_ids_are_stable_and_distinct() {
        assert_eq!(texture_id("textures/a.jpg"), texture_id("textures/a.jpg"));
        assert_ne!(texture_id("textures/a.jpg"), texture_id("textures/b.jpg"));
    }
}
