//! MakeHuman's shape rules (MakeHuman 1.x apps/human.py, apps/humanmodifier.py,
//! lib/targets.py; CC0 data, AGPL code read for its arithmetic): the macro
//! values from the macro sliders, then each target's weight as the product
//! of its macro variables' values and its slider's own side factor. The same
//! arithmetic as tools/makehuman/rookery_body/modifiers.py, which built and
//! checked the pack.

use super::look::Macros;
use super::pack::Pack;
use std::collections::HashMap;

/// The human's macro values (`_setGenderVals` .. `_setEthnicVals`).
pub fn macro_values(m: &Macros) -> HashMap<&'static str, f32> {
    let mut v = HashMap::new();
    let c = |x: f32| x.clamp(0.0, 1.0);
    let gender = c(m.gender);
    v.insert("male", gender);
    v.insert("female", 1.0 - gender);
    let age = c(m.age);
    if age < 0.5 {
        v.insert("old", 0.0);
        v.insert("baby", (1.0 - age * 5.333).max(0.0));
        let young = ((age - 0.1875) * 3.2).max(0.0);
        v.insert("young", young);
        v.insert("child", ((5.333 * age).min(1.0) - young).max(0.0));
    } else {
        v.insert("child", 0.0);
        v.insert("baby", 0.0);
        let old = (age * 2.0 - 1.0).max(0.0);
        v.insert("old", old);
        v.insert("young", 1.0 - old);
    }
    for (low, mid, high, value) in [
        ("minweight", "averageweight", "maxweight", c(m.weight)),
        ("minmuscle", "averagemuscle", "maxmuscle", c(m.muscle)),
    ] {
        let hi = (value * 2.0 - 1.0).max(0.0);
        let lo = (1.0 - value * 2.0).max(0.0);
        v.insert(high, hi);
        v.insert(low, lo);
        v.insert(mid, 1.0 - (hi + lo));
    }
    for (low, mid, high, value) in [
        ("minheight", "averageheight", "maxheight", c(m.height)),
        ("mincup", "averagecup", "maxcup", c(m.breast_size)),
        ("minfirmness", "averagefirmness", "maxfirmness", c(m.breast_firmness)),
        ("uncommonproportions", "regularproportions", "idealproportions", c(m.proportions)),
    ] {
        let hi = (value * 2.0 - 1.0).max(0.0);
        let lo = (1.0 - value * 2.0).max(0.0);
        v.insert(high, hi);
        v.insert(low, lo);
        v.insert(mid, if hi > lo { 1.0 - hi } else { 1.0 - lo });
    }
    let (mut af, mut asi, mut ca) = (c(m.african), c(m.asian), c(m.caucasian));
    let total = af + asi + ca;
    if total <= 0.0 {
        af = 1.0 / 3.0;
        asi = 1.0 / 3.0;
        ca = 1.0 / 3.0;
    } else {
        af /= total;
        asi /= total;
        ca /= total;
    }
    v.insert("african", af);
    v.insert("asian", asi);
    v.insert("caucasian", ca);
    v
}

/// Target index -> weight, for every target with a weight other than zero.
pub fn target_weights(pack: &Pack, macros: &Macros, sliders: &HashMap<String, f32>) -> Vec<(usize, f32)> {
    let values = macro_values(macros);
    let product = |target: usize| -> f32 {
        pack.targets[target]
            .variables
            .iter()
            .map(|v| values.get(v.as_str()).copied().unwrap_or(1.0))
            .product()
    };
    let mut weights: HashMap<usize, f32> = HashMap::new();
    for modifier in &pack.modifiers {
        if modifier.kind == "macro" {
            for &t in pack.groups.get(&modifier.group).into_iter().flatten() {
                // a macro group's weight is the product alone (factor of
                // the group itself 1.0), set rather than added, as
                // human.setDetail does for each macro modifier in turn
                weights.insert(t, product(t));
            }
            continue;
        }
        let value = sliders.get(&modifier.name).copied().unwrap_or(modifier.default);
        let lowest = if modifier.left.is_some() { -1.0 } else { 0.0 };
        let value = value.clamp(lowest, 1.0);
        let mut sides: Vec<(&str, f32)> = Vec::new();
        if let Some(right) = &modifier.right {
            sides.push((right.as_str(), value.max(0.0)));
        }
        if let Some(left) = &modifier.left {
            sides.push((left.as_str(), -value.min(0.0)));
        }
        if let Some(center) = &modifier.center {
            sides.push((center.as_str(), 1.0 - value.abs()));
        }
        for (key, side) in sides {
            if side == 0.0 {
                continue;
            }
            for &t in pack.groups.get(key).into_iter().flatten() {
                let w = side * product(t);
                if w != 0.0 {
                    *weights.entry(t).or_insert(0.0) += w;
                }
            }
        }
    }
    let mut list: Vec<(usize, f32)> = weights.into_iter().filter(|(_, w)| *w != 0.0).collect();
    list.sort_unstable_by_key(|(t, _)| *t);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_values_follow_makehuman() {
        let v = macro_values(&Macros::default());
        assert_eq!(v["female"], 1.0, "the default Rookery body is a woman");
        assert_eq!(v["young"], 1.0);
        assert_eq!(v["averageweight"], 1.0);
        assert_eq!(v["averagemuscle"], 1.0);
        assert!((v["african"] - 1.0 / 3.0).abs() < 1e-6);
        let heavy = macro_values(&Macros { weight: 1.0, ..Macros::default() });
        assert_eq!(heavy["maxweight"], 1.0);
        assert_eq!(heavy["averageweight"], 0.0);
        let child = macro_values(&Macros { age: 0.1, ..Macros::default() });
        assert!(child["baby"] > 0.0 && child["child"] > 0.0 && child["young"] == 0.0);
        let elder = macro_values(&Macros { age: 0.9, ..Macros::default() });
        assert!((elder["old"] - 0.8).abs() < 1e-6 && (elder["young"] - 0.2).abs() < 1e-6);
        let tall = macro_values(&Macros { height: 0.8, ..Macros::default() });
        assert!((tall["maxheight"] - 0.6).abs() < 1e-6 && (tall["averageheight"] - 0.4).abs() < 1e-6);
    }
}
