//! A resource pack made from the player's own copy of Minecraft. Mojang's textures can't ship
//! with the game, but a player who owns Minecraft has them on disk already: on start the game
//! finds the newest client jar of the official launcher or Prism, and builds
//! `resourcepacks/minecraft/` from it, enabling it the first time. The pack never leaves the
//! computer it was made on.
//!
//! What goes where is data of each mod (and the engine for `core:`): `assets/minecraft.txt`
//! lists `ours = theirs` texture paths, and files under `assets/minecraft/` are copied into the
//! pack as they are (a `.ctm` or `.gui` that fits the vanilla picture better).

use std::io::Read;
use std::path::{Path, PathBuf};

pub const PACK: &str = "minecraft";

fn get_roots_from_home() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);

    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        roots.push(appdata.join(".minecraft"));
        roots.push(appdata.join("PrismLauncher"));
    }
    if let Some(h) = &home {
        roots.push(h.join(".minecraft"));
        roots.push(h.join("Library/Application Support/minecraft"));
        roots.push(h.join("Library/Application Support/PrismLauncher"));
        roots.push(h.join(".local/share/PrismLauncher"));
    }
    roots
}

fn get_roots_from_env() -> Option<Vec<PathBuf>> {
    Some(
        std::env::var_os("MINECRAFT_DIR_LOOKUP")?
            .to_str()?
            .split(':')
            .map(PathBuf::from)
            .collect(),
    )
}

/// Client jars the launchers keep, newest first.
fn jars() -> Vec<(PathBuf, String)> {
    let roots = get_roots_from_env().unwrap_or_else(get_roots_from_home);

    let mut found: Vec<(std::time::SystemTime, PathBuf, String)> = Vec::new();
    let mut add = |path: PathBuf, version: String| {
        if let Ok(t) = std::fs::metadata(&path).and_then(|m| m.modified()) {
            found.push((t, path, version));
        }
    };
    for root in roots {
        // Official launcher: versions/<v>/<v>.jar.
        for e in std::fs::read_dir(root.join("versions"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let v = e.file_name().to_string_lossy().into_owned();
            add(e.path().join(format!("{v}.jar")), v);
        }
        // Prism: libraries/com/mojang/minecraft/<v>/minecraft-<v>-client.jar.
        for e in std::fs::read_dir(root.join("libraries/com/mojang/minecraft"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let v = e.file_name().to_string_lossy().into_owned();
            add(e.path().join(format!("minecraft-{v}-client.jar")), v);
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0));
    found.into_iter().map(|(_, p, v)| (p, v)).collect()
}

/// Makes or refreshes the pack; true when it was made for the first time (the caller enables it).
/// `mods` are (namespace, assets folder), the engine's `core` among them.
pub fn ensure_pack(run_dir: &Path, mods: &[(String, PathBuf)]) -> bool {
    let maps: Vec<(String, PathBuf, String)> = mods
        .iter()
        .filter_map(|(ns, dir)| {
            std::fs::read_to_string(dir.join("minecraft.txt"))
                .ok()
                .map(|m| (ns.clone(), dir.clone(), m))
        })
        .collect();
    if maps.is_empty() {
        return false;
    }
    let out = crate::packs::resource_dir(run_dir).join(PACK);
    let fresh = !out.is_dir();
    // Snapshots and modded versions can't be told from releases by name: a jar without the
    // textures is skipped.
    for (jar, version) in jars() {
        let Ok(stamp) = std::fs::metadata(&jar).and_then(|m| m.modified()) else {
            continue;
        };
        let mut key = format!("{}|{:?}", jar.display(), stamp);
        for (ns, _, m) in &maps {
            key.push_str(ns);
            key.push_str(m);
        }
        let key = format!("{:016x}", fnv(&key));
        let cfg = std::fs::read_to_string(out.join("pack.cfg")).unwrap_or_default();
        if crate::kv::get(&cfg, "source").is_some_and(|k| k == key) {
            return false;
        }
        match build(&jar, &version, &maps, &out, &key) {
            Ok(n) => {
                log::info!(
                    "made the {PACK} resource pack from {} ({n} files)",
                    jar.display()
                );
                return fresh;
            }
            Err(e) => log::warn!("{}: {e}", jar.display()),
        }
    }
    false
}

fn build(
    jar: &Path,
    version: &str,
    maps: &[(String, PathBuf, String)],
    out: &Path,
    key: &str,
) -> Result<usize, String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(jar).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    const T: &str = "assets/minecraft/textures/";
    if zip.by_name(&format!("{T}block/dirt.png")).is_err() {
        return Err("no textures in it".into());
    }
    let mut read = |name: &str| -> Option<Vec<u8>> {
        let mut f = zip.by_name(name).ok()?;
        let mut v = Vec::new();
        f.read_to_end(&mut v).ok()?;
        Some(v)
    };
    let tmp = out.with_extension("new");
    let _ = std::fs::remove_dir_all(&tmp);
    let mut files = 0;
    for (ns, dir, map) in maps {
        let base = tmp.join(ns).join("textures");
        for (n, line) in map.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            let Some((ours, theirs)) = line.split_once('=') else {
                continue;
            };
            let (ours, theirs) = (ours.trim(), theirs.trim());
            let mut words = theirs.split_whitespace();
            let (mode, src) = match words.next() {
                Some("glow") => ("glow", words.next().unwrap_or("")),
                // Drawn additively there, black for nothing: see `light_to_alpha`.
                Some("light") => ("light", words.next().unwrap_or("")),
                // Two pictures one above the other, for a model drawn from one texture.
                Some("stack") => ("stack", words.next().unwrap_or("")),
                // Colored by the game there, gray for nothing: see `dye`.
                Some("dyed") => ("dyed", words.next().unwrap_or("")),
                // Turned by a model there, one picture for every way it faces.
                Some("turned") => ("turned", words.next().unwrap_or("")),
                Some(s) => ("copy", s),
                None => continue,
            };
            let next = words.next();
            let lower = (mode == "stack").then_some(next).flatten();
            let overlay = (mode == "dyed").then_some(next).flatten();
            let color = words.clone().next().and_then(|w| u32::from_str_radix(w, 16).ok()).filter(|_| mode == "dyed");
            let turned = (mode == "turned").then_some(next).flatten().and_then(|w| w.parse::<u32>().ok());
            let crop: Vec<u32> = match next {
                Some("crop") => words.filter_map(|w| w.parse().ok()).collect(),
                _ if turned.is_some() && words.clone().next() == Some("crop") => words.skip(1).filter_map(|w| w.parse().ok()).collect(),
                _ => Vec::new(),
            };
            // Names change between versions: the first one this version has.
            let Some(png) = src.split('|').find_map(|s| read(&format!("{T}{s}.png"))) else {
                // Older or newer versions name some textures differently: ours stays.
                log::debug!("{ns}/minecraft.txt:{}: no {src} in this version", n + 1);
                continue;
            };
            let dst = base.join(format!("{ours}.png"));
            let dst_dir = dst.parent().map(Path::to_path_buf).unwrap_or_default();
            std::fs::create_dir_all(&dst_dir).map_err(|e| e.to_string())?;
            if let Some(lower) = lower {
                let below = read(&format!("{T}{lower}.png")).and_then(|b| crate::assets::decode_png(&b));
                let above = crate::assets::decode_png(&png);
                let (Some(a), Some(b)) = (above, below) else { continue };
                let mut px = a.rgba;
                px.extend_from_slice(&b.rgba);
                crate::assets::encode_png(&dst, a.width, a.height + b.height, &px).map_err(|e| e.to_string())?;
            } else if let (Some(overlay), Some(color)) = (overlay, color) {
                let img = crate::assets::decode_png(&png).ok_or(format!("{src}: not a png"))?;
                let above = read(&format!("{T}{overlay}.png")).and_then(|b| crate::assets::decode_png(&b));
                let Some(above) = above.filter(|a| (a.width, a.height) == (img.width, img.height)) else { continue };
                let mut px = img.rgba;
                dye(&mut px, &above.rgba, color);
                crate::assets::encode_png(&dst, img.width, img.height, &px).map_err(|e| e.to_string())?;
            } else if let Some(degrees) = turned {
                let img = crate::assets::decode_png(&png).ok_or(format!("{src}: not a png"))?;
                let (w, h, px) = cropped(&img, &crop);
                crate::assets::encode_png(&dst, w, h, &turn(&px, w, degrees / 90)).map_err(|e| e.to_string())?;
            } else if mode == "glow" || mode == "light" || crop.len() == 4 {
                let img = crate::assets::decode_png(&png).ok_or(format!("{src}: not a png"))?;
                let (w, h, mut px) = cropped(&img, &crop);
                if mode == "glow" {
                    // White where the texel is bright and warm (fire, embers).
                    for p in px.chunks_exact_mut(4) {
                        let on = p[3] > 0 && p[0] > 150 && p[0] as i32 - p[2] as i32 > 80;
                        p.copy_from_slice(&if on { [255; 4] } else { [0, 0, 0, 255] });
                    }
                }
                if mode == "light" {
                    light_to_alpha(&mut px);
                }
                crate::assets::encode_png(&dst, w, h, &px).map_err(|e| e.to_string())?;
            } else {
                std::fs::write(&dst, &png).map_err(|e| e.to_string())?;
            }
            files += 1;
            if let Some(meta) =
                read(&format!("{T}{src}.png.mcmeta")).and_then(|m| String::from_utf8(m).ok())
            {
                if meta.contains("\"gui\"") {
                    let _ = std::fs::write(dst.with_extension("png.mcmeta"), &meta);
                }
                if let Some(a) = serde_json::from_str::<serde_json::Value>(&meta)
                    .ok()
                    .and_then(|v| v.get("animation").cloned())
                {
                    let mut s = format!(
                        "frametime = {}\n",
                        a.get("frametime").and_then(|f| f.as_u64()).unwrap_or(1)
                    );
                    if a.get("interpolate").and_then(|i| i.as_bool()) == Some(true) {
                        s.push_str("interpolate = true\n");
                    }
                    let _ = std::fs::write(dst.with_extension("anim"), s);
                }
            }
        }
        // Files the mod made to go with the vanilla pictures.
        copy_tree(&dir.join("minecraft"), &tmp.join(ns)).map_err(|e| e.to_string())?;
    }
    std::fs::write(
        tmp.join("pack.cfg"),
        format!("name = Minecraft {version}\ndescription = Made from your own copy of Minecraft, stays on this computer\nsource = {key}\n"),
    )
    .map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(out);
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())?;
    Ok(files)
}

/// A picture meant to be added to what is behind it, as an ordinary transparent one: how bright a
/// pixel is becomes how much it covers, and its color what is left once that is taken out.
fn light_to_alpha(px: &mut [u8]) {
    for p in px.chunks_exact_mut(4) {
        let bright = p[0].max(p[1]).max(p[2]) as u32;
        if bright == 0 {
            p.copy_from_slice(&[0; 4]);
            continue;
        }
        for c in &mut p[..3] {
            *c = (*c as u32 * 255 / bright) as u8;
        }
        p[3] = (bright * p[3] as u32 / 255) as u8;
    }
}

/// A picture the game colors as it draws, as an ordinary one: gray times the color (`rrggbb`), with
/// what stays undyed over it.
fn dye(px: &mut [u8], overlay: &[u8], color: u32) {
    let tint = [(color >> 16) & 255, (color >> 8) & 255, color & 255];
    for (p, o) in px.chunks_exact_mut(4).zip(overlay.chunks_exact(4)) {
        let a = o[3] as u32;
        for i in 0..3 {
            let dyed = p[i] as u32 * tint[i] / 255;
            p[i] = ((o[i] as u32 * a + dyed * (255 - a)) / 255) as u8;
        }
        p[3] = p[3].max(o[3]);
    }
}

/// The part `crop` (x, y, w, h) of a picture, or all of it: its width, height and pixels.
fn cropped(img: &crate::assets::Image, crop: &[u32]) -> (u32, u32, Vec<u8>) {
    let (x0, y0, w, h) = if crop.len() == 4 {
        (crop[0], crop[1], crop[2], crop[3])
    } else {
        (0, 0, img.width, img.height)
    };
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in y0..(y0 + h).min(img.height) {
        let row = ((y * img.width + x0) * 4) as usize;
        px.extend_from_slice(&img.rgba[row..row + (w.min(img.width - x0) * 4) as usize]);
    }
    (w, h, px)
}

/// A square picture turned clockwise `quarters` times.
fn turn(px: &[u8], size: u32, quarters: u32) -> Vec<u8> {
    let mut out = px.to_vec();
    for _ in 0..quarters % 4 {
        let from = out.clone();
        for y in 0..size {
            for x in 0..size {
                let (a, b) = (((y * size + x) * 4) as usize, (((size - 1 - x) * size + y) * 4) as usize);
                out[a..a + 4].copy_from_slice(&from[b..b + 4]);
            }
        }
    }
    out
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    for e in std::fs::read_dir(from).into_iter().flatten().flatten() {
        let dst = to.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &dst)?;
        } else {
            std::fs::create_dir_all(to)?;
            std::fs::copy(e.path(), dst)?;
        }
    }
    Ok(())
}

fn fnv(s: &str) -> u64 {
    s.bytes().fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}

#[cfg(test)]
mod tests {
    use super::{dye, light_to_alpha, turn};

    /// Builds the real pack from the Minecraft on this computer into `run`, as a normal start does:
    /// automated runs don't, and this needs no window. `cargo test --release -p weft_engine real_pack -- --ignored`
    #[test]
    #[ignore]
    fn real_pack() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let roots = [("core".to_string(), root.join("crates/engine/assets")), ("base".to_string(), root.join("mods/base/assets"))];
        super::ensure_pack(&root.join("run"), &roots);
        let sky = root.join("run/resourcepacks").join(super::PACK).join("base/textures");
        let sun = crate::assets::decode_png(&std::fs::read(sky.join("sky/sun.png")).unwrap()).unwrap();
        assert_eq!(sun.rgba[3], 0, "the corner of the sun is transparent");
        assert!(sky.join("gui/attack_bar_fill.png").is_file());
        assert!(sky.join("entity/armor/leather_1.png").is_file(), "worn armor comes from the pack too");
    }

    #[test]
    fn additive_black_becomes_transparent() {
        let mut px = [0, 0, 0, 255, 255, 230, 100, 255, 40, 40, 20, 255, 40, 40, 20, 128];
        light_to_alpha(&mut px);
        assert_eq!(px[..4], [0, 0, 0, 0], "nothing to add covers nothing");
        assert_eq!(px[4..8], [255, 230, 100, 255], "the brightest stay as they are");
        assert_eq!(px[8..12], [255, 255, 127, 40], "a dim halo is its color, barely covering");
        assert_eq!(px[12..16], [255, 255, 127, 20], "alpha it already had counts");
    }

    #[test]
    fn turned_clockwise() {
        let (a, b, c, d) = ([1, 0, 0, 255], [2, 0, 0, 255], [3, 0, 0, 255], [4, 0, 0, 255]);
        let px = [a, b, c, d].concat();
        assert_eq!(turn(&px, 2, 1), [c, a, d, b].concat());
        assert_eq!(turn(&px, 2, 2), [d, c, b, a].concat());
        assert_eq!(turn(&px, 2, 4), px);
    }

    #[test]
    fn dyed_gray_takes_the_color_under_its_overlay() {
        let mut px = [255, 255, 255, 255, 128, 128, 128, 255, 200, 200, 200, 255, 0, 0, 0, 0];
        let overlay = [0, 0, 0, 0, 0, 0, 0, 0, 10, 20, 30, 255, 40, 50, 60, 255];
        dye(&mut px, &overlay, 0xa06540);
        assert_eq!(px[..4], [160, 101, 64, 255], "white becomes the color");
        assert_eq!(px[4..8], [80, 50, 32, 255], "gray a darker one");
        assert_eq!(px[8..12], [10, 20, 30, 255], "the overlay is not dyed");
        assert_eq!(px[12..16], [40, 50, 60, 255], "and shows where nothing is under it");
    }
}
