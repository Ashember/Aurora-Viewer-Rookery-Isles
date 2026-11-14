//! Rookery Isles character select (Rookery Isles Viewer).
//!
//! An account on the Rookery Isles grid holds up to three characters, each
//! with its own name, its own world (Star Trek, medieval Rookery Isles, ...)
//! and its own last place. Everything any of them rezzes is owned by the
//! account. Every login lands in the Rookery Gate, a small region that is
//! always up; there the viewer asks the grid for the account's characters
//! (POST /rookery/characters, naming itself by its live session), shows them,
//! and Play sends the avatar on to that character's place. When that region is
//! not accepting people yet, the grid says so and the viewer offers to wait or
//! to go somewhere else (GET /rookery/regions).
//!
//! The grid decides everything (which worlds exist, name rules, the three
//! slots, where a character goes); this module carries the requests and the
//! answers.
//!
//! Nobody enters the world with a character they have not made: Play on a
//! character with no saved look answers `needsLook` instead of a
//! destination, the viewer opens the Character Creator, and its Save
//! (`Action::SaveLook`) is the only way on from the Gate.

use serde::Deserialize;

/// The grid's path for the character select, beside its login.
const PATH: &str = "/rookery/characters";
/// The grid's path for which regions are accepting people.
const REGIONS_PATH: &str = "/rookery/regions";

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    pub id: String,
    pub slot: u8,
    pub name: String,
    #[serde(default)]
    pub active: bool,
    /// The key of the world it began in (empty for one made before worlds).
    #[serde(default)]
    pub world: String,
    /// Where it last was; absent until its first visit ends.
    #[serde(default)]
    pub last_location: Option<serde_json::Value>,
    /// Its body and appearance from the Character Creator
    /// (`rookery_body::look::Look`); absent until it has been made.
    #[serde(default)]
    pub look: Option<serde_json::Value>,
}

impl Character {
    /// Its saved look, read (None: not made yet, or not one this viewer reads).
    pub fn saved_look(&self) -> Option<crate::rookery_body::look::Look> {
        self.look.clone().and_then(|v| serde_json::from_value(v).ok())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct World {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub region: String,
}

/// Where a character goes from the Gate: the region (a facet's name) and a
/// position inside it, and whether that region is accepting people now.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Destination {
    /// "last" (its own last place), "world" (its world's start) or
    /// "welcome" (the grid's arrival point).
    pub kind: String,
    pub region: String,
    pub position: [f32; 3],
    #[serde(default)]
    pub look_at: [f32; 3],
    pub ready: bool,
    /// The region's place on the grid's map, in regions (0: not given):
    /// the teleport goes by it, as the region's name may be a facet's.
    #[serde(default)]
    pub grid_x: u32,
    #[serde(default)]
    pub grid_y: u32,
}

impl Destination {
    pub fn position(&self) -> glam::Vec3 {
        glam::Vec3::from(self.position)
    }

    /// The region handle of its map place (None when the grid gave none).
    pub fn handle(&self) -> Option<u64> {
        (self.grid_x > 0 && self.grid_y > 0).then(|| (u64::from(self.grid_x) * 256) << 32 | u64::from(self.grid_y) * 256)
    }

    /// The facing to arrive with: the stored one, else east.
    pub fn look_at(&self) -> glam::Vec3 {
        let look = glam::Vec3::from(self.look_at);
        if look.x.hypot(look.y) < 0.001 { glam::Vec3::X } else { look }
    }
}

/// The grid's answer: the account and its characters, the worlds a new
/// character can start in, and, for Play, where the character goes.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Roster {
    pub account: String,
    pub max: usize,
    pub characters: Vec<Character>,
    pub worlds: Vec<World>,
    #[serde(default)]
    pub destination: Option<Destination>,
    /// Play on a character not made yet: the Character Creator opens.
    #[serde(default)]
    pub needs_look: bool,
}

impl Roster {
    /// The display name of a world key ("" when unknown).
    pub fn world_name(&self, key: &str) -> &str {
        self.worlds.iter().find(|w| w.key == key).map_or("", |w| w.name.as_str())
    }

    pub fn full(&self) -> bool {
        self.characters.len() >= self.max
    }
}

/// One region of the grid and whether it is accepting people now.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct RegionReadiness {
    pub id: String,
    pub name: String,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    List,
    Create { name: String, world: String },
    Delete { id: String },
    /// Make it the character played, and say where it goes.
    Play { id: String },
    /// Where it would go now, and whether that region is up (while waiting).
    Destination { id: String },
    /// Save the Character Creator's look on the character.
    SaveLook { id: String, look: serde_json::Value },
}

/// Who is asking: a viewer in world names itself by its session and the
/// secure id only it holds; before a login it would name the account and the
/// password hash exactly as the login sends them.
#[derive(Debug, Clone, PartialEq)]
pub enum Auth {
    Session { session: String, secure: String },
    // (the login screen's select, before a session exists; only the tests
    // build one yet)
    #[cfg_attr(not(test), allow(dead_code))]
    Password { username: String, hash: String },
}

/// The character select's address for a login address, on the grids that
/// have one (the Rookery Isles grid); None elsewhere.
pub fn endpoint(login_uri: &str) -> Option<String> {
    crate::grids::is_rookery(login_uri).then(|| format!("{}{PATH}", crate::grids::ROOKERY_SERVICE))
}

/// The address of the grid's region readiness list (Rookery Isles only).
pub fn regions_endpoint(login_uri: &str) -> Option<String> {
    crate::grids::is_rookery(login_uri).then(|| format!("{}{REGIONS_PATH}", crate::grids::ROOKERY_SERVICE))
}

/// The request body for an action.
pub fn body(auth: &Auth, action: &Action) -> serde_json::Value {
    let mut body = match auth {
        Auth::Session { session, secure } => serde_json::json!({ "session": session, "secure": secure }),
        Auth::Password { username, hash } => {
            let (first, last) = aurora_net::login::split_username(username);
            serde_json::json!({ "first": first, "last": last, "passwd": hash })
        }
    };
    match action {
        Action::List => body["action"] = "list".into(),
        Action::Create { name, world } => {
            body["action"] = "create".into();
            body["name"] = name.trim().into();
            body["world"] = world.as_str().into();
        }
        Action::Delete { id } => {
            body["action"] = "delete".into();
            body["id"] = id.as_str().into();
        }
        Action::Play { id } => {
            body["action"] = "play".into();
            body["id"] = id.as_str().into();
        }
        Action::Destination { id } => {
            body["action"] = "destination".into();
            body["id"] = id.as_str().into();
        }
        Action::SaveLook { id, look } => {
            body["action"] = "save_look".into();
            body["id"] = id.as_str().into();
            body["look"] = look.clone();
        }
    }
    body
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .user_agent(aurora_version::user_agent())
        .build()
        .map_err(|e| e.to_string())
}

/// Sends one request and reads the roster, or the grid's message on refusal.
pub async fn call(endpoint: String, body: serde_json::Value) -> Result<Roster, String> {
    let response = client()?
        .post(&endpoint)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .map_err(|_| aurora_i18n::tr!("characters-error-network").to_owned())?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| aurora_i18n::tr!("characters-error-network").to_owned())?;
    answer(status.as_u16(), &bytes)
}

/// The regions of the grid that are accepting people now, by name.
pub async fn ready_regions(endpoint: String) -> Result<Vec<RegionReadiness>, String> {
    let response = client()?
        .get(&endpoint)
        .send()
        .await
        .map_err(|_| aurora_i18n::tr!("characters-error-network").to_owned())?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| aurora_i18n::tr!("characters-error-network").to_owned())?;
    regions_answer(status.as_u16(), &bytes)
}

fn regions_answer(status: u16, bytes: &[u8]) -> Result<Vec<RegionReadiness>, String> {
    #[derive(Deserialize)]
    struct Regions {
        regions: Vec<RegionReadiness>,
    }
    if status != 200 {
        return Err(format!("{} (HTTP {status})", aurora_i18n::tr!("characters-error-answer")));
    }
    let mut regions = serde_json::from_slice::<Regions>(bytes)
        .map_err(|_| aurora_i18n::tr!("characters-error-answer").to_owned())?
        .regions;
    regions.retain(|r| r.ready && !r.name.eq_ignore_ascii_case(crate::grids::ROOKERY_GATE));
    regions.sort_by_key(|r| crate::natural_order::key(&r.name));
    Ok(regions)
}

/// The grid's answer read: a roster on success, else its message.
fn answer(status: u16, bytes: &[u8]) -> Result<Roster, String> {
    if status == 200 {
        return serde_json::from_slice(bytes).map_err(|_| aurora_i18n::tr!("characters-error-answer").to_owned());
    }
    #[derive(Deserialize)]
    struct Refusal {
        #[serde(default)]
        message: String,
    }
    let message = serde_json::from_slice::<Refusal>(bytes).map(|r| r.message).unwrap_or_default();
    Err(if message.trim().is_empty() {
        format!("{} (HTTP {status})", aurora_i18n::tr!("characters-error-answer"))
    } else {
        message
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_rookery_grid_has_a_character_select() {
        assert_eq!(
            endpoint(crate::grids::ROOKERY_LOGIN).as_deref(),
            Some("http://play.rookery.pro:42000/rookery/characters")
        );
        assert_eq!(
            regions_endpoint(crate::grids::ROOKERY_LOGIN).as_deref(),
            Some("http://play.rookery.pro:42000/rookery/regions")
        );
        assert_eq!(endpoint(aurora_net::login::SL_MAIN_GRID), None);
        assert_eq!(endpoint("http://grid.example:8002/"), None);
    }

    #[test]
    fn requests_name_the_viewer_by_session_in_world_and_by_account_before() {
        let session = Auth::Session {
            session: "11111111-1111-1111-1111-111111111111".into(),
            secure: "22222222-2222-2222-2222-222222222222".into(),
        };
        let play = body(&session, &Action::Play { id: "abc".into() });
        assert_eq!(play["session"], "11111111-1111-1111-1111-111111111111");
        assert_eq!(play["secure"], "22222222-2222-2222-2222-222222222222");
        assert_eq!(play["action"], "play");
        assert_eq!(play["id"], "abc");
        assert!(play.get("passwd").is_none());
        let hash = "$1$5f4dcc3b5aa765d61d8327deb882cf99";
        let list = body(
            &Auth::Password {
                username: "Ashley Ember".into(),
                hash: hash.into(),
            },
            &Action::List,
        );
        assert_eq!(list["first"], "Ashley");
        assert_eq!(list["last"], "Ember");
        assert_eq!(list["passwd"], hash);
        let create = body(
            &session,
            &Action::Create {
                name: "  Captain Vale ".into(),
                world: "trek".into(),
            },
        );
        assert_eq!(create["name"], "Captain Vale");
        assert_eq!(create["world"], "trek");
        assert_eq!(body(&session, &Action::Destination { id: "abc".into() })["action"], "destination");
        let save = body(
            &session,
            &Action::SaveLook {
                id: "abc".into(),
                look: serde_json::json!({"version": 1}),
            },
        );
        assert_eq!(save["action"], "save_look");
        assert_eq!(save["look"]["version"], 1);
    }

    #[test]
    fn a_character_not_made_yet_goes_to_the_creator() {
        let roster = answer(
            200,
            br#"{"account":"a","max":3,"characters":[{"id":"a","slot":1,"name":"Ashember Love","active":true,"world":"trek"},{"id":"b","slot":2,"name":"Captain Vale","look":{"version":1,"macros":{"gender":1.0}}}],"worlds":[],"needsLook":true}"#,
        )
        .expect("roster");
        assert!(roster.needs_look);
        assert!(roster.destination.is_none());
        assert!(roster.characters[0].saved_look().is_none());
        let look = roster.characters[1].saved_look().expect("look");
        assert_eq!(look.macros.gender, 1.0);
        let place = Destination {
            grid_x: 1069,
            grid_y: 1002,
            ..Default::default()
        };
        assert_eq!(place.handle(), Some((1069u64 * 256) << 32 | 1002 * 256));
        assert_eq!(Destination::default().handle(), None);
    }

    #[test]
    fn the_answer_is_a_roster_with_its_destination_or_the_grid_s_message() {
        let roster = answer(
            200,
            br#"{"account":"ashley.ember","max":3,"characters":[{"id":"a","slot":1,"name":"Captain Vale","active":true,"world":"trek","createdAt":"2026-11-13T00:00:00Z"}],"worlds":[{"key":"trek","name":"Star Trek","description":"","region":"Memory Alpha","position":[1,2,3]}],"destination":{"kind":"world","region":"Memory Alpha 10","position":[230,53,3302],"lookAt":[0,0,0],"ready":false}}"#,
        )
        .expect("roster");
        assert_eq!(roster.characters[0].name, "Captain Vale");
        assert_eq!(roster.world_name("trek"), "Star Trek");
        assert!(!roster.full());
        let destination = roster.destination.expect("destination");
        assert_eq!(destination.region, "Memory Alpha 10");
        assert_eq!(destination.position(), glam::Vec3::new(230.0, 53.0, 3302.0));
        assert_eq!(destination.look_at(), glam::Vec3::X, "no stored facing: east");
        assert!(!destination.ready);
        assert_eq!(
            answer(401, br#"{"code":"invalid_session","message":"This session is not logged in any more. Log in again."}"#),
            Err("This session is not logged in any more. Log in again.".to_owned())
        );
        assert!(answer(502, b"").is_err());
    }

    #[test]
    fn somewhere_else_lists_only_regions_that_are_up_and_never_the_gate() {
        let regions = regions_answer(
            200,
            br#"{"regions":[{"id":"1","name":"Memory Alpha","ready":false},{"id":"2","name":"Shopping Center","ready":true},{"id":"3","name":"Rookery Gate","ready":true},{"id":"4","name":"RP Hub East","ready":true}]}"#,
        )
        .expect("regions");
        let names: Vec<&str> = regions.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["RP Hub East", "Shopping Center"]);
    }
}
