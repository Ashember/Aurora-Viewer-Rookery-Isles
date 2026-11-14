//! Rookery Isles character select: the window shown in the Rookery Gate, the
//! small always-up region every login lands in. It lists the account's
//! characters (up to three), creates one (a two-word name and the world it
//! starts in) and deletes one. Play asks the grid where that character goes:
//! when the region is accepting people the avatar is sent there; when it is
//! not, the window says so and offers to wait (it asks again every few seconds
//! and goes as soon as it can) or to go somewhere else that is up now. What it
//! shows is what the grid answered (`crate::characters`); it decides nothing.

use crate::characters::{Action, Destination, RegionReadiness, Roster};
use crate::theme::Palette;
use crate::ui::widgets::{self, ActionKind};
use egui::{Context, RichText, Vec2};

/// How often a wait asks the grid again.
const WAIT_EVERY: std::time::Duration = std::time::Duration::from_secs(8);

/// What the window asks of the app this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    /// Send this request to the grid.
    Request(Action),
    /// Fetch the regions that are up now ("Somewhere else").
    LoadRegions,
    /// Go there now and close the window.
    Go(Destination),
    /// Close the window and stay in the Gate.
    Close,
    /// Open the Character Creator on this character: (id, name, its saved
    /// look). A Play on a character not made yet comes here first.
    EditLook(String, String, Option<crate::rookery_body::look::Look>),
}

/// A Play whose region was not ready: the character and where it was going.
struct Waiting {
    id: String,
    name: String,
    destination: Destination,
    /// Asking again on its own while true.
    auto: bool,
    asked_at: std::time::Instant,
}

#[derive(Default)]
pub struct Select {
    /// The last answer of the grid; None until the first one.
    pub roster: Option<Roster>,
    /// A request under way.
    pub pending: Option<crossbeam_channel::Receiver<Result<Roster, String>>>,
    /// The "Somewhere else" list under way, and its answer.
    pub regions_pending: Option<crossbeam_channel::Receiver<Result<Vec<RegionReadiness>, String>>>,
    regions: Option<Vec<RegionReadiness>>,
    pub error: Option<String>,
    /// The character a Play is for, until its destination answer comes.
    playing: Option<(String, String)>,
    waiting: Option<Waiting>,
    /// The create form, open.
    creating: bool,
    name: String,
    world: String,
    /// A delete waiting for its confirmation: (id, name).
    confirm_delete: Option<(String, String)>,
}

impl Select {
    /// Takes in the grid's answers once they have come; a Play whose region
    /// is up turns into Go at once.
    fn poll(&mut self) -> Option<Choice> {
        let mut go = None;
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Ok(Ok(roster)) => {
                    self.pending = None;
                    self.error = None;
                    if self
                        .roster
                        .as_ref()
                        .is_some_and(|before| roster.characters.len() > before.characters.len())
                    {
                        self.creating = false;
                        self.name.clear();
                    }
                    if roster.characters.is_empty() {
                        self.creating = true;
                    }
                    if self.world.is_empty() || !roster.worlds.iter().any(|w| w.key == self.world) {
                        self.world = roster.worlds.first().map(|w| w.key.clone()).unwrap_or_default();
                    }
                    if roster.needs_look
                        && let Some((id, name)) = self.playing.take()
                    {
                        // not made yet: the creator, and Play again once saved
                        go = Some(Choice::EditLook(id, name, None));
                    }
                    if let Some(destination) = roster.destination.clone() {
                        let (id, name) = self
                            .playing
                            .take()
                            .or_else(|| self.waiting.as_ref().map(|w| (w.id.clone(), w.name.clone())))
                            .unwrap_or_default();
                        if destination.ready {
                            self.waiting = None;
                            go = Some(Choice::Go(destination));
                        } else {
                            let auto = self.waiting.as_ref().is_some_and(|w| w.auto);
                            self.waiting = Some(Waiting {
                                id,
                                name,
                                destination,
                                auto,
                                asked_at: std::time::Instant::now(),
                            });
                        }
                    }
                    self.roster = Some(roster);
                }
                Ok(Err(message)) => {
                    self.pending = None;
                    self.playing = None;
                    self.error = Some(message);
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.playing = None;
                    self.error = Some(aurora_i18n::tr!("characters-error-network").to_owned());
                }
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.regions_pending {
            match rx.try_recv() {
                Ok(result) => {
                    self.regions_pending = None;
                    match result {
                        Ok(regions) => self.regions = Some(regions),
                        Err(message) => self.error = Some(message),
                    }
                }
                Err(crossbeam_channel::TryRecvError::Disconnected) => self.regions_pending = None,
                Err(crossbeam_channel::TryRecvError::Empty) => {}
            }
        }
        go
    }

    pub fn show(&mut self, ctx: &Context, p: &Palette) -> Option<Choice> {
        if let Some(go) = self.poll() {
            return Some(go);
        }
        // a wait asks again on its own
        if self.pending.is_none()
            && let Some(waiting) = &self.waiting
            && waiting.auto
            && waiting.asked_at.elapsed() >= WAIT_EVERY
        {
            return Some(Choice::Request(Action::Destination { id: waiting.id.clone() }));
        }
        if self.pending.is_some() || self.regions_pending.is_some() || self.waiting.as_ref().is_some_and(|w| w.auto) {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        let busy = self.pending.is_some();
        let mut choice = None;
        let width = (ctx.content_rect().width() - 64.0).clamp(320.0, 620.0);
        let response = widgets::Modal::new(egui::Id::unique("rookery_characters"))
            .login_surface(p)
            .stronger_tint()
            .show(ctx, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    widgets::glyph(ui, "users", p.violet_light, 30.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(aurora_i18n::tr!("characters-title")).size(21.0).strong().color(p.ink));
                        ui.label(RichText::new(aurora_i18n::tr!("characters-description")).size(12.0).color(p.muted));
                    });
                });
                ui.add_space(4.0);
                if self.waiting.is_some() {
                    choice = self.waiting_panel(ui, p, busy);
                    return;
                }
                let Some(roster) = self.roster.clone() else {
                    if busy {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(aurora_i18n::tr!("characters-loading"));
                        });
                    }
                    self.error_line(ui, p);
                    if widgets::action_button(ui, p, "x", aurora_i18n::tr!("characters-close"), ActionKind::Flat, Vec2::new(140.0, 30.0))
                        .clicked()
                    {
                        choice = Some(Choice::Close);
                    }
                    return;
                };
                for character in &roster.characters {
                    egui::Frame::new().fill(p.field).corner_radius(8).inner_margin(12).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&character.name).size(17.0).strong().color(p.ink));
                                let world = roster.world_name(&character.world);
                                let mut line = if world.is_empty() {
                                    aurora_i18n::tr!("characters-no-world").to_owned()
                                } else {
                                    world.to_owned()
                                };
                                if character.last_location.is_none() {
                                    line.push_str(" · ");
                                    line.push_str(aurora_i18n::tr!("characters-new"));
                                }
                                ui.label(RichText::new(line).size(12.0).color(p.muted));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if widgets::action_button_if(
                                    ui,
                                    p,
                                    "login-2",
                                    aurora_i18n::tr!("characters-play"),
                                    ActionKind::Primary,
                                    Vec2::new(110.0, 34.0),
                                    !busy,
                                )
                                .clicked()
                                {
                                    self.playing = Some((character.id.clone(), character.name.clone()));
                                    choice = Some(Choice::Request(Action::Play { id: character.id.clone() }));
                                }
                                if widgets::action_button_if(
                                    ui,
                                    p,
                                    "shirt",
                                    "",
                                    ActionKind::Flat,
                                    Vec2::new(34.0, 34.0),
                                    !busy,
                                )
                                .on_hover_text(aurora_i18n::tr!("characters-appearance"))
                                .clicked()
                                {
                                    choice = Some(Choice::EditLook(
                                        character.id.clone(),
                                        character.name.clone(),
                                        character.saved_look(),
                                    ));
                                }
                                if widgets::action_button_if(
                                    ui,
                                    p,
                                    "trash-filled",
                                    "",
                                    ActionKind::Flat,
                                    Vec2::new(34.0, 34.0),
                                    !busy,
                                )
                                .on_hover_text(aurora_i18n::tr!("characters-delete"))
                                .clicked()
                                {
                                    self.confirm_delete = Some((character.id.clone(), character.name.clone()));
                                }
                            });
                        });
                    });
                }
                if roster.characters.is_empty() && !self.creating {
                    ui.label(RichText::new(aurora_i18n::tr!("characters-none")).color(p.muted));
                }
                ui.label(
                    RichText::new(aurora_i18n::tr!(
                        "characters-slots",
                        used = roster.characters.len().to_string(),
                        max = roster.max.to_string()
                    ))
                    .size(11.5)
                    .color(p.muted),
                );
                if self.creating && !roster.full() {
                    widgets::divider(ui, p);
                    ui.label(RichText::new(aurora_i18n::tr!("characters-create")).size(15.0).strong().color(p.ink));
                    ui.label(RichText::new(aurora_i18n::tr!("characters-name")).size(12.0).color(p.muted));
                    let name = ui.add_enabled(
                        !busy,
                        egui::TextEdit::singleline(&mut self.name)
                            .hint_text(aurora_i18n::tr!("characters-name-hint"))
                            .char_limit(64)
                            .desired_width(f32::INFINITY),
                    );
                    if roster.worlds.is_empty() {
                        ui.label(RichText::new(aurora_i18n::tr!("characters-no-worlds")).color(p.warn));
                    } else {
                        ui.label(RichText::new(aurora_i18n::tr!("characters-world")).size(12.0).color(p.muted));
                        for world in &roster.worlds {
                            ui.add_enabled_ui(!busy, |ui| {
                                ui.radio_value(&mut self.world, world.key.clone(), RichText::new(&world.name).strong());
                            });
                            if !world.description.trim().is_empty() {
                                ui.label(RichText::new(&world.description).size(11.5).color(p.muted));
                            }
                        }
                    }
                    let ready = !busy && self.name.split_whitespace().count() == 2 && !self.world.is_empty();
                    let submit = name.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.horizontal(|ui| {
                        if widgets::action_button_if(
                            ui,
                            p,
                            "plus",
                            aurora_i18n::tr!("characters-create-button"),
                            ActionKind::Primary,
                            Vec2::new(160.0, 32.0),
                            ready,
                        )
                        .clicked()
                            || (submit && ready)
                        {
                            choice = Some(Choice::Request(Action::Create {
                                name: self.name.trim().to_owned(),
                                world: self.world.clone(),
                            }));
                        }
                        if !roster.characters.is_empty()
                            && widgets::action_button_if(
                                ui,
                                p,
                                "x",
                                aurora_i18n::tr!("common-cancel"),
                                ActionKind::Flat,
                                Vec2::new(100.0, 32.0),
                                !busy,
                            )
                            .clicked()
                        {
                            self.creating = false;
                        }
                    });
                    ui.label(RichText::new(aurora_i18n::tr!("characters-name-rule")).size(11.0).color(p.muted));
                } else if !roster.full()
                    && widgets::action_button_if(
                        ui,
                        p,
                        "plus",
                        aurora_i18n::tr!("characters-new-character"),
                        ActionKind::Flat,
                        Vec2::new(200.0, 32.0),
                        !busy,
                    )
                    .clicked()
                {
                    self.creating = true;
                }
                if busy {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(aurora_i18n::tr!("characters-working"));
                    });
                }
                self.error_line(ui, p);
                widgets::divider(ui, p);
                if widgets::action_button_if(
                    ui,
                    p,
                    "x",
                    aurora_i18n::tr!("characters-close"),
                    ActionKind::Flat,
                    Vec2::new(140.0, 30.0),
                    true,
                )
                .on_hover_text(aurora_i18n::tr!("characters-close-hint"))
                .clicked()
                {
                    choice = Some(Choice::Close);
                }
            });
        if response.should_close() && self.confirm_delete.is_none() && choice.is_none() {
            choice = Some(Choice::Close);
        }
        if let Some((id, name)) = self.confirm_delete.clone() {
            let mut answer = None;
            let modal = widgets::Modal::new(egui::Id::unique("rookery_character_delete")).show(ctx, |ui| {
                ui.set_width(380.0);
                ui.label(aurora_i18n::tr!("characters-delete-question", name = name.as_str()));
                ui.horizontal(|ui| {
                    if widgets::action_button(
                        ui,
                        p,
                        "trash-filled",
                        aurora_i18n::tr!("characters-delete"),
                        ActionKind::Danger,
                        Vec2::new(0.0, 28.0),
                    )
                    .clicked()
                    {
                        answer = Some(true);
                    }
                    if ui.button(aurora_i18n::tr!("common-cancel")).clicked() {
                        answer = Some(false);
                    }
                });
            });
            if answer.is_some() || modal.should_close() {
                self.confirm_delete = None;
                if answer == Some(true) {
                    choice = Some(Choice::Request(Action::Delete { id }));
                }
            }
        }
        choice
    }

    /// The character's region is not accepting people yet: wait for it, or
    /// pick a region that is up.
    fn waiting_panel(&mut self, ui: &mut egui::Ui, p: &Palette, busy: bool) -> Option<Choice> {
        let mut choice = None;
        let Some(waiting) = &self.waiting else {
            return None;
        };
        let auto = waiting.auto;
        ui.label(RichText::new(&waiting.name).size(17.0).strong().color(p.ink));
        ui.label(
            RichText::new(aurora_i18n::tr!(
                "characters-not-ready",
                region = waiting.destination.region.as_str()
            ))
            .color(p.warn),
        );
        if auto {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(aurora_i18n::tr!("characters-waiting"));
            });
        }
        let regions_loading = self.regions_pending.is_some();
        let (mut start_wait, mut back) = (false, false);
        ui.horizontal(|ui| {
            if !auto
                && widgets::action_button_if(
                    ui,
                    p,
                    "clock",
                    aurora_i18n::tr!("characters-wait"),
                    ActionKind::Primary,
                    Vec2::new(150.0, 32.0),
                    !busy,
                )
                .clicked()
            {
                start_wait = true;
            }
            if widgets::action_button_if(
                ui,
                p,
                "map-pin-filled",
                aurora_i18n::tr!("characters-elsewhere"),
                ActionKind::Flat,
                Vec2::new(190.0, 32.0),
                !regions_loading,
            )
            .clicked()
            {
                choice = Some(Choice::LoadRegions);
            }
            if widgets::action_button(ui, p, "arrow-left", aurora_i18n::tr!("characters-back"), ActionKind::Flat, Vec2::new(110.0, 32.0))
                .clicked()
            {
                back = true;
            }
        });
        if start_wait && let Some(waiting) = &mut self.waiting {
            waiting.auto = true;
            waiting.asked_at = std::time::Instant::now();
        }
        if back {
            self.waiting = None;
            self.regions = None;
            return None;
        }
        if self.regions_pending.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(aurora_i18n::tr!("characters-loading-regions"));
            });
        }
        if let Some(regions) = &self.regions {
            widgets::divider(ui, p);
            if regions.is_empty() {
                ui.label(RichText::new(aurora_i18n::tr!("characters-no-regions")).color(p.muted));
            }
            egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
                for region in regions {
                    if ui
                        .add_sized(Vec2::new(ui.available_width(), 30.0), egui::Button::new(&region.name))
                        .clicked()
                    {
                        // the region's middle; the region raises an arrival
                        // below its ground onto it
                        choice = Some(Choice::Go(Destination {
                            kind: "elsewhere".into(),
                            region: region.name.clone(),
                            position: [128.0, 128.0, 30.0],
                            look_at: [1.0, 0.0, 0.0],
                            ready: true,
                            ..Default::default()
                        }));
                    }
                }
            });
        }
        self.error_line(ui, p);
        choice
    }

    /// Play a character now (after its look was saved): its destination
    /// comes as for a Play pressed here.
    pub fn play(&mut self, id: String, name: String) -> Choice {
        self.playing = Some((id.clone(), name));
        Choice::Request(Action::Play { id })
    }

    /// The Wait or Go answer came: the window closes once the app has sent
    /// the avatar on.
    pub fn sent(&mut self) {
        self.waiting = None;
        self.playing = None;
        self.regions = None;
    }

    fn error_line(&self, ui: &mut egui::Ui, p: &Palette) {
        if let Some(error) = &self.error {
            ui.label(RichText::new(error).color(p.danger));
        }
    }
}
