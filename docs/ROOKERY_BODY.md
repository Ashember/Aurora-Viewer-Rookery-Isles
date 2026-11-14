# Rookery body: the MakeHuman system body

Plan and contract for the Rookery Isles Viewer's avatar body (2026-11-13).
Ashley's order: every character is made in a character builder before it may
enter the world; the body the builder makes *is* the system body (no Maitreya,
no Legacy, no skins needed to look good); clothes made for it fit it, follow
its sliders and hide what they cover; any mesh garment imported through the
HomeWorldz importer is fitted and rigged to it automatically.

## Sources (all CC0)

MakeHuman 1.x data on `I:\MakeHuman\github\makehuman\makehuman\data` and MPFB2
on `I:\MakeHuman\github\mpfb2`, fetched by `C:\HomeWorldz\tools\makehuman\fetch_assets.py`:

| Data | File | What it gives |
|---|---|---|
| Base body | `3dobjs/base.obj` | 19,158 vertices: group `body`, helper groups (`helper-tights`, ...) used to fit clothes, and `joint-*` groups whose centres are the joint positions |
| Shape targets | `targets/**/*.target` (1,280, 133 MB) | per-vertex deltas: every slider |
| Slider definitions | `modifiers/*.json` | which targets each slider blends, and how macro sliders (gender, age, weight, muscle, height, proportions) mix |
| Skeleton | `rigs/default.mhskel` | 163 bones, each joint defined by a `joint-*` vertex group, so the skeleton follows any shape |
| Skin weights | `rigs/default_weights.mhw` | per-bone vertex weights, fingers and face included |
| Clothes | `*.mhclo` (1,071 in the packs, 376 with `delete_verts`) | each garment vertex tied to 3 body vertices with weights and an offset (follows the sliders); `delete_verts` hides the body beneath |
| Skins, eyes, eyebrows, hair | asset packs | textures and proxies on the body's UV layout |

## Design

1. **Skeleton: the standard avatar skeleton** (`avatar_skeleton.xml`, 133
   bones with Bento hands and face). Every animation, pose stand, dance ball and
   AO in the OAR builds keeps working; new animations are made for the same
   skeleton. MakeHuman bones map onto it by name (`spine-1` to `mTorso`,
   `finger1-1.L` to `mHandThumb1Left`, ...); its weights are re-targeted to
   those bones.
2. **The body is a rigged mesh the viewer builds itself**, drawn as the
   avatar's body through the viewer's own rigged mesh path (skin binds, joint
   overrides), with the old system body hidden. A shape is: base vertices plus
   the weighted sum of the chosen targets. The joint positions of that shape
   (centres of its `joint-*` groups) become joint position overrides, so the
   skeleton fits the body as MakeHuman's own does.
3. **A look is data, not a mesh**: slider values, skin, eyes, eyebrows, hair
   choices. Stored per character on the grid (`rookery` schema), fetched by
   every Rookery Isles Viewer for each avatar it sees, like display names.
4. **The builder** is the Avatar Studio stage (`avatar_studio.rs`: podium,
   inspection poses) opened in the Rookery Gate: sliders, skin, eyes, hair,
   Save. A character with no saved look cannot Play (grid rule).
5. **Clothes**: MakeHuman garments are fitted by their `.mhclo` ties (they
   follow every slider) and hide the body by their `delete_verts`.
6. **Importer**: a garment uploaded through the HomeWorldz importer
   (meshsmith) is fitted to the base body the way MPFB2's MakeClothes does
   (each vertex tied to its nearest body triangle), takes the body's weights,
   and gets its hidden region computed; it then fits every character.

## Steps

| # | Step | Proof |
|---|---|---|
| 1 | Body asset: `tools/makehuman/build_body.py` turns the MakeHuman data into one compact asset in the viewer's coordinate space (metres, Z up, facing +X), weights re-targeted to the standard skeleton, targets packed | the asset loads; default body stands on the standard skeleton |
| 2 | Viewer draws the body as the avatar (own avatar first), animated | walk, sit, dance, AO, hands and face move |
| 3 | Sliders and looks: builder in the Avatar Studio, looks saved on the grid, Save before Play | a new character must be made; two viewers see each other's bodies |
| 4 | System clothes from the packs: fitting and hiding | jeans follow a heavy and a thin body, no skin through them |
| 5 | Importer: fit, rig and hide any uploaded garment | an uploaded shirt fits three different characters |
| 6 | New animations for the skeleton | |

Firestorm and other viewers see the old system body on these avatars; a
fallback mesh for them is an addition after step 5, not part of the base.

## Where everything is (2026-11-13)

| Part | Where |
|---|---|
| Pack converter | `C:\HomeWorldz\tools\makehuman\rookery_body\pack.py --out <dir> --clothes` (body, 1,095 slider targets, 77 joints, skins, eye colours, brows, lashes, hair, 386 garments from MakeHuman's official clothing packs unpacked on `I:\MakeHuman\unpacked`, each with its licence and author) |
| The pack the grid serves | `C:\HomeWorldz\var\rookery-body\pack` (`[grid] body_pack`), served at `GET /rookery/body/<file>` (`grid/internal/httpapi/rookery_body.go`) |
| Looks on the grid | `rookery.characters.look` (jsonb), `save_look` on `POST /rookery/characters`, `POST /rookery/looks` for the avatars a viewer sees; Play on a character with no look answers `needsLook` |
| Viewer: pack, looks, build | `src/rookery_body/` (`pack`, `look`, `shape`, `build`, `fetch`) |
| Viewer: drawing it | `src/scene/rookery_body.rs`: the pack and looks fetched, one geometry per shape (`GeomKey::RookeryBody`), textures in their own slots, the avatar's skeleton put on the body's joints (`apply_joints`, from `skeleton_of`), drawn in place of the system body (`sync_rookery_avatar`, from `sync_avatar`) |
| Viewer: the Character Creator | `src/ui/character_creator.rs` (content browser, sliders, colours, undo, random, save) and `src/app/character_creator.rs` (opened from the Gate's select, saved on the grid, Play after the first save) |
| Offline check | `AURORA_DEMO=1 AURORA_DEMO_ROOKERY_CREATOR=1 AURORA_ROOKERY_PACK_DIR=<pack>` (docs/TESTING.md) |

Rules the creator keeps: a character looks 18 or older (`MIN_AGE_YEARS`);
one garment per category (a new shirt replaces the shirt); left and right
sliders move together while Symmetry is on.

Not done yet: the grid's own appearance sliders (the standard shape's bone
sliders) are not applied to a Rookery body, its shape comes from the
MakeHuman sliders alone; the garment importer (step 5); new animations
(step 6); a fallback for other viewers.
