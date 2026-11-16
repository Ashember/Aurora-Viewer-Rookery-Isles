use super::catalogue::{Answer, Catalogue, Sex};
use super::*;
use crate::scene::avatar::AvatarLibrary;
use std::sync::Arc;

const NAME: &str = "Ashember Love";

/// A world wearing the demo outfit (a shape, a skin, eyes...), the demo
/// Library's Rookery Starter folder, and a new character's creator with
/// its three drafts read.
fn fixture() -> (World, Creator) {
    let mut world = World::new(Arc::new(AvatarLibrary::load()));
    world.agent_id = Uuid::from_u128(42);
    world.inventory.root = Uuid::from_u128(43);
    crate::world::appearance::seed_demo(&mut world.inventory, world.agent_id);
    seed_demo_library(&mut world.inventory);
    let mut creator = Creator::new("c1".into(), NAME.into(), None, false);
    creator.set_catalogue(catalogue::demo());
    let reads = creator.poll(&world, false);
    assert_eq!(reads.len(), 3, "the worn shape, skin and eyes are read");
    for read in reads {
        let NetCommand::PreviewInventoryItem(item) = read else {
            panic!("a read of a worn part");
        };
        let data = crate::world::inventory::wearable::create((item.flags & 0xff) as u8, &item.name, world.agent_id);
        assert!(creator.loaded(&world, item.id, item.asset_id, &data));
    }
    for part in Part::ALL {
        assert!(creator.draft(part).is_some(), "{part:?} read");
    }
    (world, creator)
}

fn slider(part: Part, id: i32) -> Slider {
    *sliders().iter().find(|s| s.part == part && s.id == id).expect("slider")
}

/// Makes an outfit change as the demo's grid would.
fn apply(world: &mut World, action: crate::world::appearance::Action) {
    let (change, _) = crate::world::appearance::plan(&world.inventory, action, &HashMap::new()).expect("outfit change");
    crate::world::appearance::demo_mutate(&mut world.inventory, world.agent_id, &change).expect("applied");
}

/// The server's copy of a Library item, as `CopyInventoryItem` answers it.
fn copy_of(world: &World, command: &NetCommand) -> (Uuid, InvItem) {
    let NetCommand::CopyInventoryItem { request, copy } = command else {
        panic!("a Library copy, not {command:?}");
    };
    let mut item = world.inventory.items.get(&copy.item).cloned().expect("library item");
    assert!(crate::world::inventory::actions::library(&world.inventory, copy.item), "copied from the Library");
    assert_eq!(copy.owner, world.inventory.lib_owner);
    item.id = Uuid::new_v4();
    item.owner = world.agent_id;
    item.parent = world.inventory.root;
    (*request, item)
}

#[test]
fn a_new_character_puts_on_its_body_and_defaults_one_change_at_a_time() {
    let (mut world, mut creator) = fixture();
    assert!(creator.mandatory);
    assert_eq!(creator.sex, Sex::Female, "the demo shape is female");
    // the defaults wait for the drafts: the next poll queues them
    assert!(creator.poll(&world, false).is_empty());
    let ids: Vec<Uuid> = catalogue::demo_items().iter().map(|d| d.id).collect();
    let (ruth, skin, eyes) = (ids[0], ids[2], ids[4]);
    assert_eq!(creator.pending_wears(), 3, "Ruth2, the default skin, the default eyes");
    // nothing while the outfit is busy
    assert!(creator.next_job(&world, true).is_none());
    // first the body: copied from the Library, then worn (added, so the
    // hair on the skull stays)
    let first = creator.next_job(&world, false).expect("the body's copy");
    let Step::Send(command) = first else { panic!("a copy") };
    let (request, body) = copy_of(&world, &command);
    assert_eq!(world.inventory.items[&ruth].name, body.name);
    assert!(creator.owns_copy(request));
    assert!(creator.next_job(&world, false).is_none(), "one change at a time: the copy is under way");
    creator.copied(&mut world, request, Ok(body.clone()));
    match creator.next_job(&world, false) {
        Some(Step::Appearance(crate::world::appearance::Action::WearItem { item, replace: false, .. })) => assert_eq!(item, body.id),
        other => panic!("the body's copy worn, not {other:?}"),
    }
    // then the skin, then the eyes, in that order
    let Some(Step::Send(command)) = creator.next_job(&world, false) else { panic!("the skin's copy") };
    let (request, copy) = copy_of(&world, &command);
    assert_eq!(copy.name, world.inventory.items[&skin].name);
    creator.copied(&mut world, request, Ok(copy));
    assert!(matches!(
        creator.next_job(&world, false),
        Some(Step::Appearance(crate::world::appearance::Action::WearItem { replace: true, .. }))
    ));
    let Some(Step::Send(command)) = creator.next_job(&world, false) else { panic!("the eyes' copy") };
    let (_, copy) = copy_of(&world, &command);
    assert_eq!(copy.name, world.inventory.items[&eyes].name);
    // chosen tiles keep the order they were clicked in
    let mut later = Creator::new("c2".into(), NAME.into(), None, true);
    later.set_catalogue(catalogue::demo());
    later.choose(ids[5]);
    later.choose(ids[7]);
    later.choose(ids[9]);
    let order: Vec<Uuid> = later
        .queue
        .iter()
        .map(|job| match job {
            Job::Wear { item, .. } | Job::Remove { item } => *item,
        })
        .collect();
    assert_eq!(order, [ids[5], ids[7], ids[9]]);
    // a stacking item clicked again comes off
    later.choose(ids[9]);
    assert_eq!(later.queue.back(), Some(&Job::Remove { item: ids[9] }));
    assert!(!later.mandatory, "a character with a look is not new");
}

#[test]
fn undo_and_redo_step_over_whole_drags() {
    let (mut world, mut creator) = fixture();
    let height = slider(Part::Shape, 33);
    let start = creator.value(&height);
    // one drag over three frames is one step
    for value in [0.5, 1.0, 1.5] {
        creator.begin_frame();
        creator.set(&height, value);
        creator.end_frame(&mut world, true);
    }
    creator.begin_frame();
    creator.end_frame(&mut world, false);
    // a second, separate change
    let pigment = slider(Part::Skin, 111);
    creator.begin_frame();
    creator.set(&pigment, 0.9);
    creator.end_frame(&mut world, false);
    assert_eq!(creator.value(&height), 1.5);
    assert!(world.visual_params(&world.agent_id).is_some(), "the avatar shows the draft");
    creator.undo();
    assert_ne!(creator.value(&pigment), 0.9, "the skin change undone");
    assert_eq!(creator.value(&height), 1.5, "the drag kept");
    creator.undo();
    assert_eq!(creator.value(&height), start, "the whole drag undone at once");
    assert!(!creator.can_undo());
    creator.redo();
    assert_eq!(creator.value(&height), 1.5);
    creator.redo();
    assert_eq!(creator.value(&pigment), 0.9);
    assert!(!creator.can_redo());
    // a new change clears redo
    creator.undo();
    creator.begin_frame();
    creator.set(&height, 0.2);
    creator.end_frame(&mut world, false);
    assert!(!creator.can_redo());
    // Symmetry holds the asymmetric sliders at their default
    let shear = slider(Part::Shape, 662);
    creator.set(&shear, 0.9);
    assert_eq!(creator.value(&shear), shear.default);
    creator.set_symmetric(false);
    creator.set(&shear, 0.9);
    assert_eq!(creator.value(&shear), 0.9);
    creator.set_symmetric(true);
    assert_eq!(creator.value(&shear), shear.default, "Symmetry on centres it again");
    // Random stays inside every slider's range and keeps the sex
    creator.randomize();
    for s in sliders().iter().filter(|s| s.part == Part::Shape) {
        let v = creator.value(s);
        assert!((s.min..=s.max).contains(&v), "{} = {v}", s.id);
    }
    assert_eq!(creator.value(&shear), shear.default);
    assert_eq!(creator.draft(Part::Shape).expect("shape").edited.params[&PARAM_MALE], 0.0);
    // Reset goes back to the values read
    creator.reset();
    assert_eq!(creator.value(&height), start);
    // closing draws the avatar as worn
    creator.close(&mut world);
    assert_eq!(world.visual_params(&world.agent_id), None, "no appearance and no preview");
}

#[test]
fn switching_the_sex_flips_the_male_param_and_swaps_the_body() {
    let (_, mut creator) = fixture();
    let ids: Vec<Uuid> = catalogue::demo_items().iter().map(|d| d.id).collect();
    let (ruth, roth) = (ids[0], ids[1]);
    creator.queue.clear();
    creator.set_sex(Sex::Male);
    assert_eq!(creator.sex, Sex::Male);
    assert_eq!(creator.draft(Part::Shape).expect("shape").edited.params[&PARAM_MALE], 1.0);
    assert_eq!(
        creator.queue.iter().cloned().collect::<Vec<_>>(),
        [
            Job::Remove { item: ruth },
            Job::Wear {
                item: roth,
                replace: false,
                catalogue: Some(roth)
            }
        ]
    );
    assert_eq!(creator.chosen.get("body"), Some(&roth));
    // male-only sliders show, female-only ones hide
    assert!(slider(Part::Shape, 879).fits(Sex::Male));
    assert!(!slider(Part::Shape, 879).fits(Sex::Female));
    assert!(!slider(Part::Shape, 105).fits(Sex::Male));
    // the same sex again changes nothing
    creator.set_sex(Sex::Male);
    assert_eq!(creator.queue.len(), 2);
    // undo flips it back, and the body with it
    creator.undo();
    assert_eq!(creator.sex, Sex::Female);
    assert_eq!(creator.draft(Part::Shape).expect("shape").edited.params[&PARAM_MALE], 0.0);
    assert_eq!(creator.queue.back(), Some(&Job::Wear { item: ruth, replace: false, catalogue: Some(ruth) }));
    // a Roth2 tile on a female body switches the body
    creator.queue.clear();
    creator.choose(roth);
    assert_eq!(creator.sex, Sex::Male);
    // the male body hides the female hair: choosing it does nothing
    let before = creator.queue.len();
    creator.choose(ids[5]);
    assert_eq!(creator.queue.len(), before);
}

#[test]
fn the_look_marker_round_trips_and_is_never_read_as_a_makehuman_body() {
    let look = SlLook {
        outfit: Uuid::from_u128(0xABC),
        shape: Uuid::from_u128(0xDEF),
    };
    let value = look.to_value();
    assert_eq!(value["kind"], "sl");
    assert_eq!(value["version"], 1);
    assert_eq!(value["outfit"], Uuid::from_u128(0xABC).to_string());
    assert_eq!(value["shape"], Uuid::from_u128(0xDEF).to_string());
    assert_eq!(SlLook::read(&value), Some(look));
    let text = serde_json::to_string(&value).expect("json");
    assert_eq!(SlLook::read(&serde_json::from_str(&text).expect("json")), Some(look));
    // not a marker: a MakeHuman look, a newer version, no outfit
    assert_eq!(SlLook::read(&serde_json::json!({"version": 1, "macros": {"gender": 1.0}})), None);
    assert_eq!(SlLook::read(&serde_json::json!({"kind": "sl", "version": 2, "outfit": look.outfit.to_string()})), None);
    assert_eq!(SlLook::read(&serde_json::json!({"kind": "sl", "version": 1})), None);
    assert_eq!(
        SlLook::read(&serde_json::json!({"kind": "sl", "version": 1, "outfit": look.outfit.to_string()})),
        Some(SlLook {
            outfit: look.outfit,
            shape: Uuid::nil()
        }),
        "the shape is optional"
    );
    // the MakeHuman side never takes the marker for its starting body
    assert_eq!(crate::rookery_body::look::Look::from_saved(value.clone()), None);
    assert!(crate::rookery_body::look::Look::from_saved(serde_json::json!({"macros": {"gender": 1.0}})).is_some());
    let character = crate::characters::Character {
        id: "a".into(),
        name: NAME.into(),
        look: Some(value),
        ..Default::default()
    };
    assert_eq!(character.saved_sl_look(), Some(look));
    assert!(character.saved_look().is_none());
    assert!(character.has_look());
    // Play wears its outfit (not with the MakeHuman body on)
    let roster = crate::characters::Roster {
        characters: vec![character],
        ..Default::default()
    };
    let play = crate::characters::Action::Play { id: "a".into() };
    assert_eq!(play_outfit(Some(&roster), &play, false, None), Some(look.outfit));
    assert_eq!(play_outfit(Some(&roster), &play, true, None), None);
    let wait = crate::characters::Action::Destination { id: "a".into() };
    assert_eq!(play_outfit(Some(&roster), &wait, false, Some(look.outfit)), Some(look.outfit), "a wait keeps it");
    assert_eq!(saved_of(Some(&roster), "a"), (Some(look), true));
    assert_eq!(saved_of(Some(&roster), "b"), (None, false));
}

#[test]
fn the_catalogue_reads_tolerantly() {
    let catalogue = Catalogue::parse(
        br#"{
            "version": 7,
            "categories": ["Hair", {"id": "body", "name": "Body"}, {"key": "jewel", "title": "Jewelry"}, 12, {"name": ""}],
            "items": [
                {"id": "11111111-1111-1111-1111-111111111111", "name": "Ruth2 v4", "assetType": 6, "invType": 6, "category": "body", "sex": "female", "thumbnail": "22222222-2222-2222-2222-222222222222", "author": "RuthAndRoth", "licence": "CC-BY", "extra": {"x": 1}},
                {"id": "33333333-3333-3333-3333-333333333333", "name": "Roth2 v2", "category": "Body", "sex": "MALE", "thumbnail": "00000000-0000-0000-0000-000000000000"},
                {"id": "44444444-4444-4444-4444-444444444444", "category": "Hair", "assetType": "6", "wearableType": null},
                {"id": "55555555-5555-5555-5555-555555555555", "name": "Studs", "category": "jewel"},
                {"id": "66666666-6666-6666-6666-666666666666", "name": "Tee", "category": "Clothes", "wearableType": 4, "add": true},
                {"id": "77777777-7777-7777-7777-777777777777", "name": "Loose"},
                {"id": "not a uuid", "name": "Broken"},
                {"name": "No id"},
                {"id": "11111111-1111-1111-1111-111111111111", "name": "Duplicate"}
            ]
        }"#,
    )
    .expect("catalogue");
    let ids: Vec<&str> = catalogue.categories.iter().map(|c| c.id.as_str()).collect();
    assert_eq!(ids, ["Hair", "body", "jewel", "Clothes", ""], "named, keyed, from an item, then the shelf for items without one");
    assert!(catalogue.category("body").is_some_and(|c| c.body && !c.stacking));
    assert!(catalogue.category("jewel").is_some_and(|c| c.stacking && !c.body));
    assert_eq!(catalogue.items.len(), 6, "bad ids and the duplicate left out");
    let ruth = &catalogue.items[0];
    assert_eq!((ruth.asset_type, ruth.inv_type, ruth.sex), (6, 6, Sex::Female));
    assert_eq!(ruth.thumbnail, Some(Uuid::from_u128(0x2222_2222_2222_2222_2222_2222_2222_2222)));
    assert_eq!(ruth.credit, "RuthAndRoth (CC-BY)");
    let roth = &catalogue.items[1];
    assert_eq!((roth.category.as_str(), roth.sex, roth.thumbnail), ("body", Sex::Male, None), "a category by its name; a nil picture is none");
    let hair = &catalogue.items[2];
    assert_eq!((hair.name.as_str(), hair.asset_type, hair.inv_type, hair.sex, hair.wearable_type), ("", 6, -1, Sex::Any, None));
    assert!(!catalogue.replaces(&catalogue.items[3]), "a stacking shelf adds");
    assert!(!catalogue.replaces(&catalogue.items[4]), "an item marked add adds");
    assert!(catalogue.replaces(hair));
    assert_eq!(catalogue.items[5].category, "");
    assert_eq!(catalogue.body_items(Sex::Female).len(), 1);
    assert_eq!(catalogue.body_items(Sex::Male)[0].name, "Roth2 v2");
    assert_eq!(catalogue.shelf("Hair", Sex::Male).count(), 1, "an item for both fits either body");
    // empty, missing and refused answers
    assert_eq!(Catalogue::parse(b"{}").expect("empty"), Catalogue::default());
    assert!(Catalogue::parse(b"[]").is_err());
    assert!(Catalogue::parse(b"not json").is_err());
    assert_eq!(catalogue::answer(404, b"Not Found"), Ok(Answer::Missing));
    assert_eq!(catalogue::answer(500, br#"{"message":"starter folder missing"}"#), Err("starter folder missing".to_owned()));
    assert!(catalogue::answer(502, b"").is_err());
    assert!(matches!(catalogue::answer(200, b"{\"items\":[]}"), Ok(Answer::Ready(c)) if c.items.is_empty()));
    assert_eq!(
        catalogue::endpoint(crate::grids::ROOKERY_LOGIN).as_deref(),
        Some("http://play.rookery.pro:42000/rookery/starter")
    );
    assert_eq!(catalogue::endpoint("http://grid.example:8002/"), None);
    // the demo's catalogue reads like the grid's
    let demo = catalogue::demo();
    assert_eq!(demo.items.len(), catalogue::demo_items().len());
    assert_eq!(demo.body_items(Sex::Female).len(), 1);
    assert_eq!(demo.body_items(Sex::Male).len(), 1);
}

/// The grid's own answer (grid internal/inventory/starter.go, 2026-11-14):
/// `itemId`, `inventoryType`, wearable types by name, keyed categories, and
/// `defaultFor` from its starter defaults file. A body shelf holds variants
/// that overlap when worn together, so only the defaults are the body.
#[test]
fn the_grid_s_catalogue_picks_each_body_s_starter_set() {
    let catalogue = Catalogue::parse(
        br#"{
            "version": "1a2b", "rootFolderId": "00000000-0000-4000-8000-000000000100",
            "categories": [
                {"key": "bodies", "name": "Bodies", "folderId": "65e83604-0659-8d69-afb1-0d069b9e5fcb", "count": 4},
                {"key": "shapes", "name": "Shapes", "count": 2},
                {"key": "skins", "name": "Skins", "count": 2},
                {"key": "eyes", "name": "Eyes", "count": 1},
                {"key": "hair", "name": "Hair", "count": 2}
            ],
            "items": [
                {"itemId": "a0000000-0000-4000-8000-000000000001", "name": "Ruth2 v4 (Body+Hands+Head)", "assetType": 6, "inventoryType": 6, "category": "bodies", "sex": "female", "attachmentPoint": 40, "defaultFor": ["female"]},
                {"itemId": "a0000000-0000-4000-8000-000000000002", "name": "Ruth2 v4 Headless (Body+Hands)", "assetType": 6, "inventoryType": 6, "category": "bodies", "sex": "female", "attachmentPoint": 40},
                {"itemId": "a0000000-0000-4000-8000-000000000003", "name": "Roth2 v2 Full (Body+Feet+Hands+Head)", "assetType": 6, "inventoryType": 6, "category": "bodies", "sex": "male", "attachmentPoint": 40, "defaultFor": ["male"]},
                {"itemId": "a0000000-0000-4000-8000-000000000004", "name": "Roth2 v2 Head", "assetType": 6, "inventoryType": 6, "category": "bodies", "sex": "male", "attachmentPoint": 2},
                {"itemId": "a0000000-0000-4000-8000-000000000005", "name": "Ruth2 v4 Basic Shape", "assetType": 13, "inventoryType": 18, "category": "shapes", "sex": "female", "wearableType": "shape", "defaultFor": ["female"]},
                {"itemId": "a0000000-0000-4000-8000-000000000006", "name": "Roth2 v2 Shape Default", "assetType": 13, "inventoryType": 18, "category": "shapes", "sex": "male", "wearableType": "shape", "defaultFor": ["male"]},
                {"itemId": "a0000000-0000-4000-8000-000000000007", "name": "Starlight Medium", "assetType": 13, "inventoryType": 18, "category": "skins", "sex": "female", "wearableType": "skin", "defaultFor": ["female"], "thumbnailAssetId": "b0000000-0000-4000-8000-000000000007"},
                {"itemId": "a0000000-0000-4000-8000-000000000008", "name": "Lf0y Medium", "assetType": 13, "inventoryType": 18, "category": "skins", "sex": "male", "wearableType": "skin", "defaultFor": ["male"]},
                {"itemId": "a0000000-0000-4000-8000-000000000009", "name": "Ruth2 v4 EyeBalls", "assetType": 6, "inventoryType": 6, "category": "eyes", "sex": "any", "attachmentPoint": 16, "defaultFor": ["female"]},
                {"itemId": "a0000000-0000-4000-8000-00000000000a", "name": "Biker Hair Light", "assetType": 6, "inventoryType": 6, "category": "hair", "sex": "any", "attachmentPoint": 11, "defaultFor": ["female"]},
                {"itemId": "a0000000-0000-4000-8000-00000000000b", "name": "Biker Hair Dark", "assetType": 6, "inventoryType": 6, "category": "hair", "sex": "any", "attachmentPoint": 11, "defaultFor": ["male", "elf"]}
            ]
        }"#,
    )
    .expect("the grid's catalogue");
    let item = |n: u128| catalogue.item(Uuid::from_u128(0xa000_0000_0000_4000_8000_0000_0000_0000 + n)).expect("item");
    assert!(catalogue.category("bodies").is_some_and(|c| c.body), "the grid's Bodies shelf is the body");
    assert_eq!((item(5).inv_type, item(5).wearable_type), (18, Some(0)), "inventoryType and a wearable type by name");
    assert_eq!(item(7).wearable_type, Some(1));
    assert_eq!(item(7).thumbnail, Some(Uuid::from_u128(0xb000_0000_0000_4000_8000_0000_0000_0007)));
    assert_eq!(item(11).default_for, vec![Sex::Male], "an unknown body word is left out");
    let names = |items: Vec<&Item>| items.iter().map(|i| i.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(catalogue.body_items(Sex::Female)), ["Ruth2 v4 (Body+Hands+Head)"], "the defaults, not every variant");
    assert_eq!(names(catalogue.body_items(Sex::Male)), ["Roth2 v2 Full (Body+Feet+Hands+Head)"]);
    assert_eq!(
        names(catalogue.defaults(Sex::Female)),
        ["Ruth2 v4 (Body+Hands+Head)", "Ruth2 v4 Basic Shape", "Starlight Medium", "Ruth2 v4 EyeBalls", "Biker Hair Light"],
        "an item for both bodies is a default only for the body it names"
    );
    assert_eq!(
        names(catalogue.defaults(Sex::Male)),
        ["Roth2 v2 Full (Body+Feet+Hands+Head)", "Roth2 v2 Shape Default", "Lf0y Medium", "Biker Hair Dark"]
    );
    // without any defaults marked, a body shelf still gives a body
    let unmarked = Catalogue::parse(
        br#"{"categories": [{"key": "bodies", "name": "Bodies"}],
             "items": [{"itemId": "a0000000-0000-4000-8000-000000000001", "name": "Body", "category": "bodies", "sex": "female"}]}"#,
    )
    .expect("unmarked");
    assert_eq!(unmarked.body_items(Sex::Female).len(), 1);
}

#[test]
fn save_copies_the_shape_saves_the_outfit_then_the_look() {
    let (mut world, mut creator) = fixture();
    creator.queue.clear();
    creator.defaults_worn = true;
    let height = slider(Part::Shape, 33);
    creator.begin_frame();
    creator.set(&height, 1.25);
    creator.end_frame(&mut world, false);
    let worn_before = crate::shape_editor::worn_shape(&world).expect("shape");
    creator.start_save();
    assert!(creator.saving());
    // the shape: a copy named after the character
    let Some(Step::Send(NetCommand::SaveWearable { request, item, copy_name, data })) = creator.save_step(&mut world, false) else {
        panic!("the shape's save");
    };
    assert_eq!(item.id, worn_before);
    assert_eq!(copy_name.as_deref(), Some("Ashember Love Shape"));
    assert!(creator.owns_save(request));
    assert!(creator.save_step(&mut world, false).is_none(), "waiting for the save");
    let mut copy = (*item).clone();
    copy.id = Uuid::new_v4();
    copy.name = "Ashember Love Shape".into();
    copy.asset_id = Uuid::new_v4();
    let saved = Wearable::parse(&data).expect("saved shape");
    assert_eq!(saved.params[&33], 1.25);
    creator.wearable_saved(&mut world, request, Ok(copy.clone()));
    // the skin and eyes were not changed: nothing to save; then the copy
    // is worn before the outfit is saved
    for _ in 0..3 {
        assert!(creator.save_step(&mut world, false).is_none());
    }
    match creator.next_job(&world, false) {
        Some(Step::Appearance(action @ crate::world::appearance::Action::WearItem { item, replace: true, .. })) => {
            assert_eq!(item, copy.id);
            apply(&mut world, action);
        }
        other => panic!("the shape copy worn, not {other:?}"),
    }
    assert_eq!(crate::shape_editor::worn_shape(&world), Some(copy.id));
    assert!(creator.save_step(&mut world, true).is_none(), "the outfit waits while busy");
    assert!(creator.save_step(&mut world, false).is_none(), "settled");
    let Some(Step::Appearance(action)) = creator.save_step(&mut world, false) else {
        panic!("the outfit's save");
    };
    assert!(matches!(&action, crate::world::appearance::Action::Save(Some(name)) if name == NAME));
    apply(&mut world, action);
    let Some(Step::SaveLook(look)) = ({
        assert!(creator.save_step(&mut world, false).is_none(), "the outfit found");
        creator.save_step(&mut world, false)
    }) else {
        panic!("the look's save");
    };
    let marker = SlLook::read(&look).expect("marker");
    let outfit = world.inventory.folders.get(&marker.outfit).expect("outfit folder");
    assert_eq!(outfit.info.name, NAME);
    assert_eq!(outfit.info.type_default, crate::world::appearance::FT_OUTFIT);
    assert_eq!(marker.shape, copy.id);
    let (tx, rx) = crossbeam_channel::bounded(1);
    tx.send(Ok(crate::characters::Roster::default())).expect("answer");
    creator.saving_look = Some(rx);
    assert!(matches!(creator.save_step(&mut world, false), Some(Step::Done)));
    assert!(!creator.saving());
    assert!(!creator.mandatory);
    assert_eq!(creator.saved, Some(marker));
    // saved again: into the same outfit folder, the shape in place
    creator.begin_frame();
    creator.set(&height, 0.5);
    creator.end_frame(&mut world, false);
    creator.start_save();
    let Some(Step::Send(NetCommand::SaveWearable { copy_name, .. })) = creator.save_step(&mut world, false) else {
        panic!("the shape's second save");
    };
    assert_eq!(copy_name, None, "its own shape is saved in place");
}

#[test]
fn a_refused_save_stops_with_its_message() {
    let (mut world, mut creator) = fixture();
    creator.queue.clear();
    let height = slider(Part::Shape, 33);
    creator.begin_frame();
    creator.set(&height, 1.0);
    creator.end_frame(&mut world, false);
    creator.start_save();
    let Some(Step::Send(NetCommand::SaveWearable { request, .. })) = creator.save_step(&mut world, false) else {
        panic!("the shape's save");
    };
    creator.wearable_saved(&mut world, request, Err("denied by the grid".into()));
    assert!(!creator.saving());
    assert_eq!(creator.message, "denied by the grid");
    // a shape that may be neither copied nor changed is not saved
    let shape = crate::shape_editor::worn_shape(&world).expect("shape");
    world.inventory.items.get_mut(&shape).expect("shape").owner_mask = 0;
    creator.start_save();
    assert!(creator.save_step(&mut world, false).is_none());
    assert!(!creator.saving());
    assert_eq!(creator.message, aurora_i18n::tr!("slcreator-save-denied"));
}

/// Rigged mesh garments (2026-11-15): added, never sent to replace their
/// attachment point; each brings its alpha cut and takes the one it replaces
/// off by name, cut and all; clicked again it comes off; a body switch takes
/// off what was fitted to the other body; the alpha cuts have no shelf.
#[test]
fn garments_bring_their_cuts_and_replace_each_other_by_name() {
    let id = |n: u128| Uuid::from_u128(0x6a00_0000_0000_4000_8000_0000_0000_0000 | n);
    let json = serde_json::json!({
        "categories": [
            {"key": "tops", "name": "Tops", "garment": true},
            {"key": "alphas", "name": "Alphas", "hidden": true}
        ],
        "items": [
            {"itemId": id(1).to_string(), "name": "Linen Shirt (Ruth2)", "assetType": 6, "category": "tops",
             "sex": "female", "with": [id(2).to_string()], "credit": "by joepal, CC0"},
            {"itemId": id(2).to_string(), "name": "Linen Shirt (Ruth2) alpha cut", "assetType": 5, "category": "alphas",
             "sex": "female", "wearableType": "alpha"},
            {"itemId": id(3).to_string(), "name": "Wool Sweater (Ruth2)", "assetType": 6, "category": "tops",
             "sex": "female", "with": [id(4).to_string(), id(3).to_string()]},
            {"itemId": id(4).to_string(), "name": "Wool Sweater (Ruth2) alpha cut", "assetType": 5, "category": "alphas",
             "sex": "female", "wearableType": "alpha"}
        ]
    });
    let catalogue = Catalogue::parse(json.to_string().as_bytes()).expect("catalogue");
    let shelves: Vec<&str> = catalogue.shelves(Sex::Female).iter().map(|c| c.id.as_str()).collect();
    assert_eq!(shelves, ["tops"], "the alpha cuts have no shelf");
    assert!(catalogue.category("tops").is_some_and(|c| c.garment && !c.hidden));
    assert_eq!(catalogue.item(id(3)).expect("sweater").with, [id(4)], "an item never brings itself");
    assert_eq!(catalogue.item(id(1)).expect("shirt").credit, "by joepal, CC0");

    let mut creator = Creator::new("c1".into(), NAME.into(), None, true);
    creator.set_catalogue(catalogue);
    let wear = |item: Uuid| Job::Wear { item, replace: false, catalogue: Some(item) };
    creator.choose(id(1));
    assert_eq!(creator.queue.drain(..).collect::<Vec<_>>(), [wear(id(1)), wear(id(2))]);
    creator.choose(id(3));
    assert_eq!(
        creator.queue.drain(..).collect::<Vec<_>>(),
        [Job::Remove { item: id(1) }, Job::Remove { item: id(2) }, wear(id(3)), wear(id(4))]
    );
    creator.choose(id(3));
    assert_eq!(creator.queue.drain(..).collect::<Vec<_>>(), [Job::Remove { item: id(3) }, Job::Remove { item: id(4) }]);
    assert!(creator.chosen.get("tops").is_none());
    // a body switch takes off what was fitted to the other body
    creator.choose(id(1));
    creator.queue.clear();
    creator.set_sex(Sex::Male);
    let jobs: Vec<Job> = creator.queue.drain(..).collect();
    assert!(jobs.contains(&Job::Remove { item: id(1) }) && jobs.contains(&Job::Remove { item: id(2) }), "{jobs:?}");
    assert!(creator.chosen.get("tops").is_none());
}
