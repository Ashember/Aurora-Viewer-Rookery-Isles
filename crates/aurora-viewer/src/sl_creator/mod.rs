//! The SL Character Creator's brain (Rookery Isles Viewer, 2026-11-14).
//!
//! A Rookery Isles character is a standard Second Life avatar: its sliders
//! are the visual params of the worn system Shape (and of the worn Skin and
//! Eyes for their colours), its body a starter mesh body (Ruth2 for a
//! female character, Roth2 for a male one, Bakes on Mesh) worn with skins,
//! eyes, hair and clothes from the shared Library's "Rookery Starter"
//! folder, which the grid lists at GET /rookery/starter (`catalogue`).
//!
//! Everything the creator decides lives here: the drafts of the worn shape,
//! skin and eyes, undo and redo of their slider values, the queue of wears
//! (the outfit takes one change at a time: `Creator::next_job` gives the
//! next one only when nothing is under way), the chosen body (switching
//! flips the shape's male param and swaps the starter body), and the save
//! stages (the shape, skin and eyes as copies named after the character,
//! the outfit folder, then the character's look marker on the grid). The
//! window (`ui::sl_creator`) and the app (`app/sl_creator.rs`) only carry
//! what it asks for: they send its commands and hand it the answers.
//!
//! The MakeHuman creator (`ui::character_creator`) stays in place behind
//! `Settings::rookery_makehuman_body`.

pub mod catalogue;
#[cfg(test)]
mod tests;

use crate::shape_editor::Draft;
use crate::world::World;
use aurora_assets::wearable::Wearable;
use aurora_net::{NetCommand, inventory::InvItem};
use catalogue::{Answer, Catalogue, Item, Sex};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};
use uuid::Uuid;

/// The look marker's `kind`: a character made in this creator.
pub const MARKER_KIND: &str = "sl";
/// The look marker's version.
pub const MARKER_VERSION: u64 = 1;
/// The shape's male param (0 female, 1 male).
pub const PARAM_MALE: i32 = 80;
/// The shape params that make one side of the face differ from the other
/// (Crooked Nose, Face Shear, Shift Mouth, Eye Pop): Symmetry keeps them
/// centred. A Second Life shape has no left and right pairs of its own.
pub const ASYMMETRY: [i32; 4] = [656, 662, 663, 664];
/// Steps kept for undo.
const UNDO_STEPS: usize = 100;
/// How long the outfit save may take to show in the current outfit.
const OUTFIT_WAIT: Duration = Duration::from_secs(15);

/// The worn system wearables the creator edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    Shape,
    Skin,
    Eyes,
}

impl Part {
    pub const ALL: [Part; 3] = [Part::Shape, Part::Skin, Part::Eyes];

    /// Its wearable type (the inventory flags of a body part).
    pub fn kind(self) -> u8 {
        match self {
            Part::Shape => 0,
            Part::Skin => 1,
            Part::Eyes => 3,
        }
    }

    fn index(self) -> usize {
        match self {
            Part::Shape => 0,
            Part::Skin => 1,
            Part::Eyes => 2,
        }
    }

    /// The name its saved copy takes for a character.
    pub fn copy_name(self, character: &str) -> String {
        let name = character.trim();
        match self {
            Part::Shape => aurora_i18n::tr!("slcreator-item-shape", name = name),
            Part::Skin => aurora_i18n::tr!("slcreator-item-skin", name = name),
            Part::Eyes => aurora_i18n::tr!("slcreator-item-eyes", name = name),
        }
    }
}

/// Where a slider sits in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// One of the shape editor's nine groups (Body, Head, Eyes, Ears, Nose,
    /// Mouth, Chin, Torso, Legs), as Firestorm orders them.
    Shape(usize),
    /// The skin's colour.
    Skin,
    /// The eyes' colour.
    Eyes,
}

/// One slider: a visual param of a worn part.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slider {
    pub part: Part,
    pub id: i32,
    pub group: Group,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    /// 1 female only, 2 male only, 3 both.
    pub sex: u8,
    pub order: f32,
}

impl Slider {
    /// (label, low end, high end).
    pub fn labels(&self) -> (&'static str, &'static str, &'static str) {
        match self.part {
            Part::Shape => crate::shape_editor::catalog::PARAMS
                .iter()
                .find(|p| p.id == self.id)
                .map_or((aurora_i18n::tr!("common-body"), aurora_i18n::tr!("shape-less"), aurora_i18n::tr!("shape-more")), |p| {
                    p.labels()
                }),
            Part::Skin | Part::Eyes => (
                crate::wearable_editor::catalog::PARAMS
                    .iter()
                    .find(|p| p.id == self.id)
                    .map_or(aurora_i18n::tr!("common-body"), |p| p.label()),
                aurora_i18n::tr!("shape-less"),
                aurora_i18n::tr!("shape-more"),
            ),
        }
    }

    /// Shown on a body of this sex.
    pub fn fits(&self, sex: Sex) -> bool {
        match sex {
            Sex::Female => self.sex & 1 != 0,
            Sex::Male => self.sex & 2 != 0,
            Sex::Any => true,
        }
    }

    /// Held at its default while Symmetry is on.
    pub fn asymmetric(&self) -> bool {
        self.part == Part::Shape && ASYMMETRY.contains(&self.id)
    }
}

/// Every slider of the creator: the shape's (in Firestorm's groups and
/// order), then the skin's colour and the eyes'.
pub fn sliders() -> &'static [Slider] {
    static SLIDERS: std::sync::OnceLock<Vec<Slider>> = std::sync::OnceLock::new();
    SLIDERS.get_or_init(|| {
        let mut out: Vec<Slider> = crate::shape_editor::catalog::PARAMS
            .iter()
            .map(|p| Slider {
                part: Part::Shape,
                id: p.id,
                group: Group::Shape(p.group),
                min: p.min,
                max: p.max,
                default: p.default,
                sex: p.sex,
                order: p.order,
            })
            .collect();
        out.sort_by(|a, b| {
            let group = |s: &Slider| if let Group::Shape(g) = s.group { g } else { 9 };
            group(a).cmp(&group(b)).then(a.order.total_cmp(&b.order)).then(a.id.cmp(&b.id))
        });
        let mut extra: Vec<Slider> = crate::wearable_editor::catalog::PARAMS
            .iter()
            .filter_map(|p| {
                let (part, group) = match (p.kind, p.group) {
                    (1, "skin_color") => (Part::Skin, Group::Skin),
                    (3, _) => (Part::Eyes, Group::Eyes),
                    _ => return None,
                };
                Some(Slider {
                    part,
                    id: p.id,
                    group,
                    min: p.min,
                    max: p.max,
                    default: p.default,
                    sex: p.sex,
                    order: p.order,
                })
            })
            .collect();
        extra.sort_by(|a, b| a.part.index().cmp(&b.part.index()).then(a.order.total_cmp(&b.order)).then(a.id.cmp(&b.id)));
        out.extend(extra);
        out
    })
}

/// The look a character made here saves on the grid:
/// {"kind":"sl","version":1,"outfit":"<outfit folder id>","shape":"<shape item id>"}.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlLook {
    /// Its outfit folder (in My Outfits, named after the character).
    pub outfit: Uuid,
    /// Its saved shape item.
    pub shape: Uuid,
}

impl SlLook {
    pub fn to_value(&self) -> Value {
        serde_json::json!({
            "kind": MARKER_KIND,
            "version": MARKER_VERSION,
            "outfit": self.outfit.to_string(),
            "shape": self.shape.to_string(),
        })
    }

    /// A saved look read as one made here: None for a MakeHuman look, one
    /// of a newer version this viewer does not know, or one without an
    /// outfit.
    pub fn read(value: &Value) -> Option<SlLook> {
        if value.get("kind").and_then(Value::as_str) != Some(MARKER_KIND) {
            return None;
        }
        let version = value.get("version").and_then(Value::as_u64)?;
        if version == 0 || version > MARKER_VERSION {
            return None;
        }
        let id = |key: &str| value.get(key).and_then(Value::as_str).and_then(|s| Uuid::parse_str(s.trim()).ok());
        let outfit = id("outfit").filter(|id| !id.is_nil())?;
        Some(SlLook {
            outfit,
            shape: id("shape").unwrap_or_default(),
        })
    }
}

/// The outfit to wear when a character goes into the world: for a Play,
/// its own outfit folder when it was made here (None for a MakeHuman look,
/// one not made yet, or when the MakeHuman body is in use); any other
/// request keeps `current` (a wait asking again is still that Play).
pub fn play_outfit(
    roster: Option<&crate::characters::Roster>,
    action: &crate::characters::Action,
    makehuman: bool,
    current: Option<Uuid>,
) -> Option<Uuid> {
    let crate::characters::Action::Play { id } = action else {
        return current;
    };
    if makehuman {
        return None;
    }
    roster?.characters.iter().find(|c| &c.id == id)?.saved_sl_look().map(|look| look.outfit)
}

/// The fetch that loads a played character's outfit folder before it is
/// worn (None when it is loaded already).
pub fn outfit_fetch(world: &World, outfit: Uuid) -> Option<NetCommand> {
    let loaded = world
        .inventory
        .folders
        .get(&outfit)
        .is_some_and(|f| f.state == crate::world::inventory::FetchState::Fetched);
    (!loaded).then(|| NetCommand::FetchInventory {
        folders: vec![outfit],
        owner: world.agent_id,
        library: false,
    })
}

/// A character's look as the creator opens on it: (its look made here,
/// whether it has any look at all).
pub fn saved_of(roster: Option<&crate::characters::Roster>, id: &str) -> (Option<SlLook>, bool) {
    roster
        .and_then(|r| r.characters.iter().find(|c| c.id == id))
        .map_or((None, false), |c| (c.saved_sl_look(), c.has_look()))
}

/// The sliders the offline demo's `shaped` mode moves (taller, broader,
/// heavier, a bigger head, shorter legs): plainly another body.
pub const DEMO_SHAPED: [(i32, f32); 6] = [(33, 1.6), (34, 1.4), (36, 1.4), (637, 1.0), (682, 1.0), (692, -0.8)];

/// The wear of a played character's outfit, as it goes into the world.
pub fn wear_outfit(outfit: Uuid) -> crate::world::appearance::Action {
    crate::world::appearance::Action::WearOutfits {
        folders: vec![outfit],
        append: false,
    }
}

/// The slider values of the three drafts (shape, skin, eyes): one undo step.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Edits(pub [BTreeMap<i32, f32>; 3]);

/// One change of the outfit, waiting its turn.
#[derive(Debug, Clone, PartialEq)]
pub enum Job {
    /// Wear an item: from the Library through a copy first, an own item at
    /// once. `catalogue`: the catalogue item it stands for.
    Wear { item: Uuid, replace: bool, catalogue: Option<Uuid> },
    /// Take a worn item off.
    Remove { item: Uuid },
}

/// What the creator asks of the app.
#[derive(Debug, Clone)]
pub enum Step {
    Send(NetCommand),
    Appearance(crate::world::appearance::Action),
    /// Save this look on the character (`characters::Action::SaveLook`).
    SaveLook(Value),
    /// Saved: the creator closes.
    Done,
}

/// The save under way.
#[derive(Debug, Clone, PartialEq)]
enum Stage {
    /// Save this part (index into `Part::ALL`), or go on past it.
    Wearable(usize),
    AwaitWearable { part: usize, request: Uuid },
    /// Wait for the wears to finish.
    Settle,
    /// Save the outfit folder; `fresh`: a new one even when the character has one.
    Outfit { fresh: bool },
    AwaitOutfit { since: Instant, existing: Option<Uuid> },
    Look(SlLook),
    AwaitLook(SlLook),
}

/// The starter catalogue, as far as it came.
pub enum CatalogueState {
    Loading(crossbeam_channel::Receiver<Result<Answer, String>>),
    Ready(Catalogue),
    /// The grid has no starter catalogue (or this is not the Rookery grid).
    Missing,
    Failed(String),
}

/// What the window shows (kept with the creator so it survives frames).
#[derive(Debug, Clone, Default)]
pub struct View {
    /// The content browser's shelf (a category id).
    pub shelf: Option<String>,
    /// Narrows the content browser's tiles by name.
    pub content_search: String,
    /// Narrows the sliders by name.
    pub search: String,
    /// Asking whether to leave without saving.
    pub confirm_leave: bool,
}

/// A tiny random source for Random (no randomness crate in the viewer).
struct Dice(u64);

impl Dice {
    fn new(seed: u64) -> Self {
        Dice(seed | 1)
    }

    fn next(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 40) as f32 / (1u64 << 24) as f32
    }
}

pub struct Creator {
    pub character_id: String,
    pub name: String,
    /// Its saved look made here (None: not made here yet).
    pub saved: Option<SlLook>,
    /// No look at all yet: Save is the only way on, there is no Cancel.
    pub mandatory: bool,
    drafts: [Option<Draft>; 3],
    loading: [Option<(Uuid, Uuid)>; 3],
    /// Worn parts whose asset could not be read (not asked for again).
    failed: [Option<(Uuid, Uuid)>; 3],
    undo: Vec<Edits>,
    redo: Vec<Edits>,
    /// A change under way (a slider dragged): one undo step for all of it.
    editing: bool,
    /// The values at the start of this frame (`begin_frame`).
    before: Option<Edits>,
    /// The values last drawn on the avatar.
    previewed: Option<Edits>,
    pub symmetric: bool,
    /// The body worn: female (Ruth2) or male (Roth2).
    pub sex: Sex,
    queue: VecDeque<Job>,
    /// Library copies under way: request to (catalogue item, replace).
    copies: HashMap<Uuid, (Uuid, bool)>,
    /// The copy worn for a catalogue item, this session.
    worn_copies: HashMap<Uuid, Uuid>,
    /// The item chosen on each shelf.
    pub chosen: HashMap<String, Uuid>,
    pub catalogue: CatalogueState,
    /// A new character's body and defaults put on once.
    defaults_worn: bool,
    stage: Option<Stage>,
    /// A wear or a remove was sent (Cancel puts the saved outfit back).
    touched: bool,
    pub message: String,
    dice: Dice,
    pub view: View,
    /// Thumbnails the window asked for (the app loads them).
    pub wanted_images: HashSet<Uuid>,
    /// The grid's answer to SaveLook, under way.
    pub saving_look: Option<crossbeam_channel::Receiver<Result<crate::characters::Roster, String>>>,
    /// The windows open before the creator (closed while it is open).
    pub saved_panels: Option<crate::ui::Panels>,
}

/// The worn item of a part (from the current outfit).
pub fn worn_part(world: &World, part: Part) -> Option<InvItem> {
    if part == Part::Shape {
        return crate::shape_editor::worn_shape(world).and_then(|id| world.inventory.items.get(&id).cloned());
    }
    let cof = crate::world::appearance::cof(&world.inventory)?;
    crate::world::appearance::folder_links(&world.inventory, cof)
        .into_iter()
        .filter(|l| !l.folder)
        .filter_map(|l| world.inventory.items.get(&l.target))
        .find(|it| it.asset_type == 13 && it.flags & 0xff == u32::from(part.kind()))
        .cloned()
}

/// Whether an item is linked from the current outfit.
fn worn(world: &World, item: Uuid) -> bool {
    crate::world::appearance::cof(&world.inventory).is_some_and(|cof| {
        crate::world::appearance::folder_links(&world.inventory, cof)
            .iter()
            .any(|l| !l.folder && l.target == item)
    })
}

impl Creator {
    /// The creator on a character: `saved`, its look made here; `has_look`,
    /// whether it has any look at all (one made in the MakeHuman creator
    /// counts: it is not a new character).
    pub fn new(character_id: String, name: String, saved: Option<SlLook>, has_look: bool) -> Creator {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9_7f4a_7c15, |d| d.as_nanos() as u64);
        Creator {
            character_id,
            name,
            saved,
            mandatory: saved.is_none() && !has_look,
            drafts: [None, None, None],
            loading: [None; 3],
            failed: [None; 3],
            undo: Vec::new(),
            redo: Vec::new(),
            editing: false,
            before: None,
            previewed: None,
            symmetric: true,
            sex: Sex::Female,
            queue: VecDeque::new(),
            copies: HashMap::new(),
            worn_copies: HashMap::new(),
            chosen: HashMap::new(),
            catalogue: CatalogueState::Missing,
            defaults_worn: false,
            stage: None,
            touched: false,
            message: String::new(),
            dice: Dice::new(seed),
            view: View::default(),
            wanted_images: HashSet::new(),
            saving_look: None,
            saved_panels: None,
        }
    }

    // ---- the catalogue ------------------------------------------------

    /// Asks the grid for the starter catalogue (None: this grid has none).
    pub fn fetch_catalogue(&mut self, endpoint: Option<String>, runtime: &tokio::runtime::Handle, ctx: &egui::Context) {
        let Some(endpoint) = endpoint else {
            self.catalogue = CatalogueState::Missing;
            return;
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        let repaint = ctx.clone();
        runtime.spawn(async move {
            let _ = tx.send(catalogue::fetch(endpoint).await);
            repaint.request_repaint();
        });
        self.catalogue = CatalogueState::Loading(rx);
    }

    /// The catalogue given directly (the offline demo's).
    pub fn set_catalogue(&mut self, catalogue: Catalogue) {
        self.catalogue = CatalogueState::Ready(catalogue);
    }

    fn poll_catalogue(&mut self) {
        let CatalogueState::Loading(rx) = &self.catalogue else {
            return;
        };
        self.catalogue = match rx.try_recv() {
            Ok(Ok(Answer::Ready(catalogue))) => {
                log::info!("SL creator: starter catalogue, {} items on {} shelves", catalogue.items.len(), catalogue.categories.len());
                CatalogueState::Ready(catalogue)
            }
            Ok(Ok(Answer::Missing)) => {
                log::info!("SL creator: the grid has no starter catalogue");
                CatalogueState::Missing
            }
            Ok(Err(message)) => {
                log::warn!("SL creator: starter catalogue: {message}");
                CatalogueState::Failed(message)
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => CatalogueState::Failed(aurora_i18n::tr!("characters-error-network").to_owned()),
            Err(crossbeam_channel::TryRecvError::Empty) => return,
        };
    }

    pub fn catalogue(&self) -> Option<&Catalogue> {
        match &self.catalogue {
            CatalogueState::Ready(catalogue) => Some(catalogue),
            _ => None,
        }
    }

    // ---- the drafts ---------------------------------------------------

    pub fn draft(&self, part: Part) -> Option<&Draft> {
        self.drafts[part.index()].as_ref()
    }

    /// Whether a part's draft is still on its way.
    pub fn loading(&self, part: Part) -> bool {
        self.loading[part.index()].is_some()
    }

    /// Each frame: the catalogue's answer, and the worn shape, skin and
    /// eyes read again when they changed (a tile worn, the save's copy).
    /// `busy`: an outfit change is under way. The commands go to the grid.
    pub fn poll(&mut self, world: &World, busy: bool) -> Vec<NetCommand> {
        self.poll_catalogue();
        let mut out = Vec::new();
        // a draft follows what is worn only while nothing is changing it
        let settled = !busy && self.stage.is_none() && self.queue.is_empty() && self.copies.is_empty();
        if settled {
            for part in Part::ALL {
                let i = part.index();
                let current = worn_part(world, part);
                let key = current.as_ref().map(|it| (it.id, it.asset_id));
                let have = self.drafts[i].as_ref().map(|d| (d.item.id, d.item.asset_id));
                if have == key || self.loading[i].is_some_and(|l| Some(l) == key) || self.failed[i].is_some_and(|f| Some(f) == key) {
                    continue;
                }
                match current {
                    Some(item) => {
                        self.loading[i] = Some((item.id, item.asset_id));
                        out.push(NetCommand::PreviewInventoryItem(item));
                    }
                    None => {
                        self.drafts[i] = None;
                        self.loading[i] = None;
                    }
                }
            }
        }
        // a new character puts on its body (and the defaults) once the
        // catalogue and its shape are known
        let ready = self.mandatory
            && !self.defaults_worn
            && settled
            && self.loading[0].is_none()
            && (self.drafts[0].is_some() || self.failed[0].is_some() || worn_part(world, Part::Shape).is_none());
        let defaults: Option<Vec<(Uuid, String, bool)>> = self.catalogue().filter(|_| ready).map(|catalogue| {
            catalogue
                .defaults(self.sex)
                .into_iter()
                .map(|item| {
                    let body = catalogue.category(&item.category).is_some_and(|c| c.body);
                    (item.id, item.category.clone(), !body && catalogue.replaces(item))
                })
                .collect()
        });
        if let Some(defaults) = defaults {
            self.defaults_worn = true;
            for (item, category, replace) in defaults {
                self.chosen.insert(category, item);
                self.queue.push_back(Job::Wear {
                    item,
                    replace,
                    catalogue: Some(item),
                });
            }
        }
        out
    }

    /// The asset of a worn part, read. True when it was one of ours.
    pub fn loaded(&mut self, world: &World, item: Uuid, asset: Uuid, result: &Result<Vec<u8>, String>) -> bool {
        let Some(part) = Part::ALL.into_iter().find(|p| self.loading[p.index()] == Some((item, asset))) else {
            return false;
        };
        let i = part.index();
        self.loading[i] = None;
        let Some(info) = worn_part(world, part).filter(|it| it.id == item && it.asset_id == asset) else {
            // changed meanwhile: the next poll asks for the new one
            return true;
        };
        match result
            .as_ref()
            .map_err(Clone::clone)
            .and_then(|bytes| Wearable::parse(bytes).map_err(|_| aurora_i18n::tr!("shape-error").to_owned()))
        {
            Ok(mut original) if original.kind == part.kind() => {
                for slider in sliders().iter().filter(|s| s.part == part) {
                    original.params.entry(slider.id).or_insert(slider.default);
                }
                if part == Part::Shape {
                    original.params.entry(PARAM_MALE).or_insert(0.0);
                    self.sex = if original.params[&PARAM_MALE] > 0.5 { Sex::Male } else { Sex::Female };
                }
                self.drafts[i] = Some(Draft {
                    item: info,
                    edited: original.clone(),
                    original,
                    pending: None,
                });
                self.failed[i] = None;
                // the avatar shows the values read
                self.previewed = None;
            }
            Ok(_) => {
                self.failed[i] = Some((item, asset));
                self.message = aurora_i18n::tr!("shape-error").to_owned();
            }
            Err(reason) => {
                self.failed[i] = Some((item, asset));
                log::warn!("SL creator: the worn {part:?} could not be read: {reason}");
                self.message = reason;
            }
        }
        true
    }

    // ---- slider values ------------------------------------------------

    pub fn value(&self, slider: &Slider) -> f32 {
        self.drafts[slider.part.index()]
            .as_ref()
            .and_then(|d| d.edited.params.get(&slider.id).copied())
            .unwrap_or(slider.default)
            .clamp(slider.min, slider.max)
    }

    /// Sets a slider (Symmetry holds the asymmetric ones at their default).
    pub fn set(&mut self, slider: &Slider, value: f32) {
        if !value.is_finite() || (self.symmetric && slider.asymmetric()) {
            return;
        }
        if let Some(draft) = &mut self.drafts[slider.part.index()] {
            draft.edited.params.insert(slider.id, value.clamp(slider.min, slider.max));
        }
    }

    /// The slider values now.
    pub fn edits(&self) -> Edits {
        Edits(std::array::from_fn(|i| self.drafts[i].as_ref().map(|d| d.edited.params.clone()).unwrap_or_default()))
    }

    fn restore(&mut self, edits: &Edits) {
        for (i, values) in edits.0.iter().enumerate() {
            if let Some(draft) = &mut self.drafts[i] {
                for (id, value) in values {
                    draft.edited.params.insert(*id, *value);
                }
            }
        }
    }

    /// The values as one change for undo (Random, Reset, a body switch).
    fn remember(&mut self, before: Edits) {
        if Some(&before) != self.undo.last() {
            self.undo.push(before);
            if self.undo.len() > UNDO_STEPS {
                self.undo.remove(0);
            }
        }
        self.redo.clear();
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// The window starts drawing: changes made until `end_frame` are one step.
    pub fn begin_frame(&mut self) {
        self.before = Some(self.edits());
    }

    /// The window drew: a change becomes an undo step (a whole drag being
    /// one), and the avatar shows the values.
    pub fn end_frame(&mut self, world: &mut World, dragging: bool) {
        let now = self.edits();
        match self.before.take() {
            Some(before) if before != now => {
                if !self.editing {
                    self.remember(before);
                }
                self.editing = dragging;
            }
            _ => {
                if !dragging {
                    self.editing = false;
                }
            }
        }
        self.preview(world, &now);
    }

    /// Several values set at once as one undo step (the offline demo).
    pub fn set_many(&mut self, world: &mut World, values: &[(Part, i32, f32)]) {
        let before = self.edits();
        for (part, id, value) in values {
            if let Some(slider) = sliders().iter().find(|s| s.part == *part && s.id == *id) {
                self.set(slider, *value);
            }
        }
        if self.edits() != before {
            self.remember(before);
        }
        let now = self.edits();
        self.preview(world, &now);
    }

    /// The avatar drawn with the drafts' values, when they changed.
    fn preview(&mut self, world: &mut World, edits: &Edits) {
        if self.previewed.as_ref() == Some(edits) {
            return;
        }
        self.previewed = Some(edits.clone());
        if self.drafts.iter().all(Option::is_none) {
            world.preview_shape(None);
            return;
        }
        let values = world
            .avatar_lib
            .shape_params
            .edit_values(world.server_visual_params(), &self.preview_params(edits));
        world.preview_shape(Some(values));
    }

    /// The params the avatar is drawn with: the whole shape draft, and only
    /// the skin's and eyes' own sliders (their other params stay as worn).
    fn preview_params(&self, edits: &Edits) -> BTreeMap<i32, f32> {
        let mut params = edits.0[0].clone();
        for part in [Part::Skin, Part::Eyes] {
            for slider in sliders().iter().filter(|s| s.part == part) {
                if let Some(value) = edits.0[part.index()].get(&slider.id) {
                    params.insert(slider.id, *value);
                }
            }
        }
        params
    }

    pub fn undo(&mut self) {
        let Some(previous) = self.undo.pop() else {
            return;
        };
        self.redo.push(self.edits());
        self.restore(&previous);
        self.follow_sex();
        self.before = Some(self.edits());
    }

    pub fn redo(&mut self) {
        let Some(next) = self.redo.pop() else {
            return;
        };
        self.undo.push(self.edits());
        self.restore(&next);
        self.follow_sex();
        self.before = Some(self.edits());
    }

    /// The body follows the shape's male param after undo or redo.
    fn follow_sex(&mut self) {
        let Some(male) = self.drafts[0].as_ref().and_then(|d| d.edited.params.get(&PARAM_MALE).copied()) else {
            return;
        };
        let sex = if male > 0.5 { Sex::Male } else { Sex::Female };
        if sex != self.sex {
            let old = self.sex;
            self.sex = sex;
            self.swap_body(old, sex);
        }
    }

    /// Back to the values as worn when the creator opened (or last saved).
    pub fn reset(&mut self) {
        let before = self.edits();
        for draft in self.drafts.iter_mut().flatten() {
            draft.edited.params.clone_from(&draft.original.params);
        }
        if self.edits() != before {
            self.remember(before);
        }
        self.follow_sex();
        self.before = Some(self.edits());
    }

    /// A random shape: every slider of the body worn moved a bounded way
    /// from its default (the face a little more than the body), Symmetry
    /// and the sex kept.
    pub fn randomize(&mut self) {
        if self.drafts[0].is_none() {
            return;
        }
        let before = self.edits();
        let sex = self.sex;
        let symmetric = self.symmetric;
        for slider in sliders().iter().filter(|s| s.part == Part::Shape && s.fits(sex)) {
            // the hover is a placement, not a look
            if slider.id == 11001 || (symmetric && slider.asymmetric()) {
                continue;
            }
            let spread = match slider.group {
                Group::Shape(1..=6) => 0.18,
                _ => 0.10,
            };
            let span = slider.max - slider.min;
            let value = slider.default + span * spread * (self.dice.next() * 2.0 - 1.0);
            if let Some(draft) = &mut self.drafts[0] {
                draft.edited.params.insert(slider.id, value.clamp(slider.min, slider.max));
            }
        }
        if self.edits() != before {
            self.remember(before);
        }
        self.before = Some(self.edits());
    }

    /// Symmetry on centres the asymmetric sliders (one undo step).
    pub fn set_symmetric(&mut self, on: bool) {
        self.symmetric = on;
        if !on {
            return;
        }
        let before = self.edits();
        if let Some(draft) = &mut self.drafts[0] {
            for slider in sliders().iter().filter(|s| s.asymmetric()) {
                draft.edited.params.insert(slider.id, slider.default);
            }
        }
        if self.edits() != before {
            self.remember(before);
        }
        self.before = Some(self.edits());
    }

    // ---- the body -----------------------------------------------------

    /// Switches the body: the shape's male param flips (one undo step) and
    /// the starter body of the other sex replaces the worn one.
    pub fn set_sex(&mut self, sex: Sex) {
        if sex == Sex::Any || sex == self.sex {
            return;
        }
        let before = self.edits();
        if let Some(draft) = &mut self.drafts[0] {
            draft.edited.params.insert(PARAM_MALE, if sex == Sex::Male { 1.0 } else { 0.0 });
        }
        if self.edits() != before {
            self.remember(before);
        }
        let old = self.sex;
        self.sex = sex;
        self.swap_body(old, sex);
        self.before = Some(self.edits());
    }

    /// The starter set of `old` taken off, that of `new` put on: its body
    /// pieces (added, so what else is worn at their points, hair on the
    /// skull, stays), and its other defaults (skin, eyes, alpha, underwear,
    /// hair) worn in place of the old body's. The shape is never swapped:
    /// it holds the sliders being edited, and the male param already
    /// followed the switch. Body parts are never taken off (a body part can
    /// only be replaced); the new body's own replace them. 2026-11-14: was
    /// the body shelf's items only, so the old body's skin, mesh eyes and
    /// underwear stayed on the new body.
    fn swap_body(&mut self, old: Sex, new: Sex) {
        let Some(catalogue) = self.catalogue() else {
            return;
        };
        let is_shape = |i: &Item| i.wearable_type == Some(0);
        let leaving = catalogue.defaults(old);
        let arriving = catalogue.defaults(new);
        let off: Vec<Uuid> = leaving
            .iter()
            .filter(|i| !is_shape(i) && i.asset_type != 13 && !arriving.iter().any(|a| a.id == i.id))
            .map(|i| i.id)
            .collect();
        let on: Vec<(Uuid, String, bool)> = arriving
            .iter()
            .filter(|i| !is_shape(i) && !leaving.iter().any(|l| l.id == i.id))
            .map(|i| {
                let body = catalogue.category(&i.category).is_some_and(|c| c.body);
                (i.id, i.category.clone(), !body && catalogue.replaces(i))
            })
            .collect();
        for item in off {
            self.queue.push_back(Job::Remove { item });
        }
        for (item, category, replace) in on {
            self.chosen.insert(category, item);
            self.queue.push_back(Job::Wear {
                item,
                replace,
                catalogue: Some(item),
            });
        }
        // The body shelf's items only, until 2026-11-14:
        // let off: Vec<Uuid> = catalogue.body_items(old).iter().map(|i| i.id).collect();
        // let on: Vec<(Uuid, String)> = catalogue.body_items(new).iter().map(|i| (i.id, i.category.clone())).collect();
        // for item in off {
        //     self.queue.push_back(Job::Remove { item });
        // }
        // for (item, category) in on {
        //     self.chosen.insert(category, item);
        //     self.queue.push_back(Job::Wear { item, replace: false, catalogue: Some(item) });
        // }
    }

    // ---- the content browser ------------------------------------------

    /// A tile clicked: wear it (a body of the other sex switches the body;
    /// a worn item of a stacking shelf comes off again).
    pub fn choose(&mut self, item: Uuid) {
        let Some(catalogue) = self.catalogue() else {
            return;
        };
        let Some(entry) = catalogue.item(item).cloned() else {
            return;
        };
        let body = catalogue.category(&entry.category).is_some_and(|c| c.body);
        let replace = catalogue.replaces(&entry);
        if body && entry.sex != Sex::Any && entry.sex != self.sex {
            self.set_sex(entry.sex);
            return;
        }
        if !entry.sex.fits(self.sex) {
            return;
        }
        if !replace || body {
            // a stacking item toggles; a body piece is added
            if self.chosen.get(&entry.category) == Some(&item) && !body {
                self.chosen.remove(&entry.category);
                self.queue.push_back(Job::Remove { item });
                return;
            }
        }
        self.chosen.insert(entry.category.clone(), item);
        self.queue.push_back(Job::Wear {
            item,
            replace: replace && !body,
            catalogue: Some(item),
        });
    }

    /// Whether a tile shows as chosen.
    pub fn is_chosen(&self, item: &Item) -> bool {
        self.chosen.get(&item.category) == Some(&item.id)
    }

    /// Wears still to come (the window shows them as under way).
    pub fn pending_wears(&self) -> usize {
        self.queue.len() + self.copies.len()
    }

    /// The copy of a catalogue item in the inventory: the one made this
    /// session, else a worn item of its name and type (from an earlier
    /// session).
    fn copy_of(&self, world: &World, item: Uuid) -> Option<Uuid> {
        let inv = &world.inventory;
        let usable = |id: &Uuid| {
            inv.items.get(id).is_some_and(|it| it.owner == world.agent_id)
                && !crate::world::inventory::actions::library(inv, *id)
                && !crate::world::inventory::actions::in_type(inv, *id, 14)
        };
        if let Some(copy) = self.worn_copies.get(&item).filter(|id| usable(id)) {
            return Some(*copy);
        }
        let entry = self.catalogue()?.item(item)?;
        let cof = crate::world::appearance::cof(inv)?;
        crate::world::appearance::folder_links(inv, cof)
            .into_iter()
            .filter(|l| !l.folder && l.target != item)
            .map(|l| l.target)
            .find(|id| {
                usable(id)
                    && inv
                        .items
                        .get(id)
                        .is_some_and(|it| it.name == entry.name && (entry.asset_type < 0 || it.asset_type == entry.asset_type))
            })
    }

    /// The next change of the outfit, when nothing is under way (`busy`:
    /// the app's own outfit change or Library copy in flight).
    pub fn next_job(&mut self, world: &World, busy: bool) -> Option<Step> {
        if busy || !self.copies.is_empty() || matches!(self.stage, Some(Stage::AwaitWearable { .. })) {
            return None;
        }
        while let Some(job) = self.queue.pop_front() {
            match job {
                Job::Remove { item } => {
                    let target = if worn(world, item) { Some(item) } else { self.copy_of(world, item) };
                    if let Some(target) = target.filter(|id| worn(world, *id)) {
                        self.touched = true;
                        return Some(Step::Appearance(crate::world::appearance::Action::Remove(target)));
                    }
                    // not worn: nothing to take off
                }
                Job::Wear { item, replace, catalogue } => {
                    let inv = &world.inventory;
                    let own = inv.items.get(&item).is_some_and(|it| it.owner == world.agent_id)
                        && !crate::world::inventory::actions::library(inv, item);
                    if own {
                        self.touched = true;
                        return Some(Step::Appearance(crate::world::appearance::Action::WearItem { item, replace, point: 0 }));
                    }
                    if let Some(copy) = catalogue.and_then(|c| self.copy_of(world, c)) {
                        self.touched = true;
                        return Some(Step::Appearance(crate::world::appearance::Action::WearItem {
                            item: copy,
                            replace,
                            point: 0,
                        }));
                    }
                    let owner = inv.items.get(&item).map(|it| it.owner).filter(|o| !o.is_nil()).unwrap_or(inv.lib_owner);
                    if owner.is_nil() {
                        self.message = aurora_i18n::tr!("slcreator-library-unknown").to_owned();
                        continue;
                    }
                    let request = Uuid::new_v4();
                    self.copies.insert(request, (catalogue.unwrap_or(item), replace));
                    self.touched = true;
                    return Some(Step::Send(NetCommand::CopyInventoryItem {
                        request,
                        copy: aurora_net::inventory::operations::CopyItem {
                            item,
                            owner,
                            parent: Uuid::nil(),
                            name: String::new(),
                        },
                    }));
                }
            }
        }
        None
    }

    pub fn owns_copy(&self, request: Uuid) -> bool {
        self.copies.contains_key(&request)
    }

    /// A Library copy arrived: it is worn next.
    pub fn copied(&mut self, world: &mut World, request: Uuid, result: Result<InvItem, String>) {
        let Some((catalogue, replace)) = self.copies.remove(&request) else {
            return;
        };
        match result {
            Ok(item) => {
                let id = item.id;
                log::info!("SL creator: starter item copied, wearing it: {id}");
                // the server's inventory update may follow this answer
                world.inventory.add_items(vec![item]);
                self.worn_copies.insert(catalogue, id);
                self.queue.push_front(Job::Wear {
                    item: id,
                    replace,
                    catalogue: Some(catalogue),
                });
            }
            Err(reason) => {
                log::warn!("SL creator: starter item {catalogue} could not be copied: {reason}");
                self.message = reason;
            }
        }
    }

    /// An outfit change the app could not make (its message).
    pub fn refused(&mut self, message: String) {
        match self.stage {
            Some(Stage::AwaitOutfit { existing: Some(_), .. }) => {
                log::warn!("SL creator: saving into the character's outfit failed ({message}), saving a new one");
                self.stage = Some(Stage::Outfit { fresh: true });
            }
            Some(Stage::AwaitOutfit { existing: None, .. }) => self.abort(message),
            _ => self.message = message,
        }
    }

    // ---- saving -------------------------------------------------------

    /// Whether a save is under way.
    pub fn saving(&self) -> bool {
        self.stage.is_some()
    }

    /// Whether anything differs from the saved look.
    pub fn changed(&self) -> bool {
        self.touched || self.drafts.iter().flatten().any(Draft::dirty)
    }

    /// Save pressed.
    pub fn start_save(&mut self) {
        if self.stage.is_none() {
            self.message.clear();
            self.stage = Some(Stage::Wearable(0));
        }
    }

    fn abort(&mut self, message: String) {
        log::warn!("SL creator: save stopped: {message}");
        self.stage = None;
        self.saving_look = None;
        self.message = message;
    }

    pub fn owns_save(&self, request: Uuid) -> bool {
        matches!(self.stage, Some(Stage::AwaitWearable { request: r, .. }) if r == request)
    }

    /// The save of a part's copy answered.
    pub fn wearable_saved(&mut self, world: &mut World, request: Uuid, result: Result<InvItem, String>) {
        let Some(Stage::AwaitWearable { part, request: r }) = self.stage else {
            return;
        };
        if r != request {
            return;
        }
        let Some(draft) = self.drafts[part].as_mut() else {
            self.stage = Some(Stage::Wearable(part + 1));
            return;
        };
        draft.pending = None;
        match result {
            Ok(item) => {
                let copied = item.id != draft.item.id;
                let id = item.id;
                draft.item = item.clone();
                draft.original = draft.edited.clone();
                world.inventory.add_items(vec![item]);
                if copied {
                    self.queue.push_back(Job::Wear {
                        item: id,
                        replace: true,
                        catalogue: None,
                    });
                }
                self.stage = Some(Stage::Wearable(part + 1));
            }
            Err(reason) => self.abort(reason),
        }
    }

    /// The save's next step (`busy`: an outfit change or copy under way).
    pub fn save_step(&mut self, world: &mut World, busy: bool) -> Option<Step> {
        let stage = self.stage.clone()?;
        match stage {
            Stage::Wearable(i) if i >= Part::ALL.len() => {
                self.stage = Some(Stage::Settle);
                None
            }
            Stage::Wearable(i) => {
                let part = Part::ALL[i];
                let target = part.copy_name(&self.name);
                let Some(draft) = self.drafts[i].as_mut() else {
                    if part == Part::Shape && worn_part(world, Part::Shape).is_some() {
                        self.abort(aurora_i18n::tr!("slcreator-shape-loading").to_owned());
                    } else {
                        self.stage = Some(Stage::Wearable(i + 1));
                    }
                    return None;
                };
                let Some(current) = world.inventory.items.get(&draft.item.id).cloned() else {
                    self.abort(aurora_i18n::tr!("shape-changed").to_owned());
                    return None;
                };
                let dirty = draft.dirty();
                let renamed = part == Part::Shape && current.name != target;
                if !dirty && !renamed {
                    self.stage = Some(Stage::Wearable(i + 1));
                    return None;
                }
                let in_place = current.name == target && draft.can_modify(world);
                let allowed = if in_place {
                    true
                } else {
                    draft.can_copy(world) && (draft.can_modify(world) || !dirty)
                };
                if !allowed {
                    if dirty {
                        self.abort(aurora_i18n::tr!("slcreator-save-denied").to_owned());
                    } else {
                        // an unchanged shape that may not be copied stays as it is
                        self.stage = Some(Stage::Wearable(i + 1));
                    }
                    return None;
                }
                let data = match draft.edited.encode(&target) {
                    Ok(data) => data,
                    Err(_) => {
                        self.abort(aurora_i18n::tr!("shape-error").to_owned());
                        return None;
                    }
                };
                let request = Uuid::new_v4();
                draft.pending = Some(request);
                self.stage = Some(Stage::AwaitWearable { part: i, request });
                Some(Step::Send(NetCommand::SaveWearable {
                    request,
                    item: Box::new(current),
                    data,
                    copy_name: (!in_place).then_some(target),
                }))
            }
            Stage::AwaitWearable { .. } => None,
            Stage::Settle => {
                if !busy && self.queue.is_empty() && self.copies.is_empty() {
                    self.stage = Some(Stage::Outfit { fresh: false });
                }
                None
            }
            Stage::Outfit { fresh } => {
                if busy {
                    return None;
                }
                let inv = &world.inventory;
                let existing = self.saved.map(|s| s.outfit).filter(|id| {
                    !fresh
                        && inv.folders.get(id).is_some_and(|f| f.info.type_default == crate::world::appearance::FT_OUTFIT && !f.library)
                        && !crate::world::inventory::actions::in_type(inv, *id, 14)
                });
                self.stage = Some(Stage::AwaitOutfit {
                    since: Instant::now(),
                    existing,
                });
                Some(Step::Appearance(match existing {
                    Some(folder) => crate::world::appearance::Action::SaveTo(folder),
                    None => crate::world::appearance::Action::Save(Some(self.name.trim().to_owned())),
                }))
            }
            Stage::AwaitOutfit { since, existing } => {
                if busy {
                    return None;
                }
                let inv = &world.inventory;
                let base = crate::world::appearance::base(inv).filter(|id| {
                    Some(*id) == existing || inv.folders.get(id).is_some_and(|f| f.info.name.trim() == self.name.trim())
                });
                if let Some(outfit) = base {
                    let shape = self.drafts[0]
                        .as_ref()
                        .map(|d| d.item.id)
                        .or_else(|| crate::shape_editor::worn_shape(world))
                        .unwrap_or_default();
                    log::info!("SL creator: {} saved as outfit {outfit}, shape {shape}", self.name);
                    self.stage = Some(Stage::Look(SlLook { outfit, shape }));
                } else if since.elapsed() >= OUTFIT_WAIT {
                    if existing.is_some() {
                        self.stage = Some(Stage::Outfit { fresh: true });
                    } else {
                        self.abort(aurora_i18n::tr!("slcreator-outfit-failed").to_owned());
                    }
                }
                None
            }
            Stage::Look(look) => {
                self.stage = Some(Stage::AwaitLook(look));
                Some(Step::SaveLook(look.to_value()))
            }
            Stage::AwaitLook(look) => {
                let rx = self.saving_look.as_ref()?;
                match rx.try_recv() {
                    Ok(Ok(_)) => {
                        self.saving_look = None;
                        self.stage = None;
                        self.saved = Some(look);
                        self.mandatory = false;
                        self.touched = false;
                        Some(Step::Done)
                    }
                    Ok(Err(message)) => {
                        self.abort(message);
                        None
                    }
                    Err(crossbeam_channel::TryRecvError::Disconnected) => {
                        self.abort(aurora_i18n::tr!("characters-error-network").to_owned());
                        None
                    }
                    Err(crossbeam_channel::TryRecvError::Empty) => None,
                }
            }
        }
    }

    /// Cancel: the avatar drawn as worn again, and the outfit to put back
    /// when tiles were worn (the character's saved outfit).
    pub fn cancel(&mut self, world: &mut World) -> Option<crate::world::appearance::Action> {
        self.close(world);
        let outfit = self.saved.map(|s| s.outfit).filter(|id| world.inventory.folders.contains_key(id))?;
        self.touched.then(|| crate::world::appearance::Action::WearOutfits {
            folders: vec![outfit],
            append: false,
        })
    }

    /// The creator closes: the avatar drawn as worn.
    pub fn close(&mut self, world: &mut World) {
        world.preview_shape(None);
        self.previewed = None;
    }

    /// The height the shape gives (metres, as the shape editor shows it).
    pub fn height(&self, world: &World) -> Option<f32> {
        let draft = self.drafts[0].as_ref()?;
        let lib = &world.avatar_lib;
        let weights = lib.shape_params.weights_with(world.server_visual_params(), &draft.edited.params);
        let shape = lib.shape_params.shape_weights(weights, &lib.rig);
        let (_, height) = crate::scene::shape::body_size(&lib.rig, &shape.local_pos, &shape.scale);
        // LLPanelEditWearable::updateTypeSpecificControls adds 0.195 m
        Some(height + 0.195)
    }
}

/// The offline demo's Library: a "Rookery Starter" folder holding the demo
/// catalogue's items (`catalogue::demo_items`), so tiles copy and wear as
/// on the grid.
pub fn seed_demo_library(inv: &mut crate::world::inventory::Inventory) {
    let folder = Uuid::from_u128(0x5100_C2EA_7000_0000_0000_0000_0000_F000);
    if inv.folders.contains_key(&folder) {
        return;
    }
    if inv.lib_owner.is_nil() {
        inv.lib_owner = Uuid::from_u128(0x5100_C2EA_7000_0000_0000_0000_0000_F001);
    }
    inv.folders.insert(
        folder,
        crate::world::inventory::Folder {
            info: aurora_net::inventory::InvFolder {
                id: folder,
                parent: inv.lib_root,
                name: "Rookery Starter".into(),
                type_default: -1,
                version: 1,
                ..Default::default()
            },
            children: Vec::new(),
            items: Vec::new(),
            state: crate::world::inventory::FetchState::Fetched,
            library: true,
        },
    );
    let owner = inv.lib_owner;
    let items = catalogue::demo_items()
        .into_iter()
        .map(|d| InvItem {
            id: d.id,
            parent: folder,
            name: d.name.into(),
            asset_type: d.asset_type,
            inv_type: d.inv_type,
            flags: d.flags,
            owner,
            creator: owner,
            base_mask: 0x7fff_ffff,
            owner_mask: 0x7fff_ffff,
            next_owner_mask: 0x7fff_ffff,
            ..Default::default()
        })
        .collect();
    inv.add_items(items);
}
