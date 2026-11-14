//! The Rookery Isles Character Creator in the app: opened from the Gate's
//! character select (a Play on a character not made yet, or its Appearance
//! button), shown on the Avatar Studio's stage, its look drawn on this
//! viewer's own avatar as it changes, and saved on the grid. A character
//! made for the first time goes on into the world as soon as it is saved:
//! the creator is the only way from the Gate into the world for it.

use super::App;
use crate::ui::character_creator::{Creator, CreatorChoice, Framing, ThumbSource};

impl App {
    /// Opens the creator on a character (`look`: its saved look, None when
    /// it has not been made yet).
    pub(super) fn open_creator(&mut self, id: String, name: String, look: Option<crate::rookery_body::look::Look>) {
        log::info!("character creator: {} ({})", name, if look.is_some() { "editing" } else { "new" });
        let mut creator = Creator::new(id, name, look);
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
        self.creator = Some(creator);
    }

    /// The creator closed: the windows it closed open again.
    fn close_creator(&mut self, creator: Creator) {
        if let Some(saved) = creator.saved_panels {
            self.panels = saved;
        }
        self.panels.avatar_studio = false;
    }

    /// One frame of the open creator: its window, the avatar drawn with its
    /// look, Save and its answer.
    pub(super) fn creator_frame(&mut self, ctx: &egui::Context, p: &crate::theme::Palette, endpoint: String) {
        let Some(mut creator) = self.creator.take() else {
            return;
        };
        let agent = self.world.agent_id;
        // the stage stays up while the creator is open
        if !self.panels.avatar_studio && !self.world.teleporting {
            self.panels.avatar_studio = true;
        }
        let first_time = creator.mandatory;
        if let Some(Ok(())) = creator.poll_save() {
            log::info!("character creator: {} saved", creator.name);
            self.scene.set_rookery_local_look(agent, None);
            let (id, name, look) = (creator.character_id.clone(), creator.name.clone(), creator.look.clone());
            self.close_creator(creator);
            if first_time {
                // made: it is the character played now, and goes on in
                self.scene.note_rookery_saved(agent, look);
                if let Some(select) = &mut self.login_form.characters
                    && let crate::ui::characters::Choice::Request(action) = select.play(id, name)
                {
                    self.character_request(endpoint, action);
                }
            } else {
                // the look of the character played comes from the grid again
                self.scene.forget_rookery_look(agent);
                if let Some(select) = &mut self.login_form.characters {
                    select.pending = None;
                }
                self.character_request(endpoint, crate::characters::Action::List);
            }
            return;
        }
        let source = match (self.scene.rookery_manifest(), self.scene.rookery_service()) {
            (Some(manifest), Some(service)) => Some(ThumbSource {
                runtime: self.net.runtime().clone(),
                base: crate::rookery_body::fetch::base_url(&service),
                folder: crate::rookery_body::fetch::folder(),
                manifest,
            }),
            _ => None,
        };
        let pack = self.scene.rookery_pack();
        let status = self.scene.rookery_status();
        let height = self.scene.rookery_height(&agent);
        let building = self.scene.rookery_building(&agent);
        let choice = creator.show(ctx, p, pack.as_ref(), &status, source.as_ref(), height, building);
        // the avatar wears what the creator shows
        self.scene.set_rookery_local_look(agent, Some(creator.look.clone()));
        match choice {
            Some(CreatorChoice::Save(look)) => match serde_json::to_value(&look) {
                Ok(value) => {
                    if let Some(login) = self.world.login.clone() {
                        let auth = crate::characters::Auth::Session {
                            session: login.session_id.to_string(),
                            secure: login.secure_session_id.to_string(),
                        };
                        let body = crate::characters::body(
                            &auth,
                            &crate::characters::Action::SaveLook {
                                id: creator.character_id.clone(),
                                look: value,
                            },
                        );
                        let (tx, rx) = crossbeam_channel::bounded(1);
                        let repaint = self.egui_ctx.clone();
                        self.net.runtime().spawn(async move {
                            let _ = tx.send(crate::characters::call(endpoint, body).await);
                            repaint.request_repaint();
                        });
                        creator.saving = Some(rx);
                    }
                }
                Err(error) => creator.error = Some(error.to_string()),
            },
            Some(CreatorChoice::Cancel) => {
                self.scene.set_rookery_local_look(agent, None);
                self.close_creator(creator);
                return;
            }
            Some(CreatorChoice::Frame(Framing::Face)) => self.avatar_studio.frame_face(),
            Some(CreatorChoice::Frame(Framing::Body)) => self.avatar_studio.frame_body(),
            None => {}
        }
        if creator.saving.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        self.creator = Some(creator);
    }
}
