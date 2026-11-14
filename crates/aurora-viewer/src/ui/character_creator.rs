//! The Rookery Isles Character Creator: the screen every new character goes
//! through before it may enter the world, and where a character's look is
//! changed later. The avatar stands in the middle on the Avatar Studio's
//! stage (its camera, lighting and poses); on the left the content browser
//! (skins, eyes, brows, lashes, hair, clothes, each a thumbnail); on the
//! right every MakeHuman slider of the body pack (the main body sliders,
//! then the face, torso, arms and legs, measurements and body shapes),
//! with a search, symmetry and colours. Below: undo, redo, random, reset,
//! the camera's face and body framing, and Save.
//!
//! It edits a `Look` and nothing else: the scene draws the avatar with it
//! as it changes (`Scene::set_rookery_local_look`), the app sends it to the
//! grid on Save (`characters::Action::SaveLook`).

use crate::rookery_body::fetch::{self, Manifest};
use crate::rookery_body::look::{Look, Macros, WHITE};
use crate::rookery_body::pack::Pack;
use crate::scene::rookery_body::PackStatus;
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use egui::{Context, RichText, Vec2};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The youngest a character may look, in years: Rookery Isles is an adult
/// grid, no character is made a child.
pub const MIN_AGE_YEARS: f32 = 18.0;
pub const MAX_AGE_YEARS: f32 = 90.0;
/// Width of the two side panels.
const SIDE: f32 = 330.0;
/// Thumbnail tiles.
const TILE: f32 = 88.0;
/// Thumbnails downloading at once.
const THUMBS_AT_ONCE: usize = 8;
/// Steps kept for undo.
const UNDO_STEPS: usize = 100;
/// Clothes shelves whose items are worn together rather than one at a time.
const STACKING: [&str; 5] = ["Jewelry", "Accessories", "Facial hair", "Carried", "Other"];

/// The shelves of the content browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shelf {
    Skin,
    Eyes,
    Brows,
    Lashes,
    Hair,
    Clothes,
}

/// Where the camera looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framing {
    Body,
    Face,
}

/// What the creator asks of the app this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum CreatorChoice {
    /// Save this look on the character.
    Save(Look),
    /// Leave without saving (only when the character already has a look).
    Cancel,
    /// Frame the camera.
    Frame(Framing),
}

/// Where thumbnails come from: the grid's pack, cached like the rest of it.
#[derive(Clone)]
pub struct ThumbSource {
    pub runtime: tokio::runtime::Handle,
    pub base: String,
    pub folder: PathBuf,
    pub manifest: Arc<Manifest>,
}

enum Thumb {
    Loading,
    Ready(egui::TextureHandle),
    Failed,
}

#[derive(Default)]
struct Thumbs {
    map: HashMap<String, Thumb>,
    tx: Option<crossbeam_channel::Sender<(String, Result<egui::ColorImage, String>)>>,
    rx: Option<crossbeam_channel::Receiver<(String, Result<egui::ColorImage, String>)>>,
    loading: usize,
}

impl Thumbs {
    fn poll(&mut self, ctx: &Context) {
        let Some(rx) = &self.rx else {
            return;
        };
        while let Ok((file, result)) = rx.try_recv() {
            self.loading = self.loading.saturating_sub(1);
            let thumb = match result {
                Ok(image) => Thumb::Ready(ctx.load_texture(format!("rookery-thumb-{file}"), image, egui::TextureOptions::LINEAR)),
                Err(error) => {
                    log::debug!("Rookery creator: thumbnail {file}: {error}");
                    Thumb::Failed
                }
            };
            self.map.insert(file, thumb);
        }
    }

    /// The thumbnail's texture, asked for the first time it is wanted.
    fn get(&mut self, file: &str, source: Option<&ThumbSource>, ctx: &Context) -> Option<egui::TextureId> {
        match self.map.get(file) {
            Some(Thumb::Ready(texture)) => return Some(texture.id()),
            Some(_) => return None,
            None => {}
        }
        let source = source?;
        if self.loading >= THUMBS_AT_ONCE {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
            return None;
        }
        if self.tx.is_none() {
            let (tx, rx) = crossbeam_channel::unbounded();
            self.tx = Some(tx);
            self.rx = Some(rx);
        }
        let tx = self.tx.clone()?;
        self.map.insert(file.to_owned(), Thumb::Loading);
        self.loading += 1;
        let (name, source, repaint) = (file.to_owned(), source.clone(), ctx.clone());
        source.runtime.clone().spawn(async move {
            let fetched = fetch::ensure(&source.base, &source.folder, &source.manifest, std::slice::from_ref(&name)).await;
            let result = fetched.and_then(|()| {
                let data = std::fs::read(source.folder.join(&name)).map_err(|e| e.to_string())?;
                let image = image::load_from_memory(&data).map_err(|e| e.to_string())?.to_rgba8();
                let size = [image.width() as usize, image.height() as usize];
                Ok(egui::ColorImage::from_rgba_unmultiplied(size, image.as_raw()))
            });
            let _ = tx.send((name, result));
            repaint.request_repaint();
        });
        None
    }
}

/// A tiny random source for Random (no randomness crate in the viewer).
struct Dice(u64);

impl Dice {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0x9e37_79b9_7f4a_7c15, |d| d.as_nanos() as u64);
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

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.next()
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> Option<&'a T> {
        if items.is_empty() {
            None
        } else {
            items.get(((self.next() * items.len() as f32) as usize).min(items.len() - 1))
        }
    }
}

/// One slider of the pack's layout.
#[derive(Debug, Clone)]
struct SliderEntry {
    modifier: String,
    label: String,
    /// -1..1 (a decrease and an increase) or 0..1.
    two_sided: bool,
    default: f32,
}

/// A titled group of sliders, and its subgroups.
#[derive(Debug, Clone)]
struct SliderGroup {
    title: String,
    entries: Vec<SliderEntry>,
    children: Vec<SliderGroup>,
}

/// A readable label for a modifier without one ("nose/nose-scale-horiz-decr|incr"
/// becomes "Nose scale horiz").
fn label_of(modifier: &str) -> String {
    let tail = modifier.rsplit('/').next().unwrap_or(modifier);
    let mut words: Vec<&str> = tail.split(['-', '|', '_']).filter(|w| !w.is_empty()).collect();
    words.retain(|w| !matches!(*w, "decr" | "incr" | "in" | "out" | "down" | "up" | "less" | "more" | "backward" | "forward"));
    let mut text = words.join(" ");
    if let Some(first) = text.get(0..1) {
        text = first.to_uppercase() + &text[1..];
    }
    if text.is_empty() { modifier.to_owned() } else { text }
}

/// The groups of the pack's slider layout (MakeHuman's modeling,
/// measurement and body shapes tabs), the macro sliders left out (the Body
/// section has them) and anything the grid does not show.
fn slider_groups(pack: &Pack) -> Vec<SliderGroup> {
    let modifiers: HashMap<&str, &crate::rookery_body::pack::Modifier> = pack.modifiers.iter().map(|m| (m.name.as_str(), m)).collect();
    let mut out = Vec::new();
    let Some(layout) = pack.sliders.as_object() else {
        return out;
    };
    for tab in ["modeling", "measurement", "bodyshapes"] {
        let Some(tab) = layout.get(tab).and_then(|t| t.as_object()) else {
            continue;
        };
        let mut sections: Vec<(&String, &serde_json::Value)> = tab.iter().collect();
        sections.sort_by(|a, b| {
            let order = |v: &serde_json::Value| v.get("sortOrder").and_then(serde_json::Value::as_f64).unwrap_or(9.0);
            order(a.1).total_cmp(&order(b.1))
        });
        for (title, section) in sections {
            if matches!(title.as_str(), "Macro modelling") {
                continue;
            }
            let Some(groups) = section.get("modifiers").and_then(|m| m.as_object()) else {
                continue;
            };
            let mut children = Vec::new();
            for (name, list) in groups {
                if name.eq_ignore_ascii_case("genitals") || name.eq_ignore_ascii_case("macro") {
                    continue;
                }
                let entries: Vec<SliderEntry> = list
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|entry| {
                        let modifier = entry.get("mod")?.as_str()?;
                        let m = modifiers.get(modifier)?;
                        if m.kind == "macro" {
                            return None;
                        }
                        Some(SliderEntry {
                            modifier: modifier.to_owned(),
                            label: entry
                                .get("label")
                                .and_then(|l| l.as_str())
                                .map_or_else(|| label_of(modifier), str::to_owned),
                            two_sided: m.left.is_some(),
                            default: m.default,
                        })
                    })
                    .collect();
                if !entries.is_empty() {
                    let mut title = name.clone();
                    if let Some(first) = title.get(0..1) {
                        title = first.to_uppercase() + &title[1..];
                    }
                    children.push(SliderGroup {
                        title,
                        entries,
                        children: Vec::new(),
                    });
                }
            }
            if !children.is_empty() {
                out.push(SliderGroup {
                    title: title.clone(),
                    entries: Vec::new(),
                    children,
                });
            }
        }
    }
    out
}

/// The other side's modifier of a sided one ("eyes/r-eye-..." and
/// "eyes/l-eye-..."), for symmetric editing.
pub fn mirror_of(modifier: &str) -> Option<String> {
    let (group, tail) = modifier.split_once('/')?;
    let swapped = if let Some(rest) = tail.strip_prefix("r-") {
        format!("l-{rest}")
    } else if let Some(rest) = tail.strip_prefix("l-") {
        format!("r-{rest}")
    } else {
        return None;
    };
    Some(format!("{group}/{swapped}"))
}

/// The ancestry sliders keep their sum at one, as MakeHuman's do: one moved,
/// the other two share the rest in their old proportion.
pub fn set_ancestry(m: &mut Macros, which: usize, value: f32) {
    let value = value.clamp(0.0, 1.0);
    let mut v = [m.african, m.asian, m.caucasian];
    let others: f32 = (0..3).filter(|i| *i != which).map(|i| v[i]).sum();
    let rest = 1.0 - value;
    for i in (0..3).filter(|i| *i != which) {
        v[i] = if others > 1e-6 { v[i] / others * rest } else { rest / 2.0 };
    }
    v[which] = value;
    m.african = v[0];
    m.asian = v[1];
    m.caucasian = v[2];
}

pub struct Creator {
    pub character_id: String,
    pub name: String,
    pub look: Look,
    /// The grid's look of the character (None: not made yet).
    saved: Option<Look>,
    /// No saved look: Save is the only way on, there is no Cancel.
    pub mandatory: bool,
    undo: Vec<Look>,
    redo: Vec<Look>,
    /// A change under way (a slider dragged): one undo step for all of it.
    editing: bool,
    shelf: Shelf,
    clothes_category: String,
    search: String,
    /// Narrows the content browser's tiles by name.
    content_search: String,
    symmetric: bool,
    groups: Option<Vec<SliderGroup>>,
    thumbs: Thumbs,
    dice: Dice,
    /// A save under way.
    pub saving: Option<crossbeam_channel::Receiver<Result<crate::characters::Roster, String>>>,
    pub error: Option<String>,
    confirm_leave: bool,
    /// The windows open before the creator (closed while it is open, open
    /// again when it closes).
    pub saved_panels: Option<crate::ui::Panels>,
}

impl Creator {
    pub fn new(character_id: String, name: String, saved: Option<Look>) -> Creator {
        let mandatory = saved.is_none();
        let mut look = saved.clone().unwrap_or_default();
        clamp_age(&mut look.macros);
        Creator {
            character_id,
            name,
            look,
            saved,
            mandatory,
            undo: Vec::new(),
            redo: Vec::new(),
            editing: false,
            shelf: Shelf::Skin,
            clothes_category: String::new(),
            search: String::new(),
            content_search: String::new(),
            symmetric: true,
            groups: None,
            thumbs: Thumbs::default(),
            dice: Dice::new(),
            saving: None,
            error: None,
            confirm_leave: false,
            saved_panels: None,
        }
    }

    fn changed(&self) -> bool {
        self.saved.as_ref() != Some(&self.look)
    }

    /// Takes in the grid's answer to Save: Ok(true) once saved.
    pub fn poll_save(&mut self) -> Option<Result<(), String>> {
        let rx = self.saving.as_ref()?;
        match rx.try_recv() {
            Ok(Ok(_)) => {
                self.saving = None;
                self.saved = Some(self.look.clone());
                self.mandatory = false;
                Some(Ok(()))
            }
            Ok(Err(message)) => {
                self.saving = None;
                self.error = Some(message.clone());
                Some(Err(message))
            }
            Err(crossbeam_channel::TryRecvError::Disconnected) => {
                self.saving = None;
                let message = aurora_i18n::tr!("characters-error-network").to_owned();
                self.error = Some(message.clone());
                Some(Err(message))
            }
            Err(crossbeam_channel::TryRecvError::Empty) => None,
        }
    }

    fn step_back(&mut self) {
        if let Some(previous) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.look, previous));
        }
    }

    fn step_forward(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.look, next));
        }
    }

    /// A random character: a plausible adult body and face, and a random
    /// skin, eye colour and hair from the pack.
    fn randomize(&mut self, pack: &Pack) {
        let d = &mut self.dice;
        let mut look = self.look.clone();
        let m = &mut look.macros;
        m.gender = if d.next() < 0.5 { d.range(0.0, 0.2) } else { d.range(0.8, 1.0) };
        m.set_age_years(d.range(19.0, 65.0));
        m.muscle = d.range(0.3, 0.75);
        m.weight = d.range(0.3, 0.75);
        m.height = d.range(0.3, 0.75);
        m.proportions = d.range(0.4, 0.9);
        let (a, b, c) = (d.next() + 0.05, d.next() + 0.05, d.next() + 0.05);
        let total = a + b + c;
        m.african = a / total;
        m.asian = b / total;
        m.caucasian = c / total;
        m.breast_size = d.range(0.3, 0.8);
        m.breast_firmness = d.range(0.3, 0.8);
        look.sliders.clear();
        let face: Vec<&crate::rookery_body::pack::Modifier> = pack
            .modifiers
            .iter()
            .filter(|m| {
                m.kind != "macro"
                    && matches!(
                        m.group.as_str(),
                        "head" | "forehead" | "eyebrows" | "eyes" | "nose" | "mouth" | "ears" | "chin" | "cheek" | "neck"
                    )
            })
            .collect();
        for modifier in face {
            if d.next() < 0.45 {
                let low = if modifier.left.is_some() { -0.35 } else { 0.0 };
                look.sliders.insert(modifier.name.clone(), d.range(low, 0.35));
            }
        }
        if self.symmetric {
            let names: Vec<(String, f32)> = look.sliders.iter().map(|(k, v)| (k.clone(), *v)).collect();
            for (name, value) in names {
                if name.contains("/r-")
                    && let Some(other) = mirror_of(&name)
                {
                    look.sliders.insert(other, value);
                }
            }
        }
        if let Some(skin) = d.pick(&pack.content.skin) {
            look.skin = skin.id.clone();
        }
        if let Some(eyes) = d.pick(&pack.content.eyecolour) {
            look.eye_colour = eyes.id.clone();
        }
        look.hair = d.pick(&pack.content.hair).map(|h| h.id.clone());
        look.skin_tint = WHITE;
        look.hair_tint = WHITE;
        clamp_age(&mut look.macros);
        self.undo.push(std::mem::replace(&mut self.look, look));
        self.redo.clear();
    }

    /// Draws the creator; `height` is the body's height now (metres) when
    /// built, `building` whether a new shape is still on its way.
    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &Context,
        p: &Palette,
        pack: Option<&Arc<Pack>>,
        status: &PackStatus,
        source: Option<&ThumbSource>,
        height: Option<f32>,
        building: bool,
    ) -> Option<CreatorChoice> {
        self.thumbs.poll(ctx);
        let before = self.look.clone();
        let mut choice = None;
        let screen = ctx.content_rect();
        let top = 104.0;
        let bottom_bar = 64.0;
        let side_height = (screen.height() - top - bottom_bar - 24.0).max(200.0);
        let frame = egui::Frame::new()
            .fill(p.panel.gamma_multiply(0.96))
            .stroke(widgets::divider_stroke(p))
            .corner_radius(12)
            .inner_margin(12);

        let Some(pack) = pack else {
            // the pack is still coming: say so in the middle
            egui::Area::new(egui::Id::unique("rookery_creator_wait"))
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    frame.show(ui, |ui| {
                        ui.set_width(360.0);
                        ui.label(RichText::new(aurora_i18n::tr!("creator-title")).size(20.0).strong().color(p.ink));
                        match status {
                            PackStatus::Failed(error) => {
                                ui.label(RichText::new(aurora_i18n::tr!("creator-pack-failed")).color(p.danger));
                                ui.label(RichText::new(error).size(11.0).color(p.muted));
                            }
                            _ => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(aurora_i18n::tr!("creator-pack-loading"));
                                });
                            }
                        }
                    });
                });
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            return None;
        };
        if self.groups.is_none() {
            self.groups = Some(slider_groups(pack));
        }

        // left: the content browser
        egui::Area::new(egui::Id::unique("rookery_creator_content"))
            .fixed_pos(screen.min + Vec2::new(12.0, top))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.set_width(SIDE);
                    ui.set_height(side_height);
                    self.content_panel(ui, p, pack, source);
                });
            });

        // right: the sliders
        egui::Area::new(egui::Id::unique("rookery_creator_modify"))
            .fixed_pos(egui::pos2(screen.max.x - SIDE - 36.0, screen.min.y + top))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.set_width(SIDE);
                    ui.set_height(side_height);
                    self.modify_panel(ui, p, pack, height);
                });
            });

        // below: the actions
        egui::Area::new(egui::Id::unique("rookery_creator_actions"))
            .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -12.0))
            .order(egui::Order::Middle)
            .show(ctx, |ui| {
                frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&self.name).size(16.0).strong().color(p.ink));
                            let line = if self.mandatory {
                                aurora_i18n::tr!("creator-mandatory")
                            } else {
                                aurora_i18n::tr!("creator-editing")
                            };
                            ui.label(RichText::new(line).size(11.0).color(p.muted));
                        });
                        ui.add_space(12.0);
                        let size = Vec2::new(0.0, 32.0);
                        if widgets::action_button_if(ui, p, "arrow-back-up", aurora_i18n::tr!("creator-undo"), ActionKind::Flat, size, !self.undo.is_empty())
                            .clicked()
                        {
                            self.step_back();
                        }
                        if widgets::action_button_if(
                            ui,
                            p,
                            "arrow-forward-up",
                            aurora_i18n::tr!("creator-redo"),
                            ActionKind::Flat,
                            size,
                            !self.redo.is_empty(),
                        )
                        .clicked()
                        {
                            self.step_forward();
                        }
                        if widgets::action_button(ui, p, "wand", aurora_i18n::tr!("creator-random"), ActionKind::Flat, size)
                            .on_hover_text(aurora_i18n::tr!("creator-random-hint"))
                            .clicked()
                        {
                            self.randomize(pack);
                        }
                        if widgets::action_button(ui, p, "refresh", aurora_i18n::tr!("creator-reset"), ActionKind::Flat, size)
                            .on_hover_text(aurora_i18n::tr!("creator-reset-hint"))
                            .clicked()
                        {
                            let mut fresh = self.saved.clone().unwrap_or_default();
                            clamp_age(&mut fresh.macros);
                            if fresh != self.look {
                                self.undo.push(std::mem::replace(&mut self.look, fresh));
                                self.redo.clear();
                            }
                        }
                        widgets::divider(ui, p);
                        if widgets::action_button(ui, p, "user-circle", aurora_i18n::tr!("creator-frame-face"), ActionKind::Flat, size).clicked() {
                            choice = Some(CreatorChoice::Frame(Framing::Face));
                        }
                        if widgets::action_button(ui, p, "man-filled", aurora_i18n::tr!("creator-frame-body"), ActionKind::Flat, size).clicked() {
                            choice = Some(CreatorChoice::Frame(Framing::Body));
                        }
                        widgets::divider(ui, p);
                        if building {
                            ui.spinner();
                        }
                        let saving = self.saving.is_some();
                        if !self.mandatory
                            && widgets::action_button_if(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size, !saving).clicked()
                        {
                            if self.changed() {
                                self.confirm_leave = true;
                            } else {
                                choice = Some(CreatorChoice::Cancel);
                            }
                        }
                        let label = if self.mandatory {
                            aurora_i18n::tr!("creator-save-enter")
                        } else {
                            aurora_i18n::tr!("creator-save")
                        };
                        if widgets::action_button_if(ui, p, "device-floppy", label, ActionKind::Primary, Vec2::new(0.0, 36.0), !saving).clicked() {
                            self.error = None;
                            choice = Some(CreatorChoice::Save(self.look.clone()));
                        }
                        if saving {
                            ui.spinner();
                        }
                    });
                    if let Some(error) = &self.error {
                        ui.label(RichText::new(error).color(p.danger));
                    }
                });
            });

        if self.confirm_leave {
            let mut answer = None;
            let modal = widgets::Modal::new(egui::Id::unique("rookery_creator_leave")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(aurora_i18n::tr!("creator-leave-question"));
                ui.horizontal(|ui| {
                    if widgets::action_button(ui, p, "x", aurora_i18n::tr!("creator-leave"), ActionKind::Danger, Vec2::new(0.0, 28.0)).clicked() {
                        answer = Some(true);
                    }
                    if ui.button(aurora_i18n::tr!("creator-keep-editing")).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if answer.is_some() || modal.should_close() {
                self.confirm_leave = false;
                if answer == Some(true) {
                    choice = Some(CreatorChoice::Cancel);
                }
            }
        }

        // keyboard: undo and redo
        let (undo, redo) = ctx.input(|i| {
            let command = i.modifiers.command;
            (
                command && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                command && (i.key_pressed(egui::Key::Y) || (i.modifiers.shift && i.key_pressed(egui::Key::Z))),
            )
        });
        if ctx.memory(|m| m.focused().is_none()) {
            if undo {
                self.step_back();
            } else if redo {
                self.step_forward();
            }
        }

        // one undo step for each change, a whole drag being one change
        clamp_age(&mut self.look.macros);
        let dragging = ctx.dragged_id().is_some() || ctx.input(|i| i.pointer.any_down());
        if self.look != before {
            if !self.editing {
                self.undo.push(before);
                if self.undo.len() > UNDO_STEPS {
                    self.undo.remove(0);
                }
                self.redo.clear();
            }
            self.editing = dragging;
        } else if !dragging {
            self.editing = false;
        }
        choice
    }

    fn content_panel(&mut self, ui: &mut egui::Ui, p: &Palette, pack: &Pack, source: Option<&ThumbSource>) {
        ui.label(RichText::new(aurora_i18n::tr!("creator-content")).size(17.0).strong().color(p.ink));
        ui.horizontal_wrapped(|ui| {
            for (shelf, label) in [
                (Shelf::Skin, aurora_i18n::tr!("creator-shelf-skin")),
                (Shelf::Eyes, aurora_i18n::tr!("creator-shelf-eyes")),
                (Shelf::Brows, aurora_i18n::tr!("creator-shelf-brows")),
                (Shelf::Lashes, aurora_i18n::tr!("creator-shelf-lashes")),
                (Shelf::Hair, aurora_i18n::tr!("creator-shelf-hair")),
                (Shelf::Clothes, aurora_i18n::tr!("creator-shelf-clothes")),
            ] {
                if ui.selectable_label(self.shelf == shelf, label).clicked() {
                    self.shelf = shelf;
                }
            }
        });
        widgets::divider(ui, p);
        // colours of this shelf
        match self.shelf {
            Shelf::Skin => colour_row(ui, p, aurora_i18n::tr!("creator-skin-tint"), &mut self.look.skin_tint),
            Shelf::Eyes => colour_row(ui, p, aurora_i18n::tr!("creator-eye-tint"), &mut self.look.eye_tint),
            Shelf::Brows | Shelf::Lashes => colour_row(ui, p, aurora_i18n::tr!("creator-brow-tint"), &mut self.look.eyebrow_tint),
            Shelf::Hair => colour_row(ui, p, aurora_i18n::tr!("creator-hair-tint"), &mut self.look.hair_tint),
            Shelf::Clothes => {
                let mut categories: Vec<String> = Vec::new();
                for c in &pack.content.clothes {
                    if !categories.contains(&c.category) {
                        categories.push(c.category.clone());
                    }
                }
                if self.clothes_category.is_empty() || !categories.contains(&self.clothes_category) {
                    self.clothes_category = categories.first().cloned().unwrap_or_default();
                }
                ui.horizontal_wrapped(|ui| {
                    for c in &categories {
                        let shown = if c.is_empty() { aurora_i18n::tr!("creator-clothes-other") } else { c.as_str() };
                        if ui.selectable_label(&self.clothes_category == c, shown).clicked() {
                            self.clothes_category = c.clone();
                        }
                    }
                });
                // what is worn, with its colour and a way to take it off
                if !self.look.clothes.is_empty() {
                    ui.label(RichText::new(aurora_i18n::tr!("creator-wearing")).size(12.0).color(p.muted));
                    let worn = self.look.clothes.clone();
                    for id in worn {
                        let name = pack.content.proxy(&id).map_or(id.clone(), |c| pretty(&c.name));
                        ui.horizontal(|ui| {
                            let tint = self.look.clothes_tints.entry(id.clone()).or_insert(WHITE);
                            ui.color_edit_button_rgb(tint);
                            ui.label(RichText::new(name).size(12.0).color(p.ink));
                            if ui.small_button("x").on_hover_text(aurora_i18n::tr!("creator-take-off")).clicked() {
                                self.look.clothes.retain(|c| c != &id);
                                self.look.clothes_tints.remove(&id);
                            }
                        });
                    }
                    self.look.clothes_tints.retain(|id, tint| self.look.clothes.contains(id) && *tint != WHITE);
                }
            }
        }
        widgets::divider(ui, p);
        ui.add(
            egui::TextEdit::singleline(&mut self.content_search)
                .hint_text(aurora_i18n::tr!("creator-content-search"))
                .desired_width(f32::INFINITY),
        );
        let wanted = self.content_search.trim().to_lowercase();
        let ctx = ui.ctx().clone();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let columns = ((ui.available_width() + 6.0) / (TILE + 6.0)).floor().max(1.0) as usize;
            // (id, name, thumbnail, selected)
            let mut tiles: Vec<(Option<String>, String, Option<String>, bool)> = Vec::new();
            // CC-BY items name their makers (the licence asks it)
            let credit = |licence: &str, author: &str| -> String {
                match (licence, author.trim()) {
                    ("CC-BY", "") => aurora_i18n::tr!("creator-credit-ccby").to_owned(),
                    ("CC-BY", by) => aurora_i18n::tr!("creator-credit-by", author = by),
                    _ => String::new(),
                }
            };
            match self.shelf {
                Shelf::Skin => {
                    for s in &pack.content.skin {
                        tiles.push((Some(s.id.clone()), named(&s.name, &credit(&s.licence, "")), s.thumbnail.clone(), self.look.skin == s.id));
                    }
                }
                Shelf::Eyes => {
                    for s in &pack.content.eyecolour {
                        tiles.push((Some(s.id.clone()), named(&s.name, &credit(&s.licence, "")), s.thumbnail.clone(), self.look.eye_colour == s.id));
                    }
                }
                Shelf::Brows => {
                    tiles.push((None, aurora_i18n::tr!("creator-none").to_owned(), None, self.look.eyebrows.is_none()));
                    for s in &pack.content.eyebrows {
                        tiles.push((Some(s.id.clone()), named(&s.name, &credit(&s.licence, &s.author)), s.thumbnail.clone(), self.look.eyebrows.as_ref() == Some(&s.id)));
                    }
                }
                Shelf::Lashes => {
                    tiles.push((None, aurora_i18n::tr!("creator-none").to_owned(), None, self.look.eyelashes.is_none()));
                    for s in &pack.content.eyelashes {
                        tiles.push((Some(s.id.clone()), named(&s.name, &credit(&s.licence, &s.author)), s.thumbnail.clone(), self.look.eyelashes.as_ref() == Some(&s.id)));
                    }
                }
                Shelf::Hair => {
                    tiles.push((None, aurora_i18n::tr!("creator-bald").to_owned(), None, self.look.hair.is_none()));
                    for s in &pack.content.hair {
                        tiles.push((Some(s.id.clone()), named(&s.name, &credit(&s.licence, &s.author)), s.thumbnail.clone(), self.look.hair.as_ref() == Some(&s.id)));
                    }
                }
                Shelf::Clothes => {
                    for s in pack.content.clothes.iter().filter(|c| c.category == self.clothes_category) {
                        tiles.push((Some(s.id.clone()), named(&s.name, &credit(&s.licence, &s.author)), s.thumbnail.clone(), self.look.clothes.contains(&s.id)));
                    }
                }
            }
            // the search: the tiles whose name has it (the "none" tile stays)
            if !wanted.is_empty() {
                tiles.retain(|(id, name, _, _)| id.is_none() || name.to_lowercase().contains(&wanted));
            }
            let mut clicked: Option<Option<String>> = None;
            for row in tiles.chunks(columns) {
                ui.horizontal(|ui| {
                    for (id, name, thumb, selected) in row {
                        let texture = thumb.as_deref().and_then(|t| self.thumbs.get(t, source, &ctx));
                        if tile(ui, p, texture, name, *selected).clicked() {
                            clicked = Some(id.clone());
                        }
                    }
                });
            }
            if let Some(id) = clicked {
                self.choose(pack, id);
            }
        });
    }

    /// A tile of the content browser chosen.
    fn choose(&mut self, pack: &Pack, id: Option<String>) {
        match self.shelf {
            Shelf::Skin => {
                if let Some(id) = id {
                    self.look.skin = id;
                }
            }
            Shelf::Eyes => {
                if let Some(id) = id {
                    self.look.eye_colour = id;
                }
            }
            Shelf::Brows => self.look.eyebrows = id,
            Shelf::Lashes => self.look.eyelashes = id,
            Shelf::Hair => self.look.hair = id,
            Shelf::Clothes => {
                let Some(id) = id else {
                    return;
                };
                if self.look.clothes.contains(&id) {
                    self.look.clothes.retain(|c| c != &id);
                    self.look.clothes_tints.remove(&id);
                    return;
                }
                // one garment of a kind: a new shirt replaces the shirt
                // (jewelry, accessories, facial hair and what is carried are
                // worn together)
                let category = pack.content.proxy(&id).map(|c| c.category.clone()).unwrap_or_default();
                if !STACKING.contains(&category.as_str()) {
                    self.look
                        .clothes
                        .retain(|c| pack.content.proxy(c).is_none_or(|worn| worn.category != category));
                }
                self.look.clothes.push(id);
                // layered as MakeHuman layers them: under the ones above
                self.look
                    .clothes
                    .sort_by_key(|c| pack.content.proxy(c).map_or(50, |worn| worn.z_depth));
            }
        }
    }

    fn modify_panel(&mut self, ui: &mut egui::Ui, p: &Palette, pack: &Pack, height: Option<f32>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("creator-modify")).size(17.0).strong().color(p.ink));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.checkbox(&mut self.symmetric, aurora_i18n::tr!("creator-symmetry"))
                    .on_hover_text(aurora_i18n::tr!("creator-symmetry-hint"));
            });
        });
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text(aurora_i18n::tr!("creator-search"))
                .desired_width(f32::INFINITY),
        );
        widgets::divider(ui, p);
        let search = self.search.trim().to_lowercase();
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            if search.is_empty() {
                egui::CollapsingHeader::new(RichText::new(aurora_i18n::tr!("creator-body")).strong())
                    .default_open(true)
                    .show(ui, |ui| self.body_sliders(ui, p, height));
            }
            let groups = self.groups.clone().unwrap_or_default();
            for group in &groups {
                self.group(ui, p, pack, group, &search, 0);
            }
        });
    }

    fn body_sliders(&mut self, ui: &mut egui::Ui, p: &Palette, height: Option<f32>) {
        let m = &mut self.look.macros;
        slider(ui, p, aurora_i18n::tr!("creator-gender"), &mut m.gender, 0.0..=1.0, 0.0, Some((aurora_i18n::tr!("creator-female"), aurora_i18n::tr!("creator-male"))));
        let mut years = m.age_years();
        ui.horizontal(|ui| {
            ui.label(RichText::new(aurora_i18n::tr!("creator-age")).size(12.0).color(p.ink));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(
                        egui::Slider::new(&mut years, MIN_AGE_YEARS..=MAX_AGE_YEARS)
                            .suffix(aurora_i18n::tr!("creator-years"))
                            .custom_formatter(|v, _| format!("{v:.0}")),
                    )
                    .changed()
                {
                    m.set_age_years(years);
                }
            });
        });
        slider(ui, p, aurora_i18n::tr!("creator-muscle"), &mut m.muscle, 0.0..=1.0, 0.5, None);
        slider(ui, p, aurora_i18n::tr!("creator-weight"), &mut m.weight, 0.0..=1.0, 0.5, Some((aurora_i18n::tr!("creator-thin"), aurora_i18n::tr!("creator-heavy"))));
        slider(ui, p, aurora_i18n::tr!("creator-height"), &mut m.height, 0.0..=1.0, 0.5, None);
        if let Some(height) = height {
            let cm = format!("{:.0}", height * 100.0);
            let feet_total = height / 0.3048;
            let feet = feet_total.floor();
            let inches = format!("{:.0}", (feet_total - feet) * 12.0);
            let feet = format!("{feet:.0}");
            ui.label(
                RichText::new(aurora_i18n::tr!("creator-height-now", cm = cm.as_str(), feet = feet.as_str(), inches = inches.as_str()))
                    .size(11.0)
                    .color(p.muted),
            );
        }
        slider(ui, p, aurora_i18n::tr!("creator-proportions"), &mut m.proportions, 0.0..=1.0, 0.5, None);
        ui.label(RichText::new(aurora_i18n::tr!("creator-ancestry")).size(12.0).strong().color(p.muted));
        let mut ancestry = [m.african, m.asian, m.caucasian];
        for (i, label) in [
            aurora_i18n::tr!("creator-african"),
            aurora_i18n::tr!("creator-asian"),
            aurora_i18n::tr!("creator-caucasian"),
        ]
        .into_iter()
        .enumerate()
        {
            let before = ancestry[i];
            slider(ui, p, label, &mut ancestry[i], 0.0..=1.0, 1.0 / 3.0, None);
            if (ancestry[i] - before).abs() > f32::EPSILON {
                set_ancestry(m, i, ancestry[i]);
            }
        }
        ui.label(RichText::new(aurora_i18n::tr!("creator-chest")).size(12.0).strong().color(p.muted));
        slider(ui, p, aurora_i18n::tr!("creator-breast-size"), &mut m.breast_size, 0.0..=1.0, 0.5, None);
        slider(ui, p, aurora_i18n::tr!("creator-breast-firmness"), &mut m.breast_firmness, 0.0..=1.0, 0.5, None);
    }

    fn group(&mut self, ui: &mut egui::Ui, p: &Palette, pack: &Pack, group: &SliderGroup, search: &str, depth: usize) {
        let matches = |e: &SliderEntry| search.is_empty() || e.label.to_lowercase().contains(search) || e.modifier.to_lowercase().contains(search);
        let any = |g: &SliderGroup| -> bool {
            fn walk(g: &SliderGroup, f: &dyn Fn(&SliderEntry) -> bool) -> bool {
                g.entries.iter().any(f) || g.children.iter().any(|c| walk(c, f))
            }
            walk(g, &matches)
        };
        if !any(group) {
            return;
        }
        let header = egui::CollapsingHeader::new(RichText::new(&group.title).strong())
            .id_salt(("rookery_creator_group", depth, &group.title))
            .default_open(!search.is_empty());
        header.show(ui, |ui| {
            for entry in group.entries.iter().filter(|e| matches(e)) {
                let mut value = self.look.sliders.get(&entry.modifier).copied().unwrap_or(entry.default);
                let range = if entry.two_sided { -1.0..=1.0 } else { 0.0..=1.0 };
                let before = value;
                slider(ui, p, &entry.label, &mut value, range, entry.default, None);
                if (value - before).abs() > f32::EPSILON {
                    self.set_slider(&entry.modifier, value, entry.default);
                }
            }
            for child in &group.children {
                self.group(ui, p, pack, child, search, depth + 1);
            }
        });
    }

    fn set_slider(&mut self, modifier: &str, value: f32, default: f32) {
        let mut set = |name: &str| {
            if (value - default).abs() < 1e-4 {
                self.look.sliders.remove(name);
            } else {
                self.look.sliders.insert(name.to_owned(), value);
            }
        };
        set(modifier);
        if self.symmetric
            && let Some(other) = mirror_of(modifier)
        {
            set(&other);
        }
    }
}

/// The youngest age the grid allows, kept whatever a saved look says.
pub fn clamp_age(m: &mut Macros) {
    let years = m.age_years();
    if !(MIN_AGE_YEARS..=MAX_AGE_YEARS).contains(&years) {
        m.set_age_years(years.clamp(MIN_AGE_YEARS, MAX_AGE_YEARS));
    }
}

/// A tile's name with its credit on a second line (the tile shows the
/// first line, its hover all of it).
fn named(name: &str, credit: &str) -> String {
    if credit.is_empty() { pretty(name) } else { format!("{}\n{credit}", pretty(name)) }
}

/// "young_eurasian_female" becomes "Young eurasian female".
fn pretty(name: &str) -> String {
    let text = name.replace(['_', '-'], " ");
    let text = text.trim();
    match text.get(0..1) {
        Some(first) => first.to_uppercase() + &text[1..],
        None => String::new(),
    }
}

/// A labelled slider with a double click (or the small button) back to its
/// default; `ends` names the two ends below it.
fn slider(ui: &mut egui::Ui, p: &Palette, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, default: f32, ends: Option<(&str, &str)>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(label).size(12.0).color(p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled((*value - default).abs() > 1e-4, egui::Button::new("↺").small())
                .on_hover_text(aurora_i18n::tr!("creator-default"))
                .clicked()
            {
                *value = default;
            }
            // (two decimals shown by a formatter: egui's fixed decimals round
            // the value itself each frame it stores it back (Slider::
            // add_contents), which drifted the ancestry sliders, and with
            // them the look, on every frame; and only a change made with
            // the slider is written back)
            let mut shown = *value;
            let response = ui.add(egui::Slider::new(&mut shown, range).show_value(true).custom_formatter(|v, _| format!("{v:.2}")));
            if response.double_clicked() {
                *value = default;
            } else if response.changed() {
                *value = shown;
            }
        });
    });
    if let Some((low, high)) = ends {
        ui.horizontal(|ui| {
            ui.label(RichText::new(low).size(10.0).color(p.muted));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(high).size(10.0).color(p.muted));
            });
        });
    }
}

fn colour_row(ui: &mut egui::Ui, p: &Palette, label: &str, tint: &mut [f32; 3]) {
    ui.horizontal(|ui| {
        ui.color_edit_button_rgb(tint);
        ui.label(RichText::new(label).size(12.0).color(p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled(*tint != WHITE, egui::Button::new("↺").small())
                .on_hover_text(aurora_i18n::tr!("creator-default"))
                .clicked()
            {
                *tint = WHITE;
            }
        });
    });
}

/// A thumbnail tile with its name under it.
fn tile(ui: &mut egui::Ui, p: &Palette, texture: Option<egui::TextureId>, name: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(TILE, TILE + 18.0), egui::Sense::click());
    let painter = ui.painter_at(rect);
    let image = egui::Rect::from_min_size(rect.min, Vec2::splat(TILE));
    let fill = if response.hovered() { p.raised } else { p.field };
    painter.rect_filled(image, 8.0, fill);
    match texture {
        Some(texture) => {
            painter.image(
                texture,
                image.shrink(4.0),
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        None => {
            painter.text(image.center(), egui::Align2::CENTER_CENTER, "·", egui::FontId::proportional(22.0), p.muted);
        }
    }
    if selected {
        painter.rect_stroke(image, 8.0, egui::Stroke::new(2.5, p.violet_light), egui::StrokeKind::Inside);
    }
    let first = name.lines().next().unwrap_or(name);
    let mut short: String = first.chars().take(14).collect();
    if first.chars().count() > 14 {
        short.push('.');
    }
    painter.text(
        egui::pos2(rect.center().x, image.max.y + 9.0),
        egui::Align2::CENTER_CENTER,
        short,
        egui::FontId::proportional(10.5),
        if selected { p.ink } else { p.muted },
    );
    response.on_hover_text(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sided_sliders_mirror_and_labels_read() {
        assert_eq!(mirror_of("eyes/r-eye-height1-decr|incr").as_deref(), Some("eyes/l-eye-height1-decr|incr"));
        assert_eq!(mirror_of("armslegs/l-hand-scale-decr|incr").as_deref(), Some("armslegs/r-hand-scale-decr|incr"));
        assert_eq!(mirror_of("nose/nose-scale-horiz-decr|incr"), None);
        assert_eq!(label_of("nose/nose-scale-horiz-decr|incr"), "Nose scale horiz");
        assert_eq!(pretty("young_eurasian_female"), "Young eurasian female");
    }

    #[test]
    fn ancestry_keeps_its_sum_and_age_stays_adult() {
        let mut m = Macros::default();
        set_ancestry(&mut m, 0, 0.7);
        let total = m.african + m.asian + m.caucasian;
        assert!((total - 1.0).abs() < 1e-5, "{total}");
        assert!((m.african - 0.7).abs() < 1e-6);
        assert!((m.asian - m.caucasian).abs() < 1e-6, "the other two share the rest alike");
        m.set_age_years(11.0);
        clamp_age(&mut m);
        assert!((m.age_years() - MIN_AGE_YEARS).abs() < 1e-3, "{}", m.age_years());
        let creator = Creator::new("id".into(), "Ashember Love".into(), None);
        assert!(creator.mandatory);
        assert!(creator.look.macros.age_years() >= MIN_AGE_YEARS);
    }

    /// Shown frame after frame with nobody touching it, the creator leaves
    /// its look as it is (a look that changes rebuilds the body each time).
    #[test]
    fn an_untouched_creator_keeps_its_look() {
        let folder = std::env::var_os("ROOKERY_BODY_PACK")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:/HomeWorldz/var/rookery-body/pack"));
        if !folder.join("body.json").exists() {
            eprintln!("skipping: no Rookery body pack on this machine");
            return;
        }
        let pack = Arc::new(Pack::load(&folder).expect("pack"));
        let theme = crate::theme::Theme::default();
        let palette = theme.palette();
        let mut creator = Creator::new("id".into(), "Ashember Love".into(), None);
        let start = creator.look.clone();
        let ctx = egui::Context::default();
        theme.apply(&ctx, 1.0);
        for _ in 0..6 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    creator.show(&ctx, &palette, Some(&pack), &PackStatus::Ready, None, Some(1.6), false);
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(creator.look, start);
        assert!(creator.undo.is_empty());
    }

    #[test]
    fn random_draws_stay_in_their_range() {
        let mut d = Dice::new();
        for _ in 0..1000 {
            let v = d.range(0.2, 0.4);
            assert!((0.2..=0.4).contains(&v), "{v}");
        }
        assert_eq!(d.pick::<u8>(&[]), None);
        assert_eq!(d.pick(&[7u8]), Some(&7));
    }
}
