//! A picture standing for a share of something: an icon of a bar in the HUD or in text, a progress
//! bar's fill. With pictures of its states beside it (`food_0.5`, `food_0.25`, or `clock_0` to
//! `clock_23`) the nearest one shows; without any, the picture is cut off like a progress bar.

use weft_abi::ui_dir;

/// The part of a picture (u0, v0, u1, v1) that a share `f` of it fills in direction `dir`.
pub fn crop(f: f32, dir: u32) -> [f32; 4] {
    let f = f.clamp(0.0, 1.0);
    match dir {
        ui_dir::UP => [0.0, 1.0 - f, 1.0, 1.0],
        ui_dir::LEFT => [1.0 - f, 0.0, 1.0, 1.0],
        ui_dir::DOWN => [0.0, 0.0, 1.0, f],
        _ => [0.0, 0.0, f, 1.0],
    }
}

/// The state nearest to share `f`; on a tie the fuller one.
fn nearest(states: &[(f32, String)], f: f32) -> Option<&(f32, String)> {
    states.iter().min_by(|a, b| (a.0 - f).abs().total_cmp(&(b.0 - f).abs()).then(b.0.total_cmp(&a.0)))
}

/// What to draw for share `f` of the picture `name`: a picture and the part of it, or nothing.
pub fn part(name: &str, f: f32, dir: u32) -> Option<(String, [f32; 4])> {
    let mut states = crate::assets::picture_states(name);
    if states.is_empty() {
        return (f > 0.0).then(|| (name.to_string(), crop(f, dir)));
    }
    // Numbered frames have a first one of their own; else none of it shows at nothing.
    if !states.iter().any(|s| s.0 == 0.0) {
        states.push((0.0, String::new()));
    }
    if crate::assets::texture_size(name).is_some() {
        states.push((1.0, name.to_string()));
    }
    nearest(&states, f.clamp(0.0, 1.0)).filter(|s| !s.1.is_empty()).map(|s| (s.1.clone(), [0.0, 0.0, 1.0, 1.0]))
}

/// A picture's states from the names of all pictures: `<name>_<share>`, `<name>_<frame>` when any
/// number is over 1 (frames of a whole turn, so the last isn't all of it), and vanilla's
/// `<stem>_half` for `<stem>_full`.
pub fn states_from<'a>(names: impl Iterator<Item = &'a str>) -> impl Iterator<Item = (String, Vec<(f32, String)>)> {
    let mut map: std::collections::HashMap<String, Vec<(f32, String)>> = Default::default();
    for name in names {
        let Some((stem, end)) = name.rsplit_once('_') else { continue };
        if end == "half" {
            map.entry(format!("{stem}_full")).or_default().push((0.5, name.to_string()));
        } else if let Some(n) = end.parse::<f32>().ok().filter(|n| n.is_finite() && *n >= 0.0) {
            map.entry(stem.to_string()).or_default().push((n, name.to_string()));
        }
    }
    for states in map.values_mut() {
        let top = states.iter().map(|s| s.0).fold(0.0, f32::max);
        if top > 1.0 {
            for s in states.iter_mut() {
                s.0 /= top + 1.0;
            }
        }
    }
    map.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn states(names: &[&str], of: &str) -> Vec<(f32, String)> {
        states_from(names.iter().copied()).find(|s| s.0 == of).map_or(Vec::new(), |s| s.1)
    }

    #[test]
    fn crops_toward_its_direction() {
        assert_eq!(crop(0.25, ui_dir::RIGHT), [0.0, 0.0, 0.25, 1.0]);
        assert_eq!(crop(0.25, ui_dir::LEFT), [0.75, 0.0, 1.0, 1.0]);
        assert_eq!(crop(0.25, ui_dir::UP), [0.0, 0.75, 1.0, 1.0]);
        assert_eq!(crop(0.25, ui_dir::DOWN), [0.0, 0.0, 1.0, 0.25]);
    }

    #[test]
    fn shares_and_vanilla_halves_are_states() {
        let s = states(&["m:gui/food_0.5", "m:gui/food_0.25", "m:gui/other"], "m:gui/food");
        assert_eq!(nearest(&s, 0.3).map(|s| s.1.as_str()), Some("m:gui/food_0.25"));
        assert_eq!(nearest(&s, 0.45).map(|s| s.1.as_str()), Some("m:gui/food_0.5"));
        let s = states(&["m:gui/armor_half"], "m:gui/armor_full");
        assert_eq!(s, vec![(0.5, "m:gui/armor_half".to_string())]);
    }

    #[test]
    fn numbers_over_one_are_frames_of_a_turn() {
        let names: Vec<String> = (0..24).map(|i| format!("m:gui/clock_{i}")).collect();
        let s = states(&names.iter().map(String::as_str).collect::<Vec<_>>(), "m:gui/clock");
        assert_eq!(nearest(&s, 0.5).map(|s| s.1.as_str()), Some("m:gui/clock_12"));
        assert_eq!(nearest(&s, 0.0).map(|s| s.1.as_str()), Some("m:gui/clock_0"));
        assert_eq!(nearest(&s, 0.99).map(|s| s.1.as_str()), Some("m:gui/clock_23"));
    }
}
