//! The Rookery Starter catalogue: what the SL Character Creator's content
//! browser shows. The grid serves it at GET /rookery/starter beside the
//! character select (`crate::characters::endpoint`'s service): categories
//! and items, each item a real inventory item of the shared Library's
//! "Rookery Starter" folder:
//!
//! ```json
//! {"categories": [{"id": "body", "name": "Body"}, "Hair"],
//!  "items": [{"id": "<inventory item id>", "name": "Ruth2 v4 Body",
//!             "assetType": 6, "invType": 18, "category": "body",
//!             "sex": "female", "wearableType": 0, "thumbnail": "<asset id>"}]}
//! ```
//!
//! The reader is tolerant: unknown fields are ignored, a field of the wrong
//! type counts as absent, an item without a usable id is left out, the
//! categories may be names or objects, and without a category list the
//! items' own categories make one. A grid without the endpoint (404) gives
//! an empty browser with its own message, never an error.

use serde_json::Value;
use uuid::Uuid;

/// The grid's path for the starter catalogue, beside the character select.
pub const PATH: &str = "/rookery/starter";

/// Which body an item is for: the Ruth2 (female) and Roth2 (male) starter
/// bodies, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sex {
    Female,
    Male,
    #[default]
    Any,
}

impl Sex {
    /// The catalogue's word for it ("female", "male"; anything else is both).
    pub fn parse(text: &str) -> Sex {
        match text.trim().to_ascii_lowercase().as_str() {
            "female" | "f" | "woman" | "ruth" | "ruth2" => Sex::Female,
            "male" | "m" | "man" | "roth" | "roth2" => Sex::Male,
            _ => Sex::Any,
        }
    }

    /// Whether an item for this body is shown and worn on `body` (a
    /// female or male body; an item for both fits either).
    pub fn fits(self, body: Sex) -> bool {
        self == Sex::Any || body == Sex::Any || self == body
    }
}

/// One shelf of the content browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    /// What the items name it by (its id, or its name when it has none).
    pub id: String,
    pub name: String,
    /// The starter mesh bodies: switching between female and male swaps
    /// the items of these shelves.
    pub body: bool,
    /// Items worn together (jewelry, accessories) rather than one at a
    /// time in place of each other.
    pub stacking: bool,
}

/// One item of the catalogue: a Library inventory item.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: Uuid,
    pub name: String,
    /// The inventory asset type (13 body part, 5 clothing, 6 object, ...);
    /// -1 when the catalogue did not say.
    pub asset_type: i32,
    /// The inventory type; -1 when the catalogue did not say.
    pub inv_type: i32,
    /// The id of its category ("" when it named none).
    pub category: String,
    pub sex: Sex,
    /// The wearable type of a body part or clothing (0 shape, 1 skin, 3 eyes, ...).
    pub wearable_type: Option<u8>,
    /// Its picture's texture asset.
    pub thumbnail: Option<Uuid>,
    /// Who made it, shown under its name ("" when not given).
    pub credit: String,
    /// Worn alongside what is already worn instead of replacing it.
    pub add: bool,
    /// Worn on a new character before anything is chosen.
    pub default: bool,
}

/// The whole catalogue, categories in the grid's order.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Catalogue {
    pub categories: Vec<Category>,
    pub items: Vec<Item>,
}

/// Names a category's shelf of starter bodies by.
const BODY_NAMES: [&str; 4] = ["body", "bodies", "mesh body", "mesh bodies"];
/// Names a stacking category goes by when the catalogue does not say.
const STACKING_NAMES: [&str; 5] = ["jewelry", "jewellery", "accessories", "piercings", "tattoos"];

fn text(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

fn number(value: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        let field = value.get(*key)?;
        field.as_i64().or_else(|| field.as_str().and_then(|s| s.trim().parse().ok()))
    })
}

fn flag(value: &Value, key: &str) -> Option<bool> {
    let field = value.get(key)?;
    field.as_bool().or_else(|| match field.as_str()?.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" => Some(true),
        "false" | "no" | "0" => Some(false),
        _ => None,
    })
}

fn uuid(value: &Value, keys: &[&str]) -> Option<Uuid> {
    text(value, keys).and_then(|s| Uuid::parse_str(&s).ok()).filter(|id| !id.is_nil())
}

fn category_of(value: &Value) -> Option<Category> {
    let (id, name, body, stacking) = match value {
        Value::String(name) => {
            let name = name.trim().to_owned();
            (name.clone(), name, None, None)
        }
        Value::Object(_) => {
            let name = text(value, &["name", "title", "label"]);
            let id = text(value, &["id", "key", "slug"]).or_else(|| name.clone())?;
            (id.clone(), name.unwrap_or(id), flag(value, "body"), flag(value, "stacking"))
        }
        _ => return None,
    };
    if id.is_empty() {
        return None;
    }
    let lower = [id.to_ascii_lowercase(), name.to_ascii_lowercase()];
    Some(Category {
        body: body.unwrap_or_else(|| lower.iter().any(|n| BODY_NAMES.contains(&n.as_str()))),
        stacking: stacking.unwrap_or_else(|| lower.iter().any(|n| STACKING_NAMES.contains(&n.as_str()))),
        id,
        name,
    })
}

fn item_of(value: &Value) -> Option<Item> {
    let id = uuid(value, &["id", "itemId", "item_id", "item"])?;
    let credit = text(value, &["credit", "credits"]).unwrap_or_else(|| {
        let author = text(value, &["author", "creatorName", "creator", "by"]).unwrap_or_default();
        let licence = text(value, &["licence", "license"]).unwrap_or_default();
        match (author.is_empty(), licence.is_empty()) {
            (false, false) => format!("{author} ({licence})"),
            (false, true) => author,
            (true, false) => licence,
            (true, true) => String::new(),
        }
    });
    Some(Item {
        id,
        name: text(value, &["name", "title"]).unwrap_or_default(),
        asset_type: number(value, &["assetType", "asset_type"]).and_then(|n| i32::try_from(n).ok()).unwrap_or(-1),
        inv_type: number(value, &["invType", "inv_type"]).and_then(|n| i32::try_from(n).ok()).unwrap_or(-1),
        category: text(value, &["category", "categoryId", "shelf"]).unwrap_or_default(),
        sex: text(value, &["sex", "gender", "body"]).map_or(Sex::Any, |s| Sex::parse(&s)),
        wearable_type: number(value, &["wearableType", "wearable_type"]).and_then(|n| u8::try_from(n).ok()),
        thumbnail: uuid(value, &["thumbnail", "thumbnailId", "image"]),
        credit,
        add: flag(value, "add").unwrap_or(false),
        default: flag(value, "default").unwrap_or(false),
    })
}

impl Catalogue {
    /// Reads the grid's answer body (see the module comment).
    pub fn parse(bytes: &[u8]) -> Result<Catalogue, String> {
        let root: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if !root.is_object() {
            return Err(aurora_i18n::tr!("slcreator-catalogue-unreadable").to_owned());
        }
        let mut categories: Vec<Category> = Vec::new();
        for category in root.get("categories").and_then(Value::as_array).into_iter().flatten().filter_map(category_of) {
            if !categories.iter().any(|c| c.id == category.id) {
                categories.push(category);
            }
        }
        let mut items: Vec<Item> = Vec::new();
        for item in root.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(item_of) {
            if !items.iter().any(|i| i.id == item.id) {
                items.push(item);
            }
        }
        // an item may name its category by the category's name, or name one
        // the list left out: the shelf is made for it
        for item in &mut items {
            if let Some(found) = categories
                .iter()
                .find(|c| c.id != item.category && c.name.eq_ignore_ascii_case(&item.category))
            {
                item.category.clone_from(&found.id);
            }
            if !categories.iter().any(|c| c.id == item.category)
                && let Some(category) = category_of(&Value::String(item.category.clone()))
            {
                categories.push(category);
            }
        }
        let mut catalogue = Catalogue { categories, items };
        // an item with no category goes on a shelf of its own at the end
        if catalogue.items.iter().any(|i| i.category.is_empty()) {
            catalogue.categories.push(Category {
                id: String::new(),
                name: aurora_i18n::tr!("slcreator-category-other").to_owned(),
                body: false,
                stacking: false,
            });
        }
        Ok(catalogue)
    }

    pub fn category(&self, id: &str) -> Option<&Category> {
        self.categories.iter().find(|c| c.id == id)
    }

    pub fn item(&self, id: Uuid) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// The items of a shelf that fit a body, in the grid's order.
    pub fn shelf(&self, category: &str, body: Sex) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(move |i| i.category == category && i.sex.fits(body))
    }

    /// The starter body items for a body (female: Ruth2, male: Roth2), by
    /// their shelves and their sex: what switching to that body wears.
    pub fn body_items(&self, body: Sex) -> Vec<&Item> {
        self.items
            .iter()
            .filter(|i| i.sex == body && self.category(&i.category).is_some_and(|c| c.body))
            .collect()
    }

    /// What a new character wears before anything is chosen: the body for
    /// its sex, then the items marked `default` that fit it.
    pub fn defaults(&self, body: Sex) -> Vec<&Item> {
        let mut out = self.body_items(body);
        for item in self.items.iter().filter(|i| i.default && i.sex.fits(body)) {
            if !out.iter().any(|o| o.id == item.id) {
                out.push(item);
            }
        }
        out
    }

    /// Whether wearing this item takes the place of what is worn of its
    /// kind (false: it is added to what is worn).
    pub fn replaces(&self, item: &Item) -> bool {
        !item.add && !self.category(&item.category).is_some_and(|c| c.stacking)
    }
}

/// The catalogue's address for a login address, on the grids that have
/// one (the Rookery Isles grid); None elsewhere.
pub fn endpoint(login_uri: &str) -> Option<String> {
    crate::grids::is_rookery(login_uri).then(|| format!("{}{PATH}", crate::grids::ROOKERY_SERVICE))
}

/// What came of asking the grid for the catalogue.
#[derive(Debug, Clone, PartialEq)]
pub enum Answer {
    Ready(Catalogue),
    /// The grid has no starter catalogue (404): an empty browser.
    Missing,
}

/// The grid's answer read: 200 with a catalogue, 404 for none, else the
/// grid's message or the status.
pub fn answer(status: u16, bytes: &[u8]) -> Result<Answer, String> {
    match status {
        200 => Catalogue::parse(bytes).map(Answer::Ready),
        404 | 410 => Ok(Answer::Missing),
        _ => {
            let message = serde_json::from_slice::<Value>(bytes)
                .ok()
                .and_then(|v| text(&v, &["message", "error"]))
                .unwrap_or_default();
            Err(if message.is_empty() {
                format!("{} (HTTP {status})", aurora_i18n::tr!("slcreator-catalogue-failed"))
            } else {
                message
            })
        }
    }
}

/// Asks the grid for the catalogue.
pub async fn fetch(endpoint: String) -> Result<Answer, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(aurora_version::user_agent())
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .get(&endpoint)
        .send()
        .await
        .map_err(|_| aurora_i18n::tr!("characters-error-network").to_owned())?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| aurora_i18n::tr!("characters-error-network").to_owned())?;
    answer(status, &bytes)
}

/// The offline demo's catalogue (`AURORA_DEMO_SL_CREATOR`): a Ruth2 and a
/// Roth2 body, two skins, eyes, hair and two clothes, with the inventory
/// ids `demo_items` puts in the demo Library.
pub fn demo() -> Catalogue {
    let json = serde_json::json!({
        "categories": [
            {"id": "body", "name": "Body"},
            {"id": "skin", "name": "Skin"},
            {"id": "eyes", "name": "Eyes"},
            {"id": "hair", "name": "Hair"},
            {"id": "clothes", "name": "Clothes"},
            {"id": "jewelry", "name": "Jewelry"}
        ],
        "items": demo_items().iter().map(|d| serde_json::json!({
            "id": d.id.to_string(),
            "name": d.name,
            "assetType": d.asset_type,
            "invType": d.inv_type,
            "category": d.category,
            "sex": d.sex,
            "wearableType": d.wearable_type,
            "credit": d.credit,
            "default": d.default,
        })).collect::<Vec<_>>()
    });
    Catalogue::parse(json.to_string().as_bytes()).unwrap_or_default()
}

/// One item of the demo catalogue, as the demo Library holds it.
pub struct DemoItem {
    pub id: Uuid,
    pub name: &'static str,
    pub asset_type: i32,
    pub inv_type: i32,
    /// The inventory flags: the wearable type, or the attachment point.
    pub flags: u32,
    pub category: &'static str,
    pub sex: &'static str,
    pub wearable_type: Option<u8>,
    pub credit: &'static str,
    pub default: bool,
}

/// The demo catalogue's items (stable ids, so a capture is the same each run).
pub fn demo_items() -> Vec<DemoItem> {
    let id = |n: u128| Uuid::from_u128(0x5100_C2EA_7000_0000_0000_0000_0000_0000 | n);
    let ruth = "RuthAndRoth project";
    vec![
        DemoItem { id: id(1), name: "Ruth2 Body (demo)", asset_type: 6, inv_type: 6, flags: 10, category: "body", sex: "female", wearable_type: None, credit: ruth, default: false },
        DemoItem { id: id(2), name: "Roth2 Body (demo)", asset_type: 6, inv_type: 6, flags: 10, category: "body", sex: "male", wearable_type: None, credit: ruth, default: false },
        DemoItem { id: id(3), name: "Starter Skin Light (demo)", asset_type: 13, inv_type: 18, flags: 1, category: "skin", sex: "any", wearable_type: Some(1), credit: "Rookery Isles", default: true },
        DemoItem { id: id(4), name: "Starter Skin Deep (demo)", asset_type: 13, inv_type: 18, flags: 1, category: "skin", sex: "any", wearable_type: Some(1), credit: "Rookery Isles", default: false },
        DemoItem { id: id(5), name: "Starter Eyes Brown (demo)", asset_type: 13, inv_type: 18, flags: 3, category: "eyes", sex: "any", wearable_type: Some(3), credit: "Rookery Isles", default: true },
        DemoItem { id: id(6), name: "Starter Bob Hair (demo)", asset_type: 6, inv_type: 6, flags: 2, category: "hair", sex: "female", wearable_type: None, credit: "Rookery Isles", default: false },
        DemoItem { id: id(7), name: "Starter Crop Hair (demo)", asset_type: 6, inv_type: 6, flags: 2, category: "hair", sex: "male", wearable_type: None, credit: "Rookery Isles", default: false },
        DemoItem { id: id(8), name: "Starter Tee (demo)", asset_type: 5, inv_type: 18, flags: 4, category: "clothes", sex: "any", wearable_type: Some(4), credit: "Rookery Isles", default: false },
        DemoItem { id: id(9), name: "Starter Jeans (demo)", asset_type: 5, inv_type: 18, flags: 5, category: "clothes", sex: "any", wearable_type: Some(5), credit: "Rookery Isles", default: false },
        DemoItem { id: id(10), name: "Silver Studs (demo)", asset_type: 6, inv_type: 6, flags: 13, category: "jewelry", sex: "any", wearable_type: None, credit: "Rookery Isles", default: false },
    ]
}
