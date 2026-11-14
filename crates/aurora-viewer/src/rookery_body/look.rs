//! A character's look: everything the Character Creator sets, as plain
//! data. Saved per character on the grid (POST /rookery/characters
//! "save_look") and fetched for every avatar a viewer sees, so each viewer
//! builds the same body from the same pack. Small: slider values and ids.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The macro sliders (all 0..1), as MakeHuman's macro modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Macros {
    /// 0 woman, 1 man.
    pub gender: f32,
    /// 0 is 1 year, 0.1875 is 11, 0.5 is 25, 1 is 90 (MakeHuman's scale).
    pub age: f32,
    pub muscle: f32,
    pub weight: f32,
    pub height: f32,
    pub proportions: f32,
    pub african: f32,
    pub asian: f32,
    pub caucasian: f32,
    pub breast_size: f32,
    pub breast_firmness: f32,
}

impl Default for Macros {
    fn default() -> Self {
        Macros {
            gender: 0.0,
            age: 0.5,
            muscle: 0.5,
            weight: 0.5,
            height: 0.5,
            proportions: 0.5,
            african: 1.0 / 3.0,
            asian: 1.0 / 3.0,
            caucasian: 1.0 / 3.0,
            breast_size: 0.5,
            breast_firmness: 0.5,
        }
    }
}

impl Macros {
    /// MakeHuman's age in years for the age slider (human.py getAgeYears:
    /// 1 to 25 over the first half, 25 to 90 over the second).
    pub fn age_years(&self) -> f32 {
        let a = self.age.clamp(0.0, 1.0);
        if a < 0.5 { 1.0 + 48.0 * a } else { 25.0 + 130.0 * (a - 0.5) }
    }

    pub fn set_age_years(&mut self, years: f32) {
        let y = years.clamp(1.0, 90.0);
        self.age = if y < 25.0 { (y - 1.0) / 48.0 } else { 0.5 + (y - 25.0) / 130.0 };
    }
}

/// Colours multiplied into textures: white leaves them as painted.
pub const WHITE: [f32; 3] = [1.0, 1.0, 1.0];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Look {
    pub version: u32,
    pub macros: Macros,
    /// Detail sliders by MakeHuman modifier name ("nose/nose-scale-horiz-decr|incr").
    pub sliders: BTreeMap<String, f32>,
    pub skin: String,
    pub skin_tint: [f32; 3],
    pub eyes: String,
    pub eye_colour: String,
    pub eye_tint: [f32; 3],
    pub eyebrows: Option<String>,
    pub eyebrow_tint: [f32; 3],
    pub eyelashes: Option<String>,
    pub hair: Option<String>,
    pub hair_tint: [f32; 3],
    /// Worn clothes in layer order.
    pub clothes: Vec<String>,
    /// A colour per worn garment (white, or absent, leaves it as made).
    pub clothes_tints: BTreeMap<String, [f32; 3]>,
}

impl Default for Look {
    fn default() -> Self {
        Look {
            version: 1,
            macros: Macros::default(),
            sliders: BTreeMap::new(),
            skin: "skin-onlytheghosts_young_eurasian_female".into(),
            skin_tint: WHITE,
            eyes: "eyes-highpoly".into(),
            eye_colour: "eyecolour-brown".into(),
            eye_tint: WHITE,
            eyebrows: Some("eyebrows-mindfront_eyebrows_01".into()),
            eyebrow_tint: WHITE,
            eyelashes: Some("eyelashes-mindfront_eyelashes_01".into()),
            hair: None,
            hair_tint: WHITE,
            clothes: Vec::new(),
            clothes_tints: BTreeMap::new(),
        }
    }
}

impl Look {
    /// A stable hash of everything that changes the geometry (not colours or
    /// textures): the body's cache key.
    pub fn shape_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let m = &self.macros;
        for v in [
            m.gender, m.age, m.muscle, m.weight, m.height, m.proportions, m.african, m.asian, m.caucasian, m.breast_size,
            m.breast_firmness,
        ] {
            v.to_bits().hash(&mut h);
        }
        for (k, v) in &self.sliders {
            k.hash(&mut h);
            v.to_bits().hash(&mut h);
        }
        self.eyes.hash(&mut h);
        self.eyebrows.hash(&mut h);
        self.eyelashes.hash(&mut h);
        self.hair.hash(&mut h);
        self.clothes.hash(&mut h);
        h.finish()
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_look_round_trips_and_ages_follow_makehuman() {
        let mut look = Look::default();
        look.macros.weight = 0.8;
        look.sliders.insert("nose/nose-scale-horiz-decr|incr".into(), 0.4);
        look.hair = Some("hair-ponytail".into());
        let text = serde_json::to_string(&look).expect("json");
        let back: Look = serde_json::from_str(&text).expect("json");
        assert_eq!(back, look);
        // unknown or missing fields keep their defaults
        let partial: Look = serde_json::from_str(r#"{"macros":{"gender":1.0}}"#).expect("json");
        assert_eq!(partial.macros.gender, 1.0);
        assert_eq!(partial.macros.age, 0.5);
        assert_eq!(partial.skin, Look::default().skin);
        let mut m = Macros::default();
        assert!((m.age_years() - 25.0).abs() < 1e-4);
        m.set_age_years(40.0);
        assert!((m.age_years() - 40.0).abs() < 1e-3);
        m.set_age_years(11.0);
        assert!((m.age - 0.2083).abs() < 1e-3);
        // colours do not change the geometry's key; shape sliders do
        let mut tinted = look.clone();
        tinted.skin_tint = [0.8, 0.6, 0.5];
        assert_eq!(tinted.shape_hash(), look.shape_hash());
        tinted.macros.height = 0.7;
        assert_ne!(tinted.shape_hash(), look.shape_hash());
    }
}
