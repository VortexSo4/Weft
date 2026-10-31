//! Particles, text effects and conditions, shaders from mods, glow masks, models.

use std::time::Duration;

use weft_sdk::*;

use super::{add, probe};
use crate::runner::{R, T};

static SPARK: Particle = Particle::named("testmod:spark");

/// Bursts from the client and from the server (a broken block's dust) live and die out.
pub async fn particles(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let pos = [f[0] as f64 + 0.5, f[1] as f64 + 1.5, f[2] as f64 - 2.5];
    drop(t.c().spawn_particles(&SPARK, pos).count(200).spread(0.5).speed(1.0));
    t.until("client particles", Duration::from_secs(2), |t| t.state().particles >= 200).await?;
    // Sparks live at most 1.2 s.
    t.until("particles gone", Duration::from_secs(4), |t| t.state().particles == 0).await?;
    let b = add(f, [0, 0, -2]);
    t.set(b, "dirt").await?;
    t.op(&format!("break {} {} {}", b[0], b[1], b[2])).await?;
    // Vanilla's 4x4x4 pieces, sent by the server.
    t.until("block dust", Duration::from_secs(3), |t| t.state().particles >= 60).await?;
    t.until("dust settled", Duration::from_secs(4), |t| t.state().particles == 0).await?;
    Ok(())
}

/// Huge counts stay cheap: simulated in parallel, drawn in one instanced draw.
pub async fn particle_crowd(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let pos = [f[0] as f64 + 0.5, f[1] as f64 + 1.5, f[2] as f64 - 2.5];
    for _ in 0..10 {
        drop(t.c().spawn_particles(&SPARK, pos).count(10_000).spread(4.0).speed(1.0));
    }
    t.frames(2).await?;
    let n = t.state().particles;
    let b = t.bench(0.6, "100k particles").await?;
    t.note(format!("{n} particles: {:.0} fps, frame {:.2} ms, gpu {:.2} ms", b[0], b[1], b[3]));
    t.check(n >= 99_000 && b[1] < 33.0, format!("{n} particles in {:.1} ms frames", b[1]));
    Ok(())
}

/// Texture edges at a distance, where mip levels mix texels: no light line along the edge of a
/// snowy grass block over dirt; a 64x64 texture shows its own colors among 16x16 ones.
pub async fn texture_seams(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    t.fill(add(f, [-8, 0, -12]), add(f, [8, 0, -12]), "dirt").await?;
    t.fill(add(f, [-8, 1, -12]), add(f, [8, 1, -12]), "snowy_grass").await?;
    t.fill(add(f, [-8, 2, -12]), add(f, [8, 2, -12]), "dirt").await?;
    t.fill(add(f, [-8, 3, -12]), add(f, [8, 3, -12]), "grass").await?;
    t.fill(add(f, [-8, 0, -9]), add(f, [8, 0, -9]), "tall_grass").await?;
    // 64x64 next to 16x16 textures.
    let mosaic = add(f, [1, 0, -5]);
    t.fill(mosaic, add(mosaic, [1, 1, 0]), "testmod:mosaic").await?;
    t.c().look(35.0, 8.0);
    t.settled().await?;
    let shot = t.shot("seams").await?;
    // Just below and above the edge between the dirt and the snowy grass on top of it, on the
    // face the camera sees.
    let mut light = 0;
    let mut seen = 0;
    for x in -7..=7 {
        for dy in [0.96, 1.04] {
            if let Some((sx, sy)) = t.on_screen([(f[0] + x) as f64 + 0.5, f[1] as f64 + dy, (f[2] - 11) as f64 + 0.01]) {
                if (0.0..1.0).contains(&sx) && (0.0..1.0).contains(&sy) {
                    seen += 1;
                    light += (probe::lum(shot.at(sx, sy)) > 170.0) as u32;
                }
            }
        }
    }
    t.check(seen > 6, format!("the wall's edge is in view ({seen} points)"));
    t.probe("light points along the snowy grass edge", light as f32, light == 0);
    // The middle of the 2x2 mosaic face the camera sees.
    let p = t.on_screen([(mosaic[0] + 1) as f64, (mosaic[1] + 1) as f64, mosaic[2] as f64 + 1.01]).unwrap_or((0.5, 0.5));
    let colors = shot.share(p.0 - 0.03, p.1 - 0.05, p.0 + 0.03, p.1 + 0.05, |c| {
        let (hi, lo) = (*c.iter().max().unwrap() as i32, *c.iter().min().unwrap() as i32);
        hi - lo > 80
    });
    t.probe("the mosaic's saturated colors", colors, colors > 0.3);
    Ok(())
}

/// A 16x16 texture of one color.
fn write_png(path: &str, rgb: [u8; 3]) -> Result<(), String> {
    let dir = std::path::Path::new(path).parent().ok_or("no folder")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), 16, 16);
    enc.set_color(png::ColorType::Rgba);
    let px: Vec<u8> = (0..256).flat_map(|_| [rgb[0], rgb[1], rgb[2], 255]).collect();
    enc.write_header().and_then(|mut w| w.write_image_data(&px)).map_err(|e| e.to_string())
}

/// A mod's tint kind takes the biome's color where it sets one (plains: teal), else its
/// default (violet; the flatlands are beach).
pub async fn tint_kinds(t: T) -> R {
    for (flat, teal) in [(true, false), (false, true)] {
        if flat {
            t.flat().await?;
        } else {
            t.world(5).await?;
            t.idle().await?;
        }
        let f = t.feet();
        let (a, b) = (add(f, [-3, 0, -3]), add(f, [3, 3, -3]));
        t.fill(a, b, "testmod:glow_moss").await?;
        let moss = Block::lookup("testmod:glow_moss");
        // Every block of the wall must reach the client before its color can be looked at.
        t.until("the moss wall on the client", Duration::from_secs(5), |t| (a[1]..=b[1]).all(|y| (a[0]..=b[0]).all(|x| t.c().block([x, y, a[2]]) == moss))).await?;
        t.c().look(0.0, 0.0);
        t.settled().await?;
        let [r, g, b] = t.pixel(0.5, 0.45).await?;
        let ok = if teal { g > r + 40 && b > r + 30 } else { r > g + 20 && b > r };
        t.check(ok, format!("{} moss, got {r} {g} {b}", if teal { "teal" } else { "violet" }));
        t.shot(if teal { "plains" } else { "flatlands" }).await?;
    }
    Ok(())
}

/// Reloading assets picks up a resource pack's texture and later changes to that file.
pub async fn texture_reload(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    t.fill(add(f, [-3, 0, -3]), add(f, [3, 3, -3]), "dirt").await?;
    t.c().look(0.0, 0.0);
    t.settled().await?;
    let plain = t.grab().await?;
    let pack = format!("{}/resourcepacks/testkit_reload", t.c().run_dir());
    let file = format!("{pack}/base/textures/block/dirt.png");
    let reload = async |rgb: Option<[u8; 3]>| -> Result<Vec<u8>, String> {
        match rgb {
            Some(c) => write_png(&file, c)?,
            None => drop(std::fs::remove_dir_all(&pack)),
        }
        t.c().reload_assets();
        t.frames(5).await?;
        t.settled().await?;
        t.grab().await
    };
    weft_sdk::settings::RESOURCE_PACKS.set("testkit_reload");
    let red = reload(Some([200, 40, 40])).await?;
    let blue = reload(Some([40, 40, 200])).await?;
    weft_sdk::settings::RESOURCE_PACKS.set("");
    let back = reload(None).await?;
    t.check(T::differ(&plain, &red) > 0.2, "the pack's dirt shows after a reload");
    t.check(T::differ(&red, &blue) > 0.2, "a changed file shows after the next reload");
    t.check(T::differ(&plain, &back) < 0.01, "without the pack the dirt is back");
    Ok(())
}

/// A resource pack's translations change only the keys it has; the rest of the same mod's file
/// stays.
pub async fn pack_lang(t: T) -> R {
    let pack = format!("{}/resourcepacks/testkit_lang", t.c().run_dir());
    let dir = format!("{pack}/core/lang");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    std::fs::write(format!("{dir}/en_us.lang"), "menu.done = Pack Done\n").map_err(|e| e.to_string())?;
    weft_sdk::settings::RESOURCE_PACKS.set("testkit_lang");
    t.c().reload_assets();
    t.frames(2).await?;
    t.c().open_screen(&screens::SETTINGS);
    t.screen("core:settings").await?;
    t.frames(2).await?;
    t.laid_out(&["text:Pack Done", "text:View distance", "text:Language"]);
    t.c().close_screen();
    weft_sdk::settings::RESOURCE_PACKS.set("");
    drop(std::fs::remove_dir_all(&pack));
    t.c().reload_assets();
    t.frames(2).await?;
    t.c().open_screen(&screens::SETTINGS);
    t.screen("core:settings").await?;
    t.frames(2).await?;
    t.check(t.c().ui_rect("text:Pack Done").is_none(), "without the pack the key is the mod's again");
    t.laid_out(&["text:Done"]);
    t.c().close_screen();
    Ok(())
}

/// A shader pack whose files do not compile changes nothing: every engine shader, the composed
/// ones (models, particles, screen effects, UI) too, falls back to the built-in one.
pub async fn broken_shader_pack(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    t.op(&format!("spawn testmod:idol {} {} {}", f[0], f[1], f[2] - 3)).await?;
    t.c().look(0.0, 0.0);
    t.settled().await?;
    t.secs(0.5).await?;
    let before = t.grab().await?;
    let dir = format!("{}/shaderpacks/testkit_broken", t.c().run_dir());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for name in ["world.wgsl", "entity.wgsl", "model.wgsl", "particle.wgsl", "post.wgsl", "ui.wgsl"] {
        std::fs::write(format!("{dir}/{name}"), "this is not wgsl").map_err(|e| e.to_string())?;
    }
    weft_sdk::settings::SHADER_PACK.set("testkit_broken");
    t.c().reload_assets();
    t.frames(5).await?;
    let after = t.grab().await?;
    weft_sdk::settings::SHADER_PACK.set("");
    t.c().reload_assets();
    let _ = std::fs::remove_dir_all(&dir);
    t.frames(2).await?;
    t.check(T::differ(&before, &after) < 0.02, format!("the picture stays ({:.1}% differ)", T::differ(&before, &after) * 100.0));
    // A working pack only writes functions: the engine's own world shader with a sepia ending,
    // its structs and bindings come from the prelude.
    let dir = format!("{}/shaderpacks/testkit_sepia", t.c().run_dir());
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let world = include_str!("../../../../crates/engine/assets/shaders/world.wgsl");
    let ending = "    return vec4(mix(lit.rgb, scene.fog_color.rgb, fog), lit.a);";
    t.check(world.contains(ending), "the world shader still ends its shading like this");
    // Packs are copies of it, from whenever they were made.
    t.check(world.contains("tex_info[name * 2u]"), "the texture table still reads as packs read it");
    let sepia = world.replace(ending, "    let y = dot(mix(lit.rgb, scene.fog_color.rgb, fog), vec3(0.3, 0.59, 0.11));
    return vec4(vec3(y * 1.2, y, y * 0.75), lit.a);");
    std::fs::write(format!("{dir}/world.wgsl"), sepia).map_err(|e| e.to_string())?;
    weft_sdk::settings::SHADER_PACK.set("testkit_sepia");
    t.c().reload_assets();
    t.frames(5).await?;
    let shot = t.shot("sepia").await?;
    weft_sdk::settings::SHADER_PACK.set("");
    t.c().reload_assets();
    let _ = std::fs::remove_dir_all(&dir);
    t.frames(2).await?;
    // The grass floor turns brown: red over green over blue.
    let brown = shot.share(0.0, 0.7, 1.0, 0.9, |c| c[0] > c[1] && c[1] > c[2]);
    t.probe("a pack's sepia world shader is used", brown, brown > 0.8);
    Ok(())
}

/// The test mod's looks at night: a glow crystal's veins glow without lighting the floor next to
/// it, a wisp floats nearby, and the daydream screen effect changes the picture.
pub async fn shaders_and_glow(t: T) -> R {
    t.world(9).await?;
    t.idle().await?;
    t.op("time 18000").await?;
    let f = t.feet();
    // A level stage of its own, so terrain changes do not move the picture.
    t.fill(add(f, [-4, -1, -7]), add(f, [4, -1, 3]), "stone").await?;
    t.fill(add(f, [-4, 0, -7]), add(f, [4, 5, 3]), "air").await?;
    let crystal = add(f, [0, 1, -4]);
    t.fill(add(f, [-1, 0, -4]), add(f, [1, 2, -4]), "testmod:glow_crystal").await?;
    // A block shader (`shaders/block/stripes.wgsl`): magenta bands, lit or not.
    let striped = add(f, [-3, 1, -4]);
    t.set(striped, "testmod:reinforced_stone").await?;
    // Off to the side and back a little: it drifts, so it stays small in the picture.
    t.op(&format!("spawn testmod:wisp {} {} {}", f[0] + 3, f[1] + 1, f[2] - 5)).await?;
    t.check(t.count_near("wisp", 8.0).await? == 1, "a wisp floats nearby");
    t.op("put me 0 testmod:rune 1").await?;
    t.c().select(0);
    t.c().look(0.0, -5.0);
    t.settled().await?;
    t.secs(1.0).await?;
    let night = t.shot("night_glow").await?;
    let veins = match t.block_on_screen(crystal, [0.0, 0.0, 0.51]) {
        Some((x, y)) => night.share(x - 0.12, y - 0.15, x + 0.12, y + 0.15, |c| probe::cyan(c) && probe::lum(c) > 100.0),
        None => 0.0,
    };
    t.probe("the crystal's veins glow", veins, veins > 0.03);
    let floor = match t.on_screen([f[0] as f64 + 0.5, f[1] as f64, f[2] as f64 - 2.5]) {
        Some((x, y)) => crate::runner::lum(night.avg(x - 0.02, y - 0.02, x + 0.02, y + 0.02)),
        None => 255.0,
    };
    t.probe("the floor in front of it stays dark (glow is not light)", floor, floor < 60.0);
    let bands = match t.block_on_screen(striped, [0.0, 0.0, 0.51]) {
        Some((x, y)) => night.share(x - 0.03, y - 0.05, x + 0.03, y + 0.05, |c| c[0] > 100 && c[2] > 100 && c[1] < 60),
        None => 0.0,
    };
    t.probe("the block shader's magenta bands", bands, bands > 0.2 && bands < 0.9);
    let before = t.grab().await?;
    t.check(t.c().post_effect("testmod:dream", 1.0), "the daydream effect exists");
    t.check(!t.c().post_effect("testmod:nonexistent", 1.0), "an unknown effect is refused");
    t.frames(3).await?;
    t.shot("dream").await?;
    let after = t.grab().await?;
    let d = T::differ(&before, &after) as f32;
    t.probe("the daydream effect changes the picture", d, d > 0.03);
    t.c().post_effect("testmod:dream", 0.0);
    Ok(())
}

/// A block shaped by a Blockbench Java model file: the test mod's pedestal is a plate, a crystal
/// column and a top, so against the sky the sides of the column show sky, the column itself not.
pub async fn block_model_file(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let p = add(f, [0, 2, -3]);
    t.set(p, "testmod:pedestal").await?;
    t.c().look(0.0, 15.0);
    t.settled().await?;
    let shot = t.shot("pedestal").await?;
    let at = |x: f64| t.on_screen([p[0] as f64 + x, p[1] as f64 + 0.35, p[2] as f64 + 0.5]).map(|(sx, sy)| crate::runner::lum(shot.avg(sx - 0.003, sy - 0.003, sx + 0.003, sy + 0.003)));
    let (gap, column) = (at(0.18).unwrap_or(0.0), at(0.5).unwrap_or(255.0));
    t.probe("sky beside the column (the file's shape, not a cube)", gap, gap > 180.0);
    t.probe("the crystal column itself", column, column < 150.0);
    // A resource pack's model of the same block redraws it, here as a whole cube.
    let pack = format!("{}/resourcepacks/testkit_shapes", t.c().run_dir());
    let file = format!("{pack}/testmod/models/block/pedestal.json");
    std::fs::create_dir_all(std::path::Path::new(&file).parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&file, r#"{"elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"north":{},"south":{},"east":{},"west":{},"up":{},"down":{}}}]}"#).map_err(|e| e.to_string())?;
    weft_sdk::settings::RESOURCE_PACKS.set("testkit_shapes");
    t.c().reload_assets();
    t.frames(5).await?;
    t.settled().await?;
    let cube = t.shot("pedestal_pack").await?;
    weft_sdk::settings::RESOURCE_PACKS.set("");
    let _ = std::fs::remove_dir_all(&pack);
    t.c().reload_assets();
    t.frames(2).await?;
    let filled = t.on_screen([p[0] as f64 + 0.18, p[1] as f64 + 0.35, p[2] as f64 + 0.5]).map(|(sx, sy)| crate::runner::lum(cube.avg(sx - 0.003, sy - 0.003, sx + 0.003, sy + 0.003))).unwrap_or(255.0);
    t.probe("a pack's model draws a cube there", filled, filled < 150.0);
    Ok(())
}

/// An item shaped by a model file (the test mod's totem: a stone base, a crystal on it) shows its
/// boxes in the hand and in the hotbar slot.
pub async fn item_model_file(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 testmod:totem 1").await?;
    t.c().select(0);
    t.c().look(0.0, 20.0);
    t.settled().await?;
    // The hand comes up with it.
    t.secs(0.8).await?;
    let shot = t.shot("totem").await?;
    let crystal = |c: probe::C| probe::cyan(c) || (c[2] > 120 && c[0] < 90);
    let hand = shot.share(0.55, 0.55, 1.0, 0.94, crystal);
    t.probe("the crystal in the hand", hand, hand > 0.01);
    // The first hotbar slot of the HUD, at the bottom middle (gui scale 2, 1600x900).
    let slot = shot.share(0.388, 0.95, 0.412, 0.995, crystal);
    t.probe("the crystal in the hotbar slot", slot, slot > 0.015);
    Ok(())
}

/// Models: the orb (built in code, a sphere mesh and a child bone) turns by its file's idle
/// animation; the idol (a Blockbench file with bones and mesh wings) bows when used, a server
/// triggered animation that holds its last frame.
pub async fn models(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    t.op(&format!("spawn testmod:orb {} {} {}", f[0], f[1] + 1, f[2] - 3)).await?;
    t.op(&format!("spawn testmod:idol {} {} {}", f[0], f[1], f[2] + 3)).await?;
    t.check(t.count_near("orb", 8.0).await? == 1 && t.count_near("idol", 8.0).await? == 1, "an orb and an idol stand nearby");
    t.c().look(0.0, 0.0);
    t.settled().await?;
    t.secs(0.5).await?;
    let a = t.grab().await?;
    t.secs(0.5).await?;
    let b = t.grab().await?;
    t.check(T::differ(&a, &b) > 0.001, "the orb's ring turns by its keyframes");
    t.look_at(add(f, [0, 0, 3]));
    t.frames(3).await?;
    t.press("core:use");
    t.secs(1.5).await?;
    let a = t.grab().await?;
    t.secs(0.3).await?;
    let b = t.grab().await?;
    // Below the horizon: clouds drift up there.
    let ground = |f: &[u8]| f[f.len() / 4 / 3 * 4..].to_vec();
    t.check(T::differ(&ground(&a), &ground(&b)) == 0.0, "the bow holds its last frame");
    t.shot("idol_bow").await?;
    // The test mod gives the player a wave: it moves the first person hand by its `hand` bone.
    t.c().look(90.0, 0.0);
    t.frames(3).await?;
    let a = t.grab().await?;
    t.op("animate me wave").await?;
    t.secs(0.5).await?;
    let b = t.grab().await?;
    t.check(T::differ(&a, &b) > 0.002, "the first person hand waves");
    Ok(())
}

/// Tooltip parts behind `{on_key}` and `{if}` appear while their condition holds: a hurt player
/// sees the rune's red line, sneaking swaps "more" for its long text.
pub async fn rich_text(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 testmod:rune 1").await?;
    // A fall hurts the player, which the test mod's `{if:testmod:hurt}` shows.
    let s = t.state();
    t.c().teleport([s.pos[0], s.pos[1] + 6.0, s.pos[2]]);
    t.until("hurt by the fall", Duration::from_secs(5), |t| t.state().health < 20.0).await?;
    t.press("core:inventory");
    t.screen("core:menu").await?;
    let Some([x, y, w, h]) = t.c().ui_rect("slot:36") else { return Err("no hotbar slot in the inventory".into()) };
    t.c().cursor(x + w / 2.0, y + h / 2.0);
    t.frames(4).await?;
    let rune = t.shot("rune").await?;
    let wound = |c: [u8; 3]| c[0] > 200 && (100..170).contains(&c[1]) && (100..170).contains(&c[2]);
    // The tooltip opens beside the cursor.
    let (cx, cy) = (x + w / 2.0, y + h / 2.0);
    let v = rune.share(cx, cy - 0.15, cx + 0.3, cy + 0.15, wound);
    t.probe("the {if:testmod:hurt} line (light red)", v, v > 0.003);
    t.hold("core:sneak", 2.0);
    t.frames(4).await?;
    let sneak = t.shot("rune_sneak").await?;
    let d = T::differ(&rune.px, &sneak.px) as f32;
    t.probe("sneaking changes the tooltip ({on_key})", d, d > 0.002);
    t.c().release_all();
    t.c().close_screen();
    t.no_screen().await
}

/// The machine screen's panel is drawn by the test mod's GUI shader (teal circuit lines).
pub async fn gui_shader(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let p = add(f, [0, 0, -2]);
    t.set(p, "testmod:generator").await?;
    t.op(&format!("menu testmod:generator {} {} {}", p[0], p[1], p[2])).await?;
    t.screen("core:menu").await?;
    t.frames(3).await?;
    let shot = t.shot("generator").await?;
    let Some([x, y, _, h]) = t.c().ui_rect("text:Generator") else { return Err("no Generator title".into()) };
    let v = shot.share(x, y + h, x + 0.2, y + 0.12, probe::teal);
    t.probe("circuit lines in the panel", v, v > 0.01);
    t.c().close_screen();
    t.no_screen().await
}

/// The sky: the sun at noon and the moon at midnight stand overhead, stars show only at night, a
/// sunset warms the horizon on its side, and from inside water everything is its fog.
pub async fn sky(t: T) -> R {
    t.world(5).await?;
    t.op("mode creative").await?;
    t.c().look(0.0, 89.0);
    let bright = |c: probe::C| c.iter().all(|&v| v > 215);
    t.op("time 6000").await?;
    t.secs(0.6).await?;
    let noon = t.shot("noon").await?;
    t.probe("the sun overhead", noon.share(0.45, 0.40, 0.55, 0.57, bright), noon.share(0.45, 0.40, 0.55, 0.57, bright) > 0.5);
    // A star is a speck: bright with sky a little way off on every side, which a cloud isn't.
    let specks = |shot: &crate::runner::Shot| {
        let (sx, sy) = (1.0 / 1600.0, 1.0 / 900.0);
        let mut n = 0;
        for y in (10..225).map(|y| y as f32 * sy * 1.0) {
            for x in (10..1590).map(|x| x as f32 * sx) {
                let lit = |x: f32, y: f32| shot.at(x, y).iter().all(|&v| v > 215);
                if lit(x, y) && !lit(x - 6.0 * sx, y) && !lit(x + 6.0 * sx, y) && !lit(x, y - 6.0 * sy) && !lit(x, y + 6.0 * sy) {
                    n += 1;
                }
            }
        }
        n
    };
    t.check_eq(specks(&noon), 0, "no stars by day");
    t.op("time 18000").await?;
    t.secs(0.6).await?;
    let night = t.shot("midnight").await?;
    let moon = night.share(0.45, 0.42, 0.55, 0.56, |c| c.iter().all(|&v| v > 150));
    t.probe("the moon overhead", moon, moon > 0.4);
    let stars = night.share(0.0, 0.0, 1.0, 0.3, |c| c.iter().all(|&v| v > 60));
    t.probe("stars at night", stars, stars > 0.0005);
    // Looking at the sunset and away from it.
    t.op("time 12400").await?;
    t.c().look(90.0, 8.0);
    t.secs(0.6).await?;
    let west = t.shot("sunset").await?;
    let warm = |c: probe::C| c[0] > 170 && c[0] as i32 > c[2] as i32 + 50;
    t.c().look(-90.0, 8.0);
    t.secs(0.6).await?;
    let east = t.shot("sunset_back").await?;
    let (w, e) = (west.share(0.0, 0.3, 1.0, 0.5, warm), east.share(0.0, 0.3, 1.0, 0.5, warm));
    t.note(format!("warm sky: toward the sunset {w:.3}, away {e:.3}"));
    t.check(w > e + 0.02, format!("a sunset on its side: {w:.3}, away {e:.3}"));
    Ok(())
}

/// Clouds reach far past any grid around the camera, cover what is under them, stay behind what is
/// nearer, and the setting lifts them.
pub async fn clouds(t: T) -> R {
    static LIFT: Config<i32> = Config::named("core:cloud_height");
    t.world(5).await?;
    t.op("mode spectator").await?;
    t.op("time 6000").await?;
    t.cmd("/weather clear").await?;
    LIFT.set(0);
    let white = |c: probe::C| c.iter().all(|&v| v > 205);
    let s = t.state();
    // Near the horizon from well below them: what is there is thousands of blocks away.
    t.c().teleport([s.pos[0], 150.0, s.pos[2]]);
    t.c().look(0.0, 2.0);
    t.secs(1.0).await?;
    let (x0, y0, x1, y1) = (0.0, 0.47, 1.0, 0.53);
    let far = t.shot("far").await?.share(x0, y0, x1, y1, white);
    // Lifted out of reach at that angle.
    LIFT.set(512);
    t.secs(0.3).await?;
    let lifted = t.shot("far_lifted").await?.share(x0, y0, x1, y1, white);
    t.note(format!("white near the horizon: {far:.3}, lifted {lifted:.3}"));
    t.check(far > lifted + 0.02, format!("clouds far away: {far:.3}, lifted out of reach {lifted:.3}"));
    // From above they cover the ground; lifted over the camera they don't.
    LIFT.set(0);
    t.c().teleport([s.pos[0], 330.0, s.pos[2]]);
    t.c().look(0.0, -60.0);
    t.secs(1.0).await?;
    let above = t.shot("above").await?.share(0.0, 0.3, 1.0, 1.0, white);
    LIFT.set(512);
    t.secs(0.3).await?;
    let lifted = t.shot("above_lifted").await?.share(0.0, 0.3, 1.0, 1.0, white);
    t.note(format!("white from above: {above:.3}, lifted {lifted:.3}"));
    t.check(above > lifted + 0.02, format!("over the ground: {above:.3}, lifted away {lifted:.3}"));
    // A wall nearer than they are hides them.
    LIFT.set(0);
    let at = [s.pos[0].floor() as i32, 150, s.pos[2].floor() as i32];
    t.fill(add(at, [-30, 0, -12]), add(at, [30, 70, -12]), "stone").await?;
    t.c().teleport([s.pos[0], 150.0, s.pos[2]]);
    t.c().look(0.0, 35.0);
    t.secs(1.0).await?;
    let wall = t.shot("wall").await?.share(0.2, 0.3, 0.8, 0.7, white);
    t.check(wall < 0.005, format!("clouds through a wall: {wall:.3}"));
    // Its slider is among the others.
    t.c().open_screen(&screens::SETTINGS);
    t.screen("core:settings").await?;
    t.frames(3).await?;
    t.shot("settings").await?;
    t.c().close_screen();
    t.no_screen().await?;
    Ok(())
}

/// The attack indicator is a HUD element like the others: under the crosshair while a swing
/// charges, filling, gone when charged, and the crosshair stays in the middle of the window.
pub async fn attack_indicator(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 iron_sword 1").await?;
    t.c().select(0);
    // A swing that hits nothing spends the charge; looking down, nothing bright is behind the bar.
    t.op("attr_mod me core:block_reach no_reach 0 -10 0").await?;
    t.c().look(0.0, -40.0);
    t.secs(1.0).await?;
    let (x0, y0, x1, y1) = (0.49, 0.505, 0.51, 0.54);
    let fill = |c: probe::C| c.iter().all(|&v| v > 200);
    let rest = t.shot("charged").await?;
    t.check(rest.share(x0, y0, x1, y1, fill) < 0.01, "nothing under the crosshair when charged");
    let center = rest.share(0.497, 0.494, 0.503, 0.506, |c| c.iter().all(|&v| v > 150));
    t.probe("the crosshair in the middle of the window", center, center > 0.1);
    t.press("core:attack");
    t.secs(0.15).await?;
    let early = t.shot("charging").await?.share(x0, y0, x1, y1, fill);
    t.secs(0.2).await?;
    let later = t.shot("charging_later").await?.share(x0, y0, x1, y1, fill);
    t.note(format!("indicator filled: {early:.3}, later {later:.3}"));
    t.check(later > 0.02, format!("the indicator shows while charging: {later:.3}"));
    t.check(later > early, format!("and fills: {early:.3}, then {later:.3}"));
    t.secs(0.6).await?;
    t.check(t.shot("charged_again").await?.share(x0, y0, x1, y1, fill) < 0.01, "and is gone once charged");
    t.op("attr_rm me core:block_reach no_reach").await?;
    Ok(())
}

/// Where the ground starts on two columns of the picture; a tilted view has them differ.
fn tilt(shot: &crate::runner::Shot) -> f32 {
    let horizon = |x: f32| (0..450).map(|i| 0.1 + i as f32 / 450.0 * 0.8).find(|&y| probe::grass(shot.at(x, y))).unwrap_or(0.9);
    horizon(0.1) - horizon(0.9)
}

/// Third person shakes like first: from a hit, and from walking.
pub async fn third_person_shakes(t: T) -> R {
    t.flat().await?;
    t.c().look(0.0, 0.0);
    t.press("core:perspective");
    t.secs(0.5).await?;
    let still = tilt(&t.shot("still").await?);
    t.check(still.abs() < 0.003, format!("level standing still: {still}"));
    t.hold("core:forward", 2.0);
    t.secs(0.3).await?;
    let mut range = (f32::MAX, f32::MIN);
    for i in 0..10 {
        let v = tilt(&t.shot(&format!("walking{i}")).await?);
        range = (range.0.min(v), range.1.max(v));
    }
    t.note(format!("tilt walking: {:.4} to {:.4}", range.0, range.1));
    t.check(range.1 - range.0 > 0.001, format!("walking sways the view: {range:?}"));
    t.secs(1.0).await?;
    let s = t.state().pos;
    t.c().teleport([s[0], s[1] + 8.0, s[2]]);
    t.until("hurt by the fall", Duration::from_secs(5), |t| t.state().health < 20.0).await?;
    let mut most = 0.0f32;
    for i in 0..4 {
        most = most.max(tilt(&t.shot(&format!("hurt{i}")).await?).abs());
    }
    t.note(format!("tilt when hit: {most:.4}"));
    t.check(most > 0.01, format!("a hit jolts the view: {most}"));
    t.press("core:perspective");
    t.press("core:perspective");
    Ok(())
}

/// A broken block's pieces fly out to the sides, past where it stood, also when the client hears
/// of the dust before the block is gone.
pub async fn block_dust_scatters(t: T) -> R {
    t.flat().await?;
    let b = add(t.feet(), [3, 3, 0]);
    t.set(b, "stone").await?;
    t.until("the block on the client", Duration::from_secs(2), |t| t.c().block(b).is_some_and(|s| s != 0)).await?;
    t.look_at(b);
    t.secs(0.3).await?;
    // Darker than the sky and any cloud.
    let piece = |c: probe::C| c.iter().all(|&v| v < 170);
    let sides = |s: &crate::runner::Shot| [s.share(0.30, 0.3, 0.42, 0.7, piece), s.share(0.58, 0.3, 0.70, 0.7, piece)];
    let before = sides(&t.shot("before").await?);
    t.op(&format!("break {} {} {}", b[0], b[1], b[2])).await?;
    t.secs(0.25).await?;
    let shot = t.shot("after").await?;
    let after = sides(&shot);
    t.note(format!("pieces at the sides: {before:?} then {after:?}"));
    t.check(after[0] > before[0] + 0.0005, format!("pieces on the left: {before:?} then {after:?}"));
    t.check(after[1] > before[1] + 0.0005, format!("pieces on the right: {before:?} then {after:?}"));
    // None stay black from having started where the block was, with no light in it.
    let black = shot.share(0.3, 0.2, 0.7, 0.8, |c| c.iter().all(|&v| v < 30));
    t.check(black < 0.0002, format!("black pieces: {black}"));
    Ok(())
}

/// From high above, where the view's cubes don't reach the ground, it is land all the way under
/// the camera, not a hole of sky.
pub async fn distant_terrain_from_above(t: T) -> R {
    static VIEW: Config<i32> = Config::named("core:view_distance");
    static LOD: Config<i32> = Config::named("core:lod_distance");
    let before = (VIEW.get(), LOD.get());
    VIEW.set(8);
    LOD.set(256);
    let out = async {
        t.world(5).await?;
        t.idle().await?;
        t.op("mode spectator").await?;
        let s = t.state().pos;
        t.c().teleport([s[0], 1024.0, s[2]]);
        t.c().look(0.0, -89.0);
        t.until("distant terrain made", Duration::from_secs(20), |t| t.state().lod_tiles > 50 && t.state().lod_pending == 0).await?;
        t.secs(0.5).await?;
        let shot = t.shot("down").await?;
        // The sky's pale blue; a cloud may be in the way, which is white.
        let sky = shot.share(0.44, 0.42, 0.56, 0.58, |c| c[2] > 200 && c[0] > 120 && c[0] < 215);
        t.note(format!("sky under the camera: {sky:.3}"));
        t.check(sky < 0.05, format!("a hole in the land: {sky:.3}"));
        Ok(())
    }
    .await;
    VIEW.set(before.0);
    LOD.set(before.1);
    out
}
