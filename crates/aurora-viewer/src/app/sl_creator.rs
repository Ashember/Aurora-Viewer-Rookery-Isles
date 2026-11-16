//! The SL Character Creator in the app (2026-11-14): opened from the Gate's
//! character select (a Play on a character not made yet, or its Appearance
//! button) when the MakeHuman body is off (`Settings::rookery_makehuman_body`),
//! shown on the Avatar Studio's stage over our own Second Life avatar.
//! The creator (`sl_creator::Creator`) decides; this file only carries its
//! commands to the grid and the answers back, and opens and closes it.

use super::App;
use crate::sl_creator::{Creator, Part, SlLook, Step};
use crate::ui::character_creator::Framing;
use crate::ui::sl_creator::Choice;
use aurora_net::NetCommand;

impl App {
    /// Opens the creator on a character of the select (`id`, `name`).
    pub(super) fn open_sl_creator_for(&mut self, id: String, name: String) {
        let roster = self.login_form.characters.as_ref().and_then(|select| select.roster.as_ref());
        let (saved, has_look) = crate::sl_creator::saved_of(roster, &id);
        self.open_sl_creator(id, name, saved, has_look);
    }

    /// Opens the creator: `saved`, the character's look made here;
    /// `has_look`, whether it has any look yet.
    pub(super) fn open_sl_creator(&mut self, id: String, name: String, saved: Option<SlLook>, has_look: bool) {
        // the creator edits the worn shape, skin and eyes itself: the
        // appearance editors close first (their own question when changed)
        if !self.close_wearable_editor() || !self.close_shape_editor_for_physics() || !self.close_physics_editor_for_shape() {
            if let Some(select) = &mut self.login_form.characters {
                select.error = Some(aurora_i18n::tr!("slcreator-other-editor").to_owned());
            }
            return;
        }
        log::info!("SL character creator: {} ({})", name, if saved.is_some() { "editing" } else { "new" });
        let mut creator = Creator::new(id, name, saved, has_look);
        if self.demo {
            crate::sl_creator::seed_demo_library(&mut self.world.inventory);
            creator.set_catalogue(crate::sl_creator::catalogue::demo());
        } else {
            let endpoint = crate::sl_creator::catalogue::endpoint(&self.settings.login_uri());
            creator.fetch_catalogue(endpoint, &self.net.runtime().clone(), &self.egui_ctx);
        }
        // a clear stage: the floating windows close while it is open
        creator.saved_panels = Some(self.panels.clone());
        let p = &mut self.panels;
        for open in [
            &mut p.chat,
            &mut p.people,
            &mut p.inventory,
            &mut p.appearance,
            &mut p.minimap,
            &mut p.ao,
            &mut p.world_map,
            &mut p.settings,
            &mut p.contacts,
            &mut p.gestures,
            &mut p.snapshot,
        ] {
            *open = false;
        }
        p.avatar_studio = true;
        self.sl_creator = Some(creator);
    }

    /// The creator closed: the avatar drawn as worn, the windows it closed
    /// open again.
    fn close_sl_creator(&mut self) {
        let Some(mut creator) = self.sl_creator.take() else {
            return;
        };
        creator.close(&mut self.world);
        // the body shows its bakes as worn again
        self.sl_creator_bakes.cancel(&mut self.world);
        if let Some(saved) = creator.saved_panels.take() {
            self.panels = saved;
        }
        self.panels.avatar_studio = false;
    }

    /// Whether an outfit change of the app is under way.
    fn sl_creator_busy(&self) -> bool {
        self.appearance_ui.pending.is_some() || !self.inventory_ui.wear_copies.is_empty()
    }

    /// One frame of the open creator: its reads, its queue, its save, its
    /// window. `endpoint`: the character select's address.
    pub(super) fn sl_creator_frame(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, endpoint: String) {
        if self.sl_creator.is_none() {
            return;
        }
        // the stage stays up while the creator is open
        if !self.panels.avatar_studio && !self.world.teleporting {
            self.panels.avatar_studio = true;
        }
        let busy = self.sl_creator_busy();
        let commands = self.sl_creator.as_mut().map(|c| c.poll(&self.world, busy)).unwrap_or_default();
        for command in commands {
            self.sl_creator_send(command);
        }
        let busy = self.sl_creator_busy();
        if let Some(step) = self.sl_creator.as_mut().and_then(|c| c.next_job(&self.world, busy)) {
            self.sl_creator_step(step, &endpoint);
        }
        let busy = self.sl_creator_busy();
        let first_time = self.sl_creator.as_ref().is_some_and(|c| c.mandatory);
        if let Some(step) = self.sl_creator.as_mut().and_then(|c| c.save_step(&mut self.world, busy))
            && self.sl_creator_step(step, &endpoint)
        {
            self.sl_creator_saved(first_time, endpoint);
            return;
        }
        let Some(mut creator) = self.sl_creator.take() else {
            return;
        };
        let busy = self.sl_creator_busy();
        let height = creator.height(&self.world);
        let choice = crate::ui::sl_creator::show(ctx, p, &mut creator, &self.ui_images, height, busy);
        let dragging = ctx.dragged_id().is_some() || ctx.input(|i| i.pointer.any_down());
        creator.end_frame(&mut self.world, dragging);
        // the skin and eyes composed onto the body here, as the appearance
        // editor shows its edit (2026-11-14: they showed only once the bake
        // had gone to the region and back, and the skin colour never)
        if creator.take_bakes_stale() {
            self.sl_creator_bakes.invalidate();
        }
        let drafts = creator.bake_drafts();
        let bake_reads = if drafts.is_empty() {
            Vec::new()
        } else {
            self.sl_creator_bakes.update_drafts(&self.world, &mut self.scene, &drafts)
        };
        self.sl_creator = Some(creator);
        for command in bake_reads {
            self.sl_creator_send(command);
        }
        match choice {
            Some(Choice::Save) => {
                if let Some(creator) = &mut self.sl_creator {
                    creator.start_save();
                }
            }
            Some(Choice::Cancel) => {
                let revert = self.sl_creator.as_mut().and_then(|c| c.cancel(&mut self.world));
                self.close_sl_creator();
                if let Some(action) = revert {
                    self.apply_appearance_action(action);
                }
            }
            Some(Choice::Frame(Framing::Face)) => self.avatar_studio.frame_face(),
            Some(Choice::Frame(Framing::Body)) => self.avatar_studio.frame_body(),
            Some(Choice::RetryCatalogue) => {
                let endpoint = crate::sl_creator::catalogue::endpoint(&self.settings.login_uri());
                let runtime = self.net.runtime().clone();
                if let Some(creator) = &mut self.sl_creator {
                    creator.fetch_catalogue(endpoint, &runtime, &self.egui_ctx);
                }
            }
            None => {}
        }
    }

    /// Carries one step of the creator; true when it is saved.
    fn sl_creator_step(&mut self, step: Step, endpoint: &str) -> bool {
        match step {
            Step::Send(command) => self.sl_creator_send(command),
            Step::Appearance(action) => {
                self.appearance_ui.message.clear();
                self.apply_appearance_action(action);
                let message = self.appearance_ui.message.clone();
                if !message.is_empty()
                    && let Some(creator) = &mut self.sl_creator
                {
                    creator.refused(message);
                }
            }
            Step::SaveLook(look) => self.sl_creator_save_look(look, endpoint),
            Step::Done => return true,
        }
        false
    }

    /// A command of the creator to the grid (the offline demo answers the
    /// reads and saves itself, as the shape editor's demo does).
    fn sl_creator_send(&mut self, command: NetCommand) {
        if !self.demo {
            self.send(command);
            return;
        }
        match command {
            NetCommand::PreviewInventoryItem(item) => {
                // the appearance editor's demo wearables: their hair and eye
                // textures are the demo's own, so the body's bakes can be
                // composed offline (the grid's default ones never load here)
                let data = self
                    .demo_documents
                    .get(&item.id)
                    .cloned()
                    .map(Ok)
                    .unwrap_or_else(|| super::wearable_editor::demo_asset(&item, self.world.agent_id));
                self.sl_creator_loaded(item.id, item.asset_id, &data);
            }
            command @ NetCommand::SaveWearable { .. } => self.demo_save_wearable(command),
            command => self.send(command),
        }
    }

    /// The look marker sent to the grid (`characters::Action::SaveLook`).
    fn sl_creator_save_look(&mut self, look: serde_json::Value, endpoint: &str) {
        let Some(id) = self.sl_creator.as_ref().map(|c| c.character_id.clone()) else {
            return;
        };
        let (tx, rx) = crossbeam_channel::bounded(1);
        if self.demo {
            // the offline demo has no grid: the look is only logged
            log::info!("demo SL character creator: look {look} kept offline");
            let _ = tx.send(Ok(crate::characters::Roster::default()));
        } else if let Some(login) = self.world.login.clone() {
            let auth = crate::characters::Auth::Session {
                session: login.session_id.to_string(),
                secure: login.secure_session_id.to_string(),
            };
            let body = crate::characters::body(&auth, &crate::characters::Action::SaveLook { id, look });
            let endpoint = endpoint.to_owned();
            let repaint = self.egui_ctx.clone();
            self.net.runtime().spawn(async move {
                let _ = tx.send(crate::characters::call(endpoint, body).await);
                repaint.request_repaint();
            });
        } else {
            let _ = tx.send(Err(aurora_i18n::tr!("characters-error-network").to_owned()));
        }
        if let Some(creator) = &mut self.sl_creator {
            creator.saving_look = Some(rx);
        }
    }

    /// Saved: a character made for the first time goes on into the world;
    /// one changed later goes back to the select.
    fn sl_creator_saved(&mut self, first_time: bool, endpoint: String) {
        let Some((id, name)) = self.sl_creator.as_ref().map(|c| (c.character_id.clone(), c.name.clone())) else {
            return;
        };
        log::info!("SL character creator: {name} saved");
        self.close_sl_creator();
        if self.demo {
            return;
        }
        if first_time {
            if let Some(select) = &mut self.login_form.characters
                && let crate::ui::characters::Choice::Request(action) = select.play(id, name)
            {
                self.character_request(endpoint, action);
            }
        } else {
            if let Some(select) = &mut self.login_form.characters {
                select.pending = None;
            }
            self.character_request(endpoint, crate::characters::Action::List);
        }
    }

    /// A worn part's asset for the creator; true when it asked for it.
    pub(super) fn sl_creator_loaded(&mut self, item: uuid::Uuid, asset: uuid::Uuid, result: &Result<Vec<u8>, String>) -> bool {
        // a worn wearable the body's bakes are composed of
        let for_bakes = self.sl_creator_bakes.wants(item, asset);
        self.sl_creator_bakes.loaded(item, asset, result);
        let for_creator = match &mut self.sl_creator {
            Some(creator) => creator.loaded(&self.world, item, asset, result),
            None => false,
        };
        for_bakes || for_creator
    }

    /// A Library copy answered: taken by the creator when it asked for it
    /// (None), else given back.
    pub(super) fn sl_creator_copied(&mut self, request: uuid::Uuid, result: Result<aurora_net::inventory::InvItem, String>) -> Option<Result<aurora_net::inventory::InvItem, String>> {
        match &mut self.sl_creator {
            Some(creator) if creator.owns_copy(request) => {
                creator.copied(&mut self.world, request, result);
                None
            }
            _ => Some(result),
        }
    }

    /// A wearable save answered: taken by the creator when it was its own
    /// (None), else given back.
    pub(super) fn sl_creator_wearable_saved(&mut self, request: uuid::Uuid, result: Result<aurora_net::inventory::InvItem, String>) -> Option<Result<aurora_net::inventory::InvItem, String>> {
        match &mut self.sl_creator {
            Some(creator) if creator.owns_save(request) => {
                creator.wearable_saved(&mut self.world, request, result);
                None
            }
            _ => Some(result),
        }
    }

    /// The offline demo (docs/TESTING.md, `AURORA_DEMO_SL_CREATOR`): the
    /// creator on a new character at frame 600 with the demo catalogue;
    /// `shaped` moves several sliders at frame 800, `male` switches to the
    /// male body at frame 800, `skin` recolours the skin at frame 800,
    /// `face` frames the face at frame 900.
    pub(super) fn demo_sl_creator(&mut self, ctx: &egui::Context, p: &crate::theme::Palette) {
        let modes = std::env::var("AURORA_DEMO_SL_CREATOR").unwrap_or_default();
        let has = |mode: &str| modes.split(',').any(|m| m.trim() == mode);
        if self.frame_count == 600 && self.sl_creator.is_none() {
            if crate::shape_editor::worn_shape(&self.world).is_none() {
                crate::world::appearance::seed_demo(&mut self.world.inventory, self.world.agent_id);
            }
            self.panels.perf = false;
            self.open_sl_creator("demo".into(), "Demo Character".into(), None, false);
        }
        if self.frame_count == 800 {
            if has("male")
                && let Some(creator) = &mut self.sl_creator
            {
                creator.set_sex(crate::sl_creator::catalogue::Sex::Male);
                log::info!("demo SL character creator: male body");
            }
            if has("shaped")
                && let Some(creator) = &mut self.sl_creator
            {
                let values: Vec<(Part, i32, f32)> = crate::sl_creator::DEMO_SHAPED.iter().map(|(id, v)| (Part::Shape, *id, *v)).collect();
                creator.set_many(&mut self.world, &values);
                log::info!("demo SL character creator: shaped {values:?}");
            }
            if has("skin")
                && let Some(creator) = &mut self.sl_creator
            {
                creator.set_many(&mut self.world, &crate::sl_creator::DEMO_SKIN);
                log::info!("demo SL character creator: skin colour {:?}", crate::sl_creator::DEMO_SKIN);
            }
        }
        if self.frame_count == 900 && has("face") && self.sl_creator.is_some() {
            self.avatar_studio.frame_face();
        }
        if self.sl_creator.is_some() {
            self.sl_creator_frame(ctx, p, "http://127.0.0.1:9/rookery/characters".to_owned());
        }
    }
}
