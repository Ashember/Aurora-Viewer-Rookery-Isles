//! The SL Character Creator's window (Rookery Isles Viewer, 2026-11-14):
//! the MakeHuman creator's look and layout (`ui::character_creator`) over
//! a standard Second Life avatar. The avatar stands in the middle on the
//! Avatar Studio's stage; on the left the content browser (the Rookery
//! Starter catalogue's shelves, each item a tile with its picture and its
//! maker); on the right every slider of the worn shape in Firestorm's
//! groups, then the skin's and the eyes' colours, with a search, the body
//! (female or male) and Symmetry. Below: undo, redo, random, reset, the
//! camera's face and body framing, and Save.
//!
//! It shows the creator (`sl_creator::Creator`) and hands it the clicks;
//! everything it decides is there.

use crate::sl_creator::catalogue::Sex;
use crate::sl_creator::{CatalogueState, Creator, Group, Part, Slider, sliders};
use crate::theme::Palette;
use crate::ui::character_creator::Framing;
use crate::ui::widgets::{self, ActionKind};
use egui::{Context, RichText, Vec2};
use std::collections::HashMap;
use uuid::Uuid;

/// Width of the two side panels.
const SIDE: f32 = 330.0;
/// Thumbnail tiles.
const TILE: f32 = 88.0;

/// What the window asks of the app this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    Save,
    /// Leave without saving (only when the character already has a look).
    Cancel,
    Frame(Framing),
    /// Ask the grid for the starter catalogue again.
    RetryCatalogue,
}

/// The shape editor's group names, in its order.
fn group_name(group: Group) -> &'static str {
    match group {
        Group::Shape(0) => aurora_i18n::tr!("shape-group-body"),
        Group::Shape(1) => aurora_i18n::tr!("shape-group-head"),
        Group::Shape(2) => aurora_i18n::tr!("shape-group-eyes"),
        Group::Shape(3) => aurora_i18n::tr!("shape-group-ears"),
        Group::Shape(4) => aurora_i18n::tr!("shape-group-nose"),
        Group::Shape(5) => aurora_i18n::tr!("shape-group-mouth"),
        Group::Shape(6) => aurora_i18n::tr!("shape-group-chin"),
        Group::Shape(7) => aurora_i18n::tr!("shape-group-torso"),
        Group::Shape(_) => aurora_i18n::tr!("shape-group-legs"),
        Group::Skin => aurora_i18n::tr!("wearable-group-skin-color"),
        Group::Eyes => aurora_i18n::tr!("slcreator-group-eye-colour"),
    }
}

/// Draws the creator. `images`: the interface images loaded so far (a
/// tile's picture asks for its own through `Creator::wanted_images`);
/// `height`: the shape's height in metres; `busy`: an outfit change under
/// way.
pub fn show(ctx: &Context, p: &Palette, creator: &mut Creator, images: &HashMap<Uuid, egui::TextureHandle>, height: Option<f32>, busy: bool) -> Option<Choice> {
    creator.begin_frame();
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

    // left: the content browser
    egui::Area::new(egui::Id::unique("sl_creator_content"))
        .fixed_pos(screen.min + Vec2::new(12.0, top))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_width(SIDE);
                ui.set_height(side_height);
                if content_panel(ui, p, creator, images, busy) {
                    choice = Some(Choice::RetryCatalogue);
                }
            });
        });

    // right: the sliders
    egui::Area::new(egui::Id::unique("sl_creator_modify"))
        .fixed_pos(egui::pos2(screen.max.x - SIDE - 36.0, screen.min.y + top))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            frame.show(ui, |ui| {
                ui.set_width(SIDE);
                ui.set_height(side_height);
                modify_panel(ui, p, creator, height);
            });
        });

    // below: the actions
    egui::Area::new(egui::Id::unique("sl_creator_actions"))
        .anchor(egui::Align2::CENTER_BOTTOM, Vec2::new(0.0, -12.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            frame.show(ui, |ui| {
                if let Some(action) = action_bar(ui, p, creator, busy) {
                    choice = Some(action);
                }
                if !creator.message.is_empty() {
                    ui.label(RichText::new(&creator.message).color(p.danger));
                }
            });
        });

    if creator.view.confirm_leave {
        let mut answer = None;
        let modal = widgets::Modal::new(egui::Id::unique("sl_creator_leave")).show(ctx, |ui| {
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
            creator.view.confirm_leave = false;
            if answer == Some(true) {
                choice = Some(Choice::Cancel);
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
    if ctx.memory(|m| m.focused().is_none()) && !creator.saving() {
        if undo {
            creator.undo();
        } else if redo {
            creator.redo();
        }
    }
    if busy || creator.saving() || creator.pending_wears() > 0 || matches!(creator.catalogue, CatalogueState::Loading(_)) {
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
    }
    choice
}

/// The bottom bar; its button's choice.
fn action_bar(ui: &mut egui::Ui, p: &Palette, creator: &mut Creator, busy: bool) -> Option<Choice> {
    let mut choice = None;
    let saving = creator.saving();
    let working = busy || creator.pending_wears() > 0;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(&creator.name).size(16.0).strong().color(p.ink));
            let line = if creator.mandatory {
                aurora_i18n::tr!("creator-mandatory")
            } else {
                aurora_i18n::tr!("creator-editing")
            };
            ui.label(RichText::new(line).size(11.0).color(p.muted));
        });
        ui.add_space(12.0);
        let size = Vec2::new(0.0, 32.0);
        let edit = !saving && creator.draft(Part::Shape).is_some();
        if widgets::action_button_if(ui, p, "arrow-back-up", aurora_i18n::tr!("creator-undo"), ActionKind::Flat, size, edit && creator.can_undo()).clicked() {
            creator.undo();
        }
        if widgets::action_button_if(ui, p, "arrow-forward-up", aurora_i18n::tr!("creator-redo"), ActionKind::Flat, size, edit && creator.can_redo()).clicked() {
            creator.redo();
        }
        if widgets::action_button_if(ui, p, "wand", aurora_i18n::tr!("creator-random"), ActionKind::Flat, size, edit)
            .on_hover_text(aurora_i18n::tr!("slcreator-random-hint"))
            .clicked()
        {
            creator.randomize();
        }
        if widgets::action_button_if(ui, p, "refresh", aurora_i18n::tr!("creator-reset"), ActionKind::Flat, size, edit)
            .on_hover_text(aurora_i18n::tr!("slcreator-reset-hint"))
            .clicked()
        {
            creator.reset();
        }
        widgets::divider(ui, p);
        if widgets::action_button(ui, p, "user-circle", aurora_i18n::tr!("creator-frame-face"), ActionKind::Flat, size).clicked() {
            choice = Some(Choice::Frame(Framing::Face));
        }
        if widgets::action_button(ui, p, "man-filled", aurora_i18n::tr!("creator-frame-body"), ActionKind::Flat, size).clicked() {
            choice = Some(Choice::Frame(Framing::Body));
        }
        widgets::divider(ui, p);
        if working && !saving {
            ui.spinner();
        }
        if !creator.mandatory
            && widgets::action_button_if(ui, p, "x", aurora_i18n::tr!("common-cancel"), ActionKind::Flat, size, !saving && !working).clicked()
        {
            if creator.changed() {
                creator.view.confirm_leave = true;
            } else {
                choice = Some(Choice::Cancel);
            }
        }
        let label = if creator.mandatory {
            aurora_i18n::tr!("creator-save-enter")
        } else {
            aurora_i18n::tr!("creator-save")
        };
        if widgets::action_button_if(ui, p, "device-floppy", label, ActionKind::Primary, Vec2::new(0.0, 36.0), !saving && creator.draft(Part::Shape).is_some())
            .clicked()
        {
            choice = Some(Choice::Save);
        }
        if saving {
            ui.spinner();
            ui.label(RichText::new(aurora_i18n::tr!("slcreator-saving")).size(11.0).color(p.muted));
        }
    });
    choice
}

/// The content browser; true when Retry was pressed.
fn content_panel(ui: &mut egui::Ui, p: &Palette, creator: &mut Creator, images: &HashMap<Uuid, egui::TextureHandle>, busy: bool) -> bool {
    ui.label(RichText::new(aurora_i18n::tr!("creator-content")).size(17.0).strong().color(p.ink));
    let catalogue = match &creator.catalogue {
        CatalogueState::Loading(_) => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(aurora_i18n::tr!("slcreator-catalogue-loading"));
            });
            return false;
        }
        CatalogueState::Missing => {
            ui.label(RichText::new(aurora_i18n::tr!("slcreator-catalogue-missing")).color(p.muted));
            return false;
        }
        CatalogueState::Failed(error) => {
            ui.label(RichText::new(aurora_i18n::tr!("slcreator-catalogue-failed")).color(p.danger));
            ui.label(RichText::new(error).size(11.0).color(p.muted));
            return widgets::action_button(ui, p, "refresh", aurora_i18n::tr!("slcreator-catalogue-retry"), ActionKind::Flat, Vec2::new(0.0, 28.0)).clicked();
        }
        CatalogueState::Ready(catalogue) => catalogue.clone(),
    };
    if catalogue.items.is_empty() {
        ui.label(RichText::new(aurora_i18n::tr!("slcreator-catalogue-empty")).color(p.muted));
        return false;
    }
    let sex = creator.sex;
    // the shelves with something for this body (a body shelf shows both
    // bodies: choosing the other one switches)
    let shelves: Vec<(String, String)> = catalogue
        .categories
        .iter()
        .filter(|c| catalogue.items.iter().any(|i| i.category == c.id && (c.body || i.sex.fits(sex))))
        .map(|c| (c.id.clone(), c.name.clone()))
        .collect();
    if creator.view.shelf.as_ref().is_none_or(|s| !shelves.iter().any(|(id, _)| id == s)) {
        creator.view.shelf = shelves.first().map(|(id, _)| id.clone());
    }
    ui.horizontal_wrapped(|ui| {
        for (id, name) in &shelves {
            if ui.selectable_label(creator.view.shelf.as_ref() == Some(id), name).clicked() {
                creator.view.shelf = Some(id.clone());
            }
        }
    });
    widgets::divider(ui, p);
    ui.add(
        egui::TextEdit::singleline(&mut creator.view.content_search)
            .hint_text(aurora_i18n::tr!("slcreator-content-search"))
            .desired_width(f32::INFINITY),
    );
    let pending = creator.pending_wears();
    if pending > 0 || busy {
        let count = pending.max(1).to_string();
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(aurora_i18n::tr!("slcreator-wearing", count = count.as_str())).size(11.0).color(p.muted));
        });
    }
    let shelf = creator.view.shelf.clone().unwrap_or_default();
    let body_shelf = catalogue.category(&shelf).is_some_and(|c| c.body);
    let wanted = creator.view.content_search.trim().to_lowercase();
    // (a body shelf shows both bodies, any other what fits this body)
    let shown: Vec<&crate::sl_creator::catalogue::Item> = if body_shelf {
        catalogue.items.iter().filter(|i| i.category == shelf).collect()
    } else {
        catalogue.shelf(&shelf, sex).collect()
    };
    let tiles: Vec<&crate::sl_creator::catalogue::Item> = shown
        .into_iter()
        .filter(|i| wanted.is_empty() || i.name.to_lowercase().contains(&wanted) || i.credit.to_lowercase().contains(&wanted))
        .collect();
    let mut clicked = None;
    if tiles.is_empty() {
        ui.label(RichText::new(aurora_i18n::tr!("slcreator-shelf-empty")).color(p.muted));
    }
    // A shelf can hold well over a thousand items: only the rows in view are
    // laid out, and only their pictures are asked for (2026-11-14).
    let columns = ((ui.available_width() + 6.0) / (TILE + 6.0)).floor().max(1.0) as usize;
    let rows: Vec<&[&crate::sl_creator::catalogue::Item]> = tiles.chunks(columns).collect();
    let row_height = TILE + 30.0 + ui.spacing().item_spacing.y;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_rows(ui, row_height, rows.len(), |ui, range| {
        for row in &rows[range] {
            ui.horizontal(|ui| {
                for &item in row.iter() {
                    let texture = item.thumbnail.and_then(|thumb| match images.get(&thumb) {
                        Some(texture) => Some(texture.id()),
                        None => {
                            creator.wanted_images.insert(thumb);
                            None
                        }
                    });
                    let name = if item.name.is_empty() { aurora_i18n::tr!("slcreator-unnamed") } else { item.name.as_str() };
                    let sex_mark = match (body_shelf, item.sex) {
                        (true, Sex::Female) => aurora_i18n::tr!("creator-female"),
                        (true, Sex::Male) => aurora_i18n::tr!("creator-male"),
                        _ => "",
                    };
                    if tile(ui, p, texture, name, &item.credit, sex_mark, creator.is_chosen(item)).clicked() {
                        clicked = Some(item.id);
                    }
                }
            });
        }
    });
    if let Some(item) = clicked
        && !creator.saving()
    {
        creator.choose(item);
    }
    false
}

/// The sliders.
fn modify_panel(ui: &mut egui::Ui, p: &Palette, creator: &mut Creator, height: Option<f32>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(aurora_i18n::tr!("creator-modify")).size(17.0).strong().color(p.ink));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let mut symmetric = creator.symmetric;
            if ui
                .checkbox(&mut symmetric, aurora_i18n::tr!("creator-symmetry"))
                .on_hover_text(aurora_i18n::tr!("slcreator-symmetry-hint"))
                .changed()
            {
                creator.set_symmetric(symmetric);
            }
        });
    });
    let Some(_) = creator.draft(Part::Shape) else {
        ui.horizontal(|ui| {
            if creator.loading(Part::Shape) || creator.message.is_empty() {
                ui.spinner();
            }
            ui.label(RichText::new(aurora_i18n::tr!("slcreator-shape-loading")).color(p.muted));
        });
        return;
    };
    // the body: female (Ruth2) or male (Roth2)
    ui.horizontal(|ui| {
        let mut sex = creator.sex;
        widgets::radio_value(ui, p, &mut sex, Sex::Female, aurora_i18n::tr!("creator-female"));
        widgets::radio_value(ui, p, &mut sex, Sex::Male, aurora_i18n::tr!("creator-male"));
        if sex != creator.sex && !creator.saving() {
            creator.set_sex(sex);
        }
    });
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
    ui.add(
        egui::TextEdit::singleline(&mut creator.view.search)
            .hint_text(aurora_i18n::tr!("creator-search"))
            .desired_width(f32::INFINITY),
    );
    widgets::divider(ui, p);
    let search = creator.view.search.trim().to_lowercase();
    let sex = creator.sex;
    let groups: Vec<Group> = (0..9).map(Group::Shape).chain([Group::Skin, Group::Eyes]).collect();
    let saving = creator.saving();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for group in groups {
            let shown: Vec<&Slider> = sliders()
                .iter()
                .filter(|s| s.group == group && s.fits(sex) && creator.draft(s.part).is_some())
                .filter(|s| search.is_empty() || s.labels().0.to_lowercase().contains(&search))
                .collect();
            if shown.is_empty() {
                continue;
            }
            egui::CollapsingHeader::new(RichText::new(group_name(group)).strong())
                .id_salt(("sl_creator_group", format!("{group:?}")))
                .default_open(group == Group::Shape(0))
                .open(if search.is_empty() { None } else { Some(true) })
                .show(ui, |ui| {
                    for slider in shown {
                        let held = creator.symmetric && slider.asymmetric();
                        ui.add_enabled_ui(!held && !saving, |ui| {
                            let mut value = creator.value(slider);
                            let before = value;
                            slider_row(ui, p, slider, &mut value, held);
                            if (value - before).abs() > f32::EPSILON {
                                creator.set(slider, value);
                            }
                        });
                    }
                });
        }
    });
}

/// A labelled slider shown 0 to 100 (Firestorm's scale) with a double
/// click, or the small button, back to its default.
fn slider_row(ui: &mut egui::Ui, p: &Palette, slider: &Slider, value: &mut f32, held: bool) {
    let (label, low, high) = slider.labels();
    let span = (slider.max - slider.min).max(f32::EPSILON);
    ui.horizontal(|ui| {
        let hint = if held {
            aurora_i18n::tr!("slcreator-symmetry-held").to_owned()
        } else {
            format!("{low} / {high}")
        };
        ui.label(RichText::new(label).size(12.0).color(p.ink)).on_hover_text(hint);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .add_enabled((*value - slider.default).abs() > 1e-4, egui::Button::new("↺").small())
                .on_hover_text(aurora_i18n::tr!("creator-default"))
                .clicked()
            {
                *value = slider.default;
            }
            // (shown by a formatter, and written back only when moved: a
            // fixed decimals setting would round the value every frame)
            let mut shown = *value;
            let min = slider.min;
            let response = ui.add(
                egui::Slider::new(&mut shown, slider.min..=slider.max)
                    .show_value(true)
                    .custom_formatter(move |v, _| format!("{:.0}", (v as f32 - min) / span * 100.0))
                    .custom_parser(move |text| text.trim().parse::<f64>().ok().map(|percent| f64::from(min) + percent / 100.0 * f64::from(span))),
            );
            if response.double_clicked() {
                *value = slider.default;
            } else if response.changed() {
                *value = shown;
            }
        });
    });
}

/// A thumbnail tile: its picture, its name and its maker under it.
fn tile(ui: &mut egui::Ui, p: &Palette, texture: Option<egui::TextureId>, name: &str, credit: &str, mark: &str, selected: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(TILE, TILE + 30.0), egui::Sense::click());
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
    if !mark.is_empty() {
        painter.text(image.left_top() + Vec2::new(6.0, 5.0), egui::Align2::LEFT_TOP, mark, egui::FontId::proportional(9.5), p.muted);
    }
    if selected {
        painter.rect_stroke(image, 8.0, egui::Stroke::new(2.5, p.violet_light), egui::StrokeKind::Inside);
    }
    let short = |text: &str, count: usize| {
        let mut out: String = text.chars().take(count).collect();
        if text.chars().count() > count {
            out.push('.');
        }
        out
    };
    painter.text(
        egui::pos2(rect.center().x, image.max.y + 9.0),
        egui::Align2::CENTER_CENTER,
        short(name, 14),
        egui::FontId::proportional(10.5),
        if selected { p.ink } else { p.muted },
    );
    if !credit.is_empty() {
        painter.text(
            egui::pos2(rect.center().x, image.max.y + 22.0),
            egui::Align2::CENTER_CENTER,
            short(credit, 16),
            egui::FontId::proportional(9.0),
            p.muted,
        );
    }
    let hover = if credit.is_empty() { name.to_owned() } else { format!("{name}\n{credit}") };
    response.on_hover_text(hover)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shown frame after frame with nobody touching it, the window changes
    /// no slider value and makes no undo step.
    #[test]
    fn an_untouched_window_keeps_the_values() {
        let theme = crate::theme::Theme::default();
        let palette = theme.palette();
        let mut creator = Creator::new("id".into(), "Ashember Love".into(), None, false);
        creator.set_catalogue(crate::sl_creator::catalogue::demo());
        let ctx = egui::Context::default();
        theme.apply(&ctx, 1.0);
        let images = HashMap::new();
        for _ in 0..4 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
                    ..Default::default()
                },
                |ui| {
                    let ctx = ui.ctx().clone();
                    let _ = show(&ctx, &palette, &mut creator, &images, Some(1.7), false);
                },
            );
            output.textures_delta.clear();
        }
        assert!(!creator.can_undo());
        assert_eq!(creator.pending_wears(), 0, "showing the window wears nothing");
    }
}
