use std::time::Duration;

use weft_sdk::*;

use super::{add, at};
use crate::runner::{R, T};

fn item(name: &str) -> u32 {
    Item::lookup(name).unwrap_or(u32::MAX)
}

use base_api::HUNGER;

pub async fn sprint_and_sneak_modes(t: T) -> R {
    use weft_sdk::Config;
    static TOGGLE_SNEAK: Config<bool> = Config::named("core:toggle_sneak");
    static TOGGLE_SPRINT: Config<bool> = Config::named("core:toggle_sprint");
    t.flat().await?;
    // The options are in Controls, and nothing is left of them in the Settings.
    t.c().open_screen(&screens::SETTINGS);
    t.screen("core:settings").await?;
    t.frames(3).await?;
    t.check(t.c().ui_rect("text:Toggle sneak").is_none(), "input options out of the Settings");
    t.c().close_screen();
    t.no_screen().await?;
    t.c().open_screen(&screens::CONTROLS);
    t.screen("core:controls").await?;
    t.frames(3).await?;
    t.shot("controls").await?;
    t.laid_out(&["text:Invert mouse Y", "text:Toggle sneak", "text:Toggle sprint"]);
    t.check(t.click_ui("text:Toggle sneak", 0), "toggle sneak in Controls");
    t.until("toggle sneak on", Duration::from_secs(2), |_| TOGGLE_SNEAK.get()).await?;
    TOGGLE_SNEAK.set(false);
    t.c().close_screen();
    t.no_screen().await?;
    t.c().look(0.0, 0.0);

    // Blocks walked in a second of holding forward.
    let walk = async || -> Result<f64, String> {
        let a = t.state().pos;
        t.hold("core:forward", 1.0);
        t.secs(1.2).await?;
        Ok(((t.state().pos[0] - a[0]).powi(2) + (t.state().pos[2] - a[2]).powi(2)).sqrt())
    };
    let plain = walk().await?;
    t.note(format!("walking: {plain:.2}"));
    let fast = |d: f64| d > plain * 1.15;

    // A double tap sprints as long as forward stays down.
    t.press("core:forward");
    t.frames(3).await?;
    let d = walk().await?;
    t.probe("double tap, against walking", (d / plain) as f32, fast(d));
    let d = walk().await?;
    t.probe("after letting go of forward, against walking", (d / plain) as f32, !fast(d));

    // Toggle sprint: the key switches it on and off, also while still held.
    TOGGLE_SPRINT.set(true);
    t.frames(2).await?;
    t.press("core:sprint");
    t.frames(4).await?;
    let d = walk().await?;
    t.probe("toggled sprint, against walking", (d / plain) as f32, fast(d));
    t.press("core:sprint");
    t.frames(4).await?;
    let d = walk().await?;
    t.probe("toggled off, against walking", (d / plain) as f32, !fast(d));
    // Held for a stack drop, the same key switches nothing.
    t.hold("core:sprint", 0.3);
    t.frames(3).await?;
    t.press("core:drop");
    t.secs(0.5).await?;
    let d = walk().await?;
    t.probe("after dropping a stack, against walking", (d / plain) as f32, !fast(d));
    TOGGLE_SPRINT.set(false);

    // Toggle sneak: one press slows the walk until the next.
    TOGGLE_SNEAK.set(true);
    t.frames(2).await?;
    t.press("core:sneak");
    t.frames(4).await?;
    let d = walk().await?;
    t.probe("toggled sneak, against walking", (d / plain) as f32, d < plain * 0.5);
    t.press("core:sneak");
    t.frames(4).await?;
    let d = walk().await?;
    t.probe("toggled off, against walking", (d / plain) as f32, d > plain * 0.8);
    TOGGLE_SNEAK.set(false);
    Ok(())
}

pub async fn crafting_table_tags(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let table = add(f, [2, 0, 0]);
    t.set(table, "crafting_table").await?;
    t.op("put me 0 birch_planks 8").await?;
    t.look_at(table);
    t.frames(2).await?;
    t.press("core:use");
    t.screen("core:menu").await?;
    // Crafting table menu: 0-8 grid, 9 result, 10-36 main, 37-45 hotbar.
    let planks = item("base:birch_planks");
    t.click_ui("slot:37", 0);
    t.until("planks on the cursor", Duration::from_secs(5), |t| t.c().cursor_stack().item == planks).await?;
    t.click_ui("slot:0", 1);
    t.frames(3).await?;
    t.click_ui("slot:3", 1);
    let stick = item("base:stick");
    let made = t.until("sticks in the result", Duration::from_secs(5), |t| t.c().menu_slot(9).item == stick).await;
    t.check(made.is_ok(), "two planks in a column give no sticks (tag ingredient #planks)");
    t.c().close_screen();
    t.no_screen().await
}

/// Breaking blocks with and without the right tool: drops, pick up, tool wear.
pub async fn mining_and_wear(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 stone_pickaxe 1").await?;
    t.c().select(0);
    let f = t.feet();
    let stone = add(f, [2, 0, 0]);
    t.set(stone, "stone").await?;
    t.look_at(stone);
    t.frames(2).await?;
    // A stone pickaxe takes 0.56 s; letting go right after keeps the block behind intact.
    t.hold("core:attack", 0.8);
    t.secs(1.2).await?;
    t.check_eq(t.block(stone).await?, "core:air".into(), "stone mined");
    t.check_eq(t.slot("me", 0).await?.2, 1, "pickaxe wear");
    t.c().teleport([stone[0] as f64 + 0.5, stone[1] as f64, stone[2] as f64 + 0.5]);
    let cobble = item("base:cobblestone");
    let picked = t.until("cobblestone picked up", Duration::from_secs(5), |t| (0..36).any(|i| t.c().inventory_slot(i).item == cobble)).await;
    t.check(picked.is_ok(), "no cobblestone after mining stone with a pickaxe");
    // Dirt by hand drops dirt.
    t.c().select(4);
    let dirt = add(f, [0, 0, 2]);
    t.set(dirt, "dirt").await?;
    t.look_at(dirt);
    t.frames(2).await?;
    t.hold("core:attack", 0.9);
    t.secs(1.4).await?;
    t.check_eq(t.block(dirt).await?, "core:air".into(), "dirt dug by hand");
    t.c().teleport([dirt[0] as f64 + 0.5, dirt[1] as f64, dirt[2] as f64 + 0.5]);
    let d = item("base:dirt");
    let got = t.until("dirt picked up", Duration::from_secs(5), |t| (0..36).any(|i| t.c().inventory_slot(i).item == d)).await;
    t.check(got.is_ok(), "digging dirt by hand gave nothing");
    // A break an interceptor refuses puts the block back on the client too.
    let gold = add(f, [-2, 0, 0]);
    t.set(gold, "gold_block").await?;
    t.op("put me 1 diamond_pickaxe 1").await?;
    t.c().select(1);
    t.cmd("/rule testkit:protect_gold 1").await?;
    t.look_at(gold);
    t.frames(2).await?;
    t.hold("core:attack", 1.0);
    t.secs(1.5).await?;
    t.cmd("/rule testkit:protect_gold 0").await?;
    t.check_eq(t.block(gold).await?, "base:gold_block".into(), "protected gold on the server");
    t.sync_blocks(gold, gold).await?;
    Ok(())
}

/// Shift-click puts armor into its slots; hits wear it.
pub async fn armor(t: T) -> R {
    t.flat().await?;
    for (slot, name) in [(0, "iron_helmet"), (1, "diamond_chestplate"), (2, "gold_leggings"), (3, "leather_boots")] {
        t.op(&format!("put me {slot} {name} 1")).await?;
    }
    t.press("core:inventory");
    t.screen("core:menu").await?;
    for i in 36..40 {
        t.c().key("ShiftLeft", 0.3);
        t.frames(1).await?;
        t.click_ui(&format!("slot:{i}"), 0);
        t.frames(4).await?;
    }
    t.until("armor worn", Duration::from_secs(5), |t| (36..40).all(|s| !t.c().inventory_slot(s).is_empty())).await?;
    // The player's inventory by group name: four equipment slots, all filled; the hotbar empty.
    t.check_eq(t.op("group me core:equipment").await?, "4 4".to_string(), "equipment group (slots, filled)");
    t.check_eq(t.op("group me base:head").await?, "1 1".to_string(), "the head slot");
    t.check_eq(t.op("group me hotbar").await?, "9 0".to_string(), "hotbar group");
    t.shot("worn").await?;
    t.c().close_screen();
    t.no_screen().await?;
    let f = t.feet();
    t.op(&format!("spawn zombie {}", at(add(f, [1, 0, 1])))).await?;
    t.until("hit by the zombie", Duration::from_secs(15), |t| t.state().health < 20.0).await?;
    t.secs(0.5).await?;
    let mut worn = 0;
    for s in 36..40 {
        worn += t.slot("me", s).await?.2;
    }
    t.check(worn > 0, "armor took no damage from a zombie hit");
    let hp = t.state().health;
    t.check(hp > 17.0, format!("armor did not reduce the hit: health {hp}"));
    Ok(())
}

/// A slot holding at most 16: shift-clicking 32 ores into the test mod's electric furnace moves
/// 16 and leaves the rest.
pub async fn slot_limit(t: T) -> R {
    t.flat().await?;
    let p = add(t.feet(), [0, 0, -2]);
    t.set(p, "testmod:electric_furnace").await?;
    t.op("put me 0 iron_ore 32").await?;
    t.op(&format!("menu testmod:electric_furnace {} {} {}", p[0], p[1], p[2])).await?;
    t.screen("core:menu").await?;
    // Menu slots: 0 input, 1 output, 2-28 main inventory, 29-37 hotbar.
    t.c().key("ShiftLeft", 0.3);
    t.frames(1).await?;
    t.click_ui("slot:29", 0);
    t.until("ores in the furnace", Duration::from_secs(5), |t| t.c().menu_slot(0).count > 0).await?;
    t.frames(4).await?;
    t.check_eq(t.c().menu_slot(0).count, 16, "the input slot holds 16");
    t.check_eq(t.c().menu_slot(29).count, 16, "the rest stays in the hotbar");
    t.c().close_screen();
    t.no_screen().await
}

/// A slot taking only some items: the test mod's generator burns fuel, so shift-clicked dirt
/// stays with the player and coal goes in.
pub async fn slot_accepts(t: T) -> R {
    t.flat().await?;
    let p = add(t.feet(), [0, 0, -2]);
    t.set(p, "testmod:generator").await?;
    t.op("put me 0 dirt 16").await?;
    t.op("put me 1 coal 8").await?;
    t.op(&format!("menu testmod:generator {} {} {}", p[0], p[1], p[2])).await?;
    t.screen("core:menu").await?;
    // Menu slots: 0 the generator's, 1-27 main inventory, 28-36 hotbar.
    for slot in [28, 29] {
        t.c().key("ShiftLeft", 0.3);
        t.frames(1).await?;
        t.click_ui(&format!("slot:{slot}"), 0);
        t.frames(4).await?;
    }
    let at = format!("{} {} {}", p[0], p[1], p[2]);
    // It starts burning one at once.
    t.until("coal in the generator", Duration::from_secs(5), |t| t.c().menu_slot(0).count >= 7).await?;
    t.check_eq(t.slot(&at, 0).await?.0, "base:coal".to_string(), "the generator's slot");
    t.check_eq(t.slot("me", 9).await?, ("base:dirt".into(), 16, 0), "dirt moved to the main inventory");
    // Its fuel slot only takes: a click or a shift-click gets nothing back out.
    let before = t.c().menu_slot(0).count;
    t.click_ui("slot:0", 0);
    t.frames(4).await?;
    t.check(t.c().cursor_stack().is_empty(), "nothing comes out of an input slot");
    t.c().key("ShiftLeft", 0.3);
    t.frames(1).await?;
    t.click_ui("slot:0", 0);
    t.frames(4).await?;
    t.check(t.c().menu_slot(0).count + 1 >= before, "a shift-click doesn't take it out either");
    t.c().close_screen();
    t.no_screen().await
}

/// Pouring and scooping water.
pub async fn bucket(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 water_bucket 1").await?;
    t.c().select(0);
    let f = t.feet();
    // A one block hole: the source cannot spread, so it is still there to scoop.
    let water = add(f, [1, -1, 0]);
    t.set(water, "air").await?;
    let ground = add(water, [0, -1, 0]);
    t.look_at(ground);
    t.frames(2).await?;
    t.press("core:use");
    t.until("water poured", Duration::from_secs(5), |t| t.c().inventory_slot(0).item == item("base:bucket")).await?;
    t.check_eq(t.block(water).await?, "base:water[level=0]".into(), "water source");
    t.secs(1.5).await?;
    t.shot("poured").await?;
    t.look_at(water);
    t.frames(2).await?;
    t.press("core:use");
    t.until("water scooped", Duration::from_secs(5), |t| t.c().inventory_slot(0).item == item("base:water_bucket")).await?;
    Ok(())
}

/// An ingot in the inventory and a placed torch each make an advancement, the client gets them
/// (a toast, the list on L), and `/advancements reset` takes them back.
pub async fn advancements(t: T) -> R {
    fn made(t: &T) -> u64 {
        t.c().view().component(&base_api::ADVANCEMENTS).map_or(0, |a| a.done)
    }
    t.flat().await?;
    t.check_eq(made(&t), 0, "a new player's advancements");
    t.op("put me 0 iron_ingot 1").await?;
    t.until("advancement for the ingot", Duration::from_secs(5), |t| made(t).count_ones() == 1).await?;
    t.secs(0.15).await?;
    t.shot("toast_sliding").await?;
    t.secs(0.6).await?;
    t.shot("toast").await?;
    t.op("put me 1 torch 1").await?;
    t.c().select(1);
    t.look_at(add(t.feet(), [1, -1, 0]));
    t.frames(2).await?;
    t.press("core:use");
    t.until("advancement for the torch", Duration::from_secs(5), |t| made(t).count_ones() == 2).await?;
    t.press("base:advancements");
    t.screen("base:advancements").await?;
    t.frames(3).await?;
    t.laid_out(&["text:Advancements", "text:Getting Started", "text:Exploration", "text:2 of 19 made"]);
    t.shot("list").await?;
    // The first tile of a group, right of its 90 pixel wide name, tells its details on hover.
    if let Some([x, y, w, h]) = t.c().ui_rect("text:Making Progress") {
        let st = t.state();
        t.c().cursor(x + w + 13.0 * st.gui_scale / st.window[0] as f32, y + h / 2.0);
        t.frames(5).await?;
        t.shot("tooltip").await?;
    }
    t.c().close_screen();
    t.no_screen().await?;
    // The pause menu's button, as wide as the engine's own, opens the list too.
    t.c().open_screen(&screens::PAUSE);
    t.screen("core:pause").await?;
    t.frames(3).await?;
    t.shot("pause").await?;
    t.check(t.click_ui("text:Advancements", 0), "the pause menu's advancements button");
    t.screen("base:advancements").await?;
    // It opened on top of the pause menu, which is back once it closes.
    t.c().close_screen();
    t.screen("core:pause").await?;
    t.c().close_screen();
    t.no_screen().await?;
    t.cmd("/advancements reset").await?;
    t.until("advancements reset", Duration::from_secs(5), |t| made(t) == 0).await?;
    Ok(())
}

/// 12 blocks down costs 9 health like vanilla.
pub async fn fall_damage(t: T) -> R {
    t.flat().await?;
    t.until("full health", Duration::from_secs(5), |t| t.state().health >= 20.0).await?;
    let s = t.state();
    t.c().teleport([s.pos[0], s.pos[1] + 12.0, s.pos[2]]);
    t.until("falling", Duration::from_secs(5), |t| !t.flag(client::client_flags::ON_GROUND)).await?;
    t.until("landed", Duration::from_secs(10), |t| t.flag(client::client_flags::ON_GROUND)).await?;
    // The lowest it gets: full and saturated it heals fast, a point in half a second.
    let mut hp = f32::MAX;
    let start = std::time::Instant::now();
    while start.elapsed() < Duration::from_millis(500) {
        hp = hp.min(t.state().health);
        t.frames(1).await?;
    }
    t.check((10.0..=13.0).contains(&hp), format!("health after a 12 block fall: {hp}, vanilla 11"));
    Ok(())
}

/// The server and the client measure the player with the same box, before and after `/morph`.
pub async fn player_box(t: T) -> R {
    t.flat().await?;
    for step in ["start", "morphed", "back"] {
        match step {
            "morphed" => t.cmd("/morph base:pig").await.map(|_| ())?,
            "back" => t.cmd("/morph").await.map(|_| ())?,
            _ => {}
        }
        t.secs(0.3).await?;
        let server = t.op("size me").await?;
        let c = t.c().entity(0).ok_or("no local player")?.size;
        let client = format!("{:.3} {:.3} {:.3}", c[0], c[1], c[2]);
        t.check(server == client, format!("{step}: server box {server}, client box {client}"));
        if step == "start" {
            t.check(server.starts_with("0.600 1.800"), format!("player box is {server}, not 0.6 x 1.8"));
        }
    }
    Ok(())
}

/// Sneaking at an edge stops at it instead of falling off.
pub async fn sneak_edge(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let base = add(f, [5, 0, 5]);
    t.fill(base, add(base, [0, 2, 0]), "stone").await?;
    t.c().teleport([base[0] as f64 + 0.5, base[1] as f64 + 3.0, base[2] as f64 + 0.5]);
    t.until("on the pillar", Duration::from_secs(5), |t| t.flag(client::client_flags::ON_GROUND)).await?;
    t.c().look(30.0, 0.0);
    t.hold("core:sneak", 4.0);
    t.frames(2).await?;
    t.hold("core:forward", 2.0);
    t.secs(1.0).await?;
    t.hold("core:left", 1.0);
    t.secs(1.5).await?;
    let y = t.state().pos[1];
    t.check(y > base[1] as f64 + 2.9, format!("fell off while sneaking: y {y:.2}"));
    Ok(())
}

/// The player moves by its attributes like a mob: +100% movement speed walks about twice as far,
/// a stronger jump goes higher, and a push from the server lifts it.
pub async fn movement_attributes(t: T) -> R {
    t.flat().await?;
    t.c().look(0.0, 0.0);
    let walk = |t: T| async move {
        let start = t.state().pos;
        t.hold("core:forward", 1.0);
        t.secs(1.3).await?;
        let end = t.state().pos;
        Ok::<f64, String>(((end[0] - start[0]).powi(2) + (end[2] - start[2]).powi(2)).sqrt())
    };
    let slow = walk(t.clone()).await?;
    t.op("attr_mod me core:movement_speed testkit:fast 1 1 0").await?;
    t.secs(0.2).await?;
    let fast = walk(t.clone()).await?;
    t.note(format!("walked {slow:.2} then {fast:.2} blocks in a second"));
    t.check(fast > slow * 1.6, format!("faster with the attribute: {slow:.2} -> {fast:.2}"));
    t.op("attr_rm me core:movement_speed testkit:fast").await?;
    let jump = |t: T| async move {
        let y0 = t.state().pos[1];
        let mut top = y0;
        t.hold("core:jump", 0.1);
        for _ in 0..30 {
            t.secs(0.03).await?;
            top = top.max(t.state().pos[1]);
        }
        t.secs(1.0).await?;
        Ok::<f64, String>(top - y0)
    };
    let low = jump(t.clone()).await?;
    t.op("attr_mod me core:jump_strength testkit:spring 1 0.5 0").await?;
    t.secs(0.2).await?;
    let high = jump(t.clone()).await?;
    t.note(format!("jumped {low:.2} then {high:.2}"));
    t.check(high > low * 1.4, format!("higher with the attribute: {low:.2} -> {high:.2}"));
    t.op("attr_rm me core:jump_strength testkit:spring").await?;
    let y = t.state().pos[1];
    t.op("push 0 12 0").await?;
    t.secs(0.25).await?;
    t.check(t.state().pos[1] > y + 0.3, format!("the push lifted the player: {y:.2} -> {:.2}", t.state().pos[1]));
    Ok(())
}

/// Mobs move by the player's rules: a pig pushed across ice slides further than on grass, walks
/// slower through oil, and climbs a ladder it walks into.
pub async fn mob_physics(t: T) -> R {
    t.flat().await?;
    t.cmd("/rule core:mob_spawning 0").await?;
    let f = t.feet();
    let floor = |p: [i32; 3]| add(p, [0, -1, 0]);
    // How far a pig slides after a push east, over `ground`.
    let run = |t: T, at: [i32; 3], ground: &'static str| async move {
        t.fill(floor(at), floor(add(at, [24, 0, 2])), ground).await?;
        t.fill(at, add(at, [24, 2, 2]), "air").await?;
        let id = t.op(&format!("spawn base:pig {} {} {}", at[0], at[1], at[2] + 1)).await?;
        t.secs(0.5).await?;
        let x0: f64 = t.op(&format!("entity {id}")).await?.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        t.op(&format!("push 12 0 0 {id}")).await?;
        t.secs(1.0).await?;
        let x1: f64 = t.op(&format!("entity {id}")).await?.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        t.op(&format!("remove {id}")).await?;
        Ok::<f64, String>(x1 - x0)
    };
    let grass = run(t.clone(), add(f, [-12, 0, 6]), "grass").await?;
    let ice = run(t.clone(), add(f, [-12, 0, 12]), "ice").await?;
    t.note(format!("a pig went {grass:.2} blocks on grass, {ice:.2} on ice"));
    t.check(ice > grass * 1.3, format!("it slides on ice: {grass:.2} -> {ice:.2}"));
    // Walked 30 ticks over grass, then through oil (a liquid with drag).
    let walk = |t: T, at: [i32; 3], liquid: bool| async move {
        t.fill(floor(at), floor(add(at, [24, 0, 2])), "grass").await?;
        t.fill(at, add(at, [24, 2, 2]), "air").await?;
        if liquid {
            t.fill(at, add(at, [24, 0, 2]), "testmod:oil").await?;
        }
        let id = t.op(&format!("spawn base:pig {} {} {}", at[0], at[1], at[2] + 1)).await?;
        t.secs(0.5).await?;
        let x0: f64 = t.op(&format!("entity {id}")).await?.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        t.op(&format!("drive {id} 1 0 30")).await?;
        t.secs(1.5).await?;
        let x1: f64 = t.op(&format!("entity {id}")).await?.split_whitespace().nth(1).and_then(|v| v.parse().ok()).unwrap_or(0.0);
        t.op(&format!("remove {id}")).await?;
        Ok::<f64, String>(x1 - x0)
    };
    let dry = walk(t.clone(), add(f, [-12, 0, 18]), false).await?;
    let oil = walk(t.clone(), add(f, [-12, 0, 24]), true).await?;
    t.note(format!("walked {dry:.2} blocks on grass, {oil:.2} through oil"));
    t.check(oil < dry * 0.75, format!("slower through oil: {dry:.2} -> {oil:.2}"));
    // A wall with a ladder: walked into it, the pig goes up.
    let wall = add(f, [6, 0, -6]);
    t.fill(add(wall, [0, 0, -1]), add(wall, [0, 5, 1]), "stone").await?;
    t.fill(add(wall, [-1, 0, -1]), add(wall, [-1, 5, 1]), "base:ladder[facing=west]").await?;
    let id = t.op(&format!("spawn base:pig {} {} {}", wall[0] - 2, wall[1], wall[2])).await?;
    t.secs(0.5).await?;
    let y0: f64 = t.op(&format!("entity {id}")).await?.split_whitespace().nth(2).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    t.op(&format!("drive {id} 1 0 60")).await?;
    t.secs(3.0).await?;
    let y1: f64 = t.op(&format!("entity {id}")).await?.split_whitespace().nth(2).and_then(|v| v.parse().ok()).unwrap_or(0.0);
    t.note(format!("the pig on the ladder: y {y0:.2} -> {y1:.2}"));
    t.check(y1 > y0 + 1.5, format!("it climbs the ladder: {y0:.2} -> {y1:.2}"));
    Ok(())
}

pub async fn drop_and_pickup(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 cobblestone 10").await?;
    t.c().select(0);
    t.c().look(0.0, 0.0);
    t.frames(2).await?;
    t.press("core:drop");
    t.until("one dropped", Duration::from_secs(5), |t| t.c().inventory_slot(0).count == 9).await?;
    t.check(t.count_near("core:item", 8.0).await? == 1, "no item entity after dropping");
    t.secs(2.2).await?;
    let item = t.c().entities().first().copied();
    t.hold("core:forward", 1.0);
    // The client lets the taken item fly to the player for three ticks before it goes.
    let mut flying = false;
    t.until("picked up again", Duration::from_secs(6), |t| {
        let got = t.c().inventory_slot(0).count == 10;
        flying |= got && item.is_some_and(|id| t.c().entity(id).is_some());
        got
    })
    .await?;
    t.check(flying, "the taken item was still on its way when it reached the inventory");
    t.secs(0.3).await?;
    t.check(item.is_none_or(|id| t.c().entity(id).is_none()), "the taken item is gone");
    Ok(())
}

/// Breaking and placing play the block's sounds (from the server), walking plays steps, and
/// `/music` starts a track. Only checks that sounds start; without a sound device it is skipped.
pub async fn sounds(t: T) -> R {
    t.flat().await?;
    let before = t.state().sounds;
    t.op("play testmod:boom").await?;
    if t.until("a sound", std::time::Duration::from_secs(2), |t| t.state().sounds > before).await.is_err() {
        t.note("no sound device here, sound checks skipped");
        return Ok(());
    }
    t.cmd("/music").await?;
    t.until("music", std::time::Duration::from_secs(3), |t| t.flag(weft_sdk::client::client_flags::MUSIC)).await?;
    Ok(())
}

/// Creative tools never wear, and invulnerable entities (the test mod's crate) take no damage
/// from hits.
pub async fn creative_and_invulnerable(t: T) -> R {
    t.flat().await?;
    t.op("mode creative").await?;
    t.op("put me 0 stone_pickaxe 1").await?;
    t.c().select(0);
    let f = t.feet();
    let stone = add(f, [2, 0, 0]);
    t.set(stone, "stone").await?;
    t.look_at(stone);
    t.frames(2).await?;
    t.hold("core:attack", 0.3);
    t.secs(0.6).await?;
    t.check_eq(t.block(stone).await?, "core:air".into(), "stone broken in creative");
    t.check_eq(t.slot("me", 0).await?.2, 0, "no pickaxe wear in creative");
    t.op("mode survival").await?;
    let id = t.op(&format!("spawn testmod:crate {} {} {}", stone[0], stone[1], stone[2])).await?;
    t.look_at(stone);
    t.frames(2).await?;
    t.hold("core:attack", 1.5);
    t.secs(2.0).await?;
    let health = t.op(&format!("entity {}", id.trim())).await?;
    t.check(health.ends_with(" 10"), format!("the crate kept its health ({health})"));
    // The same hits do hurt an ordinary entity there.
    t.op(&format!("remove {}", id.trim())).await?;
    let idol = t.op(&format!("spawn testmod:idol {} {} {}", stone[0], stone[1], stone[2])).await?;
    let before = t.op(&format!("entity {}", idol.trim())).await?;
    t.look_at(stone);
    t.frames(2).await?;
    t.hold("core:attack", 1.5);
    t.secs(2.0).await?;
    let after = t.op(&format!("entity {}", idol.trim())).await?;
    let hp = |s: &str| s.rsplit(' ').next().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
    t.check(hp(&after) < hp(&before), format!("an idol is hurt by the same hits ({before} -> {after})"));
    Ok(())
}

/// Every creative click breaks a block, also at over 1000 frames a second (instant breaks once
/// needed a frame longer than 1 ms), and a held button keeps breaking.
pub async fn creative_clicks(t: T) -> R {
    t.flat().await?;
    t.op("mode creative").await?;
    weft_sdk::settings::VSYNC.set(false);
    let r = creative_clicks_fast(&t).await;
    weft_sdk::settings::VSYNC.set(true);
    r
}

async fn creative_clicks_fast(t: &T) -> R {
    let f = t.feet();
    let row = [add(f, [2, 1, 0]), add(f, [3, 1, 0]), add(f, [4, 1, 0])];
    let build = async || {
        for b in row {
            t.set(b, "stone").await?;
        }
        t.until("the row on the client", Duration::from_secs(2), |t| row.iter().all(|&b| t.c().block(b).is_some_and(|s| s != 0))).await?;
        t.look_at(row[0]);
        t.frames(2).await
    };
    let gone = |t: &T, i: usize| t.c().block(row[i]) == Some(0);
    build().await?;
    for i in 0..row.len() {
        t.press("core:attack");
        t.until(&format!("click {} breaks a block", i + 1), Duration::from_secs(1), |t| gone(t, i)).await?;
        t.secs(0.3).await?;
    }
    for b in row {
        t.check_eq(t.block(b).await?, "core:air".into(), "the server broke it as well");
    }
    build().await?;
    t.hold("core:attack", 1.0);
    t.until("holding breaks the whole row", Duration::from_secs(2), |t| (0..row.len()).all(|i| gone(t, i))).await
}

/// Food: eating raises hunger, and another mod's component on the base game's food plus the
/// engine's `item_used` event heals (the owner's example).
pub async fn food(t: T) -> R {
    t.flat().await?;
    let s = t.state();
    t.c().teleport([s.pos[0], s.pos[1] + 8.0, s.pos[2]]);
    t.until("falling", Duration::from_secs(5), |t| !t.flag(client::client_flags::ON_GROUND)).await?;
    t.until("landed", Duration::from_secs(10), |t| t.flag(client::client_flags::ON_GROUND)).await?;
    t.until("hurt", Duration::from_secs(3), |t| t.state().health < 20.0).await?;
    let before = t.state().health;
    t.cmd("/feed 10").await?;
    t.op("put me 0 cooked_beef 3").await?;
    t.c().select(0);
    t.until("hunger synced", Duration::from_secs(5), |t| t.c().view().component(&HUNGER).is_some_and(|h| h.food == 10.0)).await?;
    t.secs(0.3).await?;
    t.hold("core:use", 2.0);
    t.until("ate", Duration::from_secs(5), |t| t.c().inventory_slot(0).count == 2).await?;
    t.until("food went up", Duration::from_secs(5), |t| t.c().view().component(&HUNGER).is_some_and(|h| h.food == 18.0)).await?;
    t.until("healed", Duration::from_secs(3), |t| t.state().health >= before + 2.0).await?;
    let after = t.state().health;
    t.check(after <= before + 3.0, format!("health {before} -> {after}: two from testmod's food_heal (plus maybe a regen tick)"));
    Ok(())
}

/// A battery has no tick: only the grid changes its energy, and its open menu still sees it.
pub async fn battery(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let g = add(f, [2, 0, -3]);
    t.set(g, "testmod:generator").await?;
    t.set(add(g, [1, 0, 0]), "testmod:cable").await?;
    let b = add(g, [2, 0, 0]);
    t.set(b, "testmod:battery").await?;
    t.op(&format!("insert {} coal 4", at(g))).await?;
    t.look_at(b);
    t.frames(2).await?;
    t.press("core:use");
    t.screen("core:menu").await?;
    let stored = |t: &T| t.c().view().menu_component(&ENERGY).map_or(0, |e| e.stored);
    let first = stored(&t);
    t.until("the battery's bar moves", Duration::from_secs(5), |t| stored(t) > first).await?;
    t.check(t.c().view().menu_component(&ENERGY).is_some_and(|e| e.capacity == 100_000), "the battery starts with its own capacity");
    t.c().close_screen();
    t.no_screen().await
}

/// Lava hurts the player, who keeps burning after getting out.
pub async fn burning(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let pool = add(f, [6, -1, 0]);
    t.fill(pool, add(pool, [1, 0, 1]), "lava").await?;
    t.c().teleport([pool[0] as f64 + 0.5, pool[1] as f64 + 0.1, pool[2] as f64 + 0.5]);
    t.until("hurt by lava", Duration::from_secs(5), |t| t.state().health <= 16.0).await?;
    t.c().teleport([f[0] as f64 + 0.5, f[1] as f64, f[2] as f64 + 0.5]);
    t.secs(0.5).await?;
    let out = t.state().health;
    t.secs(2.5).await?;
    let later = t.state().health;
    t.check(later < out, format!("still burning after leaving lava: health {out} -> {later}"));
    Ok(())
}

static ON_FIRE: Component<u8> = Component::named("core:burning");

/// Fields the engine keeps on entities reach clients like any synced component: a pig in lava
/// shows on fire and losing health, and stays for its death animation once the server drops it.
pub async fn entity_fields(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    t.fill(add(f, [3, 0, -1]), add(f, [5, 1, 1]), "stone").await?;
    t.fill(add(f, [4, 0, 0]), add(f, [4, 1, 0]), "air").await?;
    t.fill(add(f, [4, -2, 0]), add(f, [4, -1, 0]), "lava").await?;
    let id = t.op(&format!("spawn pig {}", at(add(f, [4, -1, 0])))).await?;
    let id: u64 = id.trim().parse().map_err(|_| format!("spawn pig: {id}"))?;
    t.until("the pig burns on the client", Duration::from_secs(3), |t| t.c().view().component_of(id, &ON_FIRE) == Some(1)).await?;
    t.until("the pig loses health on the client", Duration::from_secs(3), |t| t.c().entity(id).is_some_and(|e| e.health < e.max_health)).await?;
    for _ in 0..400 {
        if t.op(&format!("entity {id}")).await.is_err() {
            t.check(t.c().entity(id).is_some(), "the killed pig stays on the client for its death animation");
            return Ok(());
        }
    }
    Err("the pig did not die in lava".into())
}

/// A machine's window is titled with its block's name; a renamed chest keeps its own name from
/// the stack to the placed block (its title) and back to the item when broken.
pub async fn own_names(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let furnace = add(f, [2, 0, 0]);
    t.set(furnace, "furnace").await?;
    t.look_at(furnace);
    t.frames(2).await?;
    t.press("core:use");
    t.screen("core:menu").await?;
    t.frames(3).await?;
    t.check(t.c().ui_rect("text:Furnace").is_some(), "the furnace window is titled with the block's name");
    t.c().close_screen();
    t.no_screen().await?;
    t.op("put me 0 chest 1").await?;
    t.c().select(0);
    t.cmd("/rename Loot Box").await?;
    t.until("the stack renamed", Duration::from_secs(3), |t| weft_sdk::stack_name(&t.c().inventory_slot(0)).as_deref() == Some("Loot Box")).await?;
    let chest = add(f, [0, 0, -2]);
    t.look_at(add(chest, [0, -1, 0]));
    t.frames(2).await?;
    t.press("core:use");
    t.until("the chest placed", Duration::from_secs(3), |t| t.c().inventory_slot(0).is_empty()).await?;
    t.look_at(chest);
    // Past the right click delay of the placing click.
    t.secs(0.4).await?;
    t.press("core:use");
    t.screen("core:menu").await?;
    t.frames(3).await?;
    t.check(t.c().ui_rect("text:Loot Box").is_some(), "the chest window shows its own name");
    t.c().close_screen();
    t.no_screen().await?;
    t.op(&format!("break {} wood_axe", at(chest))).await?;
    t.c().teleport([chest[0] as f64 + 0.5, chest[1] as f64, chest[2] as f64 + 0.5]);
    let back = t.until("the chest picked up", Duration::from_secs(5), |t| (0..36).any(|i| weft_sdk::stack_name(&t.c().inventory_slot(i)).as_deref() == Some("Loot Box"))).await;
    t.check(back.is_ok(), "the broken chest's item has its name again");
    Ok(())
}

/// Respawning: every player has variable-sized components (hunger's timer), which once crashed
/// the server on any death. What belongs to the body starts over (hunger, effects); mods' timers
/// go on, or hunger's pulse, armed once on joining, would stop for good.
pub async fn respawn(t: T) -> R {
    t.flat().await?;
    t.op("mode survival").await?;
    for with_effect in [false, true] {
        t.cmd("/feed 5").await?;
        t.until("food down", Duration::from_secs(5), |t| t.c().view().component(&HUNGER).is_some_and(|h| h.food <= 5.0)).await?;
        if with_effect {
            t.op("attr_mod me core:movement_speed testkit:fast 1 1 0").await?;
        }
        t.cmd("/kill").await?;
        t.screen("core:death").await?;
        t.c().respawn();
        t.no_screen().await?;
        t.until("hunger starts over", Duration::from_secs(5), |t| t.c().view().component(&HUNGER).is_some_and(|h| h.food == 20.0)).await?;
        let speed = t.op("attr me core:movement_speed").await?;
        t.check(!speed.contains("testkit:fast"), format!("no effect after respawning: {speed}"));
        t.check_eq(t.op("health").await?, "20".to_string(), "full health again");
    }
    // Starving takes health at hunger's next pulse (every 4 seconds).
    t.cmd("/feed 0").await?;
    t.secs(4.5).await?;
    let health: f32 = t.op("health").await?.parse().unwrap_or(20.0);
    t.check(health < 20.0, format!("hunger's pulse still runs: health {health}"));
    Ok(())
}

/// Modern clicks: every press acts at once however fast it follows the last, and only holding the
/// button waits between repeats.
pub async fn presses_never_wait(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 stone 64").await?;
    t.c().select(0);
    weft_sdk::settings::VSYNC.set(false);
    let r = presses_fast(&t).await;
    weft_sdk::settings::VSYNC.set(true);
    r
}

async fn presses_fast(t: &T) -> R {
    let f = t.feet();
    for x in 2..=3 {
        t.set(add(f, [x, 0, 0]), "stone").await?;
    }
    let count = |t: &T| t.c().inventory_slot(0).count;
    t.look_at(add(f, [3, 0, 0]));
    t.frames(3).await?;
    t.press("core:use");
    // A key needs a frame up between presses, and this is far less than the 0.2 s of holding.
    t.secs(0.07).await?;
    t.press("core:use");
    t.until("two quick presses place two blocks", Duration::from_secs(2), |t| count(t) <= 62).await?;
    t.check(count(t) == 62, format!("two presses placed {} blocks", 64 - count(t)));
    // Held, it repeats about 5 times a second.
    t.secs(0.5).await?;
    let before = count(t);
    t.c().look(45.0, 40.0);
    t.hold("core:use", 1.0);
    t.secs(1.2).await?;
    let held = before - count(t);
    t.note(format!("held for a second: {held} uses"));
    t.check(held <= 6, format!("holding waits between repeats: {held} in a second"));
    Ok(())
}

/// Placing a block over a plant replaces it and drops what it would when broken, except in
/// creative.
pub async fn replaced_plants_drop(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let flower = add(f, [2, 0, 0]);
    t.op("put me 0 stone 2").await?;
    t.c().select(0);
    for creative in [false, true] {
        if creative {
            t.op("mode creative").await?;
        }
        t.set(flower, "dandelion").await?;
        t.until("the flower on the client", Duration::from_secs(2), |t| t.c().block(flower).is_some_and(|s| s != 0)).await?;
        t.look_at(flower);
        t.frames(2).await?;
        t.press("core:use");
        t.until("stone over the flower", Duration::from_secs(2), |t| t.c().block(flower).is_some_and(|s| Block::named("base:stone").is(s))).await?;
        t.secs(0.2).await?;
        let near = t.op(&format!("near {} {} {} 2", flower[0], flower[1], flower[2])).await?;
        if creative {
            t.check(!near.contains("core:item"), format!("nothing drops in creative: {near}"));
        } else {
            let d = t.op("inv me").await?;
            t.check(near.contains("core:item") || d.contains("dandelion"), format!("the flower dropped: near '{near}', inventory '{d}'"));
        }
        t.set(flower, "air").await?;
        t.secs(0.5).await?;
        t.cmd("/kill @e[type=core:item]").await?;
    }
    Ok(())
}

/// Sprinting under water swims in a low box at about 3.9 blocks a second, where wading does 2;
/// lava is not swimmable, unless the attribute says so.
pub async fn swimming(t: T) -> R {
    t.flat().await?;
    t.op("mode creative").await?;
    let f = t.feet();
    let (a, b) = (add(f, [-8, 0, -40]), add(f, [8, 12, 8]));
    let pool = |name: &'static str| {
        let t = t.clone();
        async move { t.fill(a, b, name).await }
    };
    let sprint_speed = |t: T, pitch: f32| async move {
        let s = t.state().pos;
        t.c().teleport([s[0], f[1] as f64 + 8.0, f[2] as f64 + 0.5]);
        t.secs(0.3).await?;
        t.c().look(0.0, pitch);
        t.hold("core:sprint", 2.0);
        t.hold("core:forward", 2.0);
        t.secs(1.0).await?;
        let from = t.state().pos;
        t.secs(0.8).await?;
        let to = t.state().pos;
        t.secs(1.0).await?;
        Ok::<(f64, f64), String>((((to[0] - from[0]).powi(2) + (to[2] - from[2]).powi(2)).sqrt() / 0.8, to[1] - from[1]))
    };
    pool("water").await?;
    t.secs(0.5).await?;
    let (swim, _) = sprint_speed(t.clone(), 0.0).await?;
    t.note(format!("sprinting under water: {swim:.2} blocks/s"));
    t.check((3.3..4.6).contains(&swim), format!("swimming speed {swim:.2}, vanilla's code gives 3.9"));
    let (_, down) = sprint_speed(t.clone(), -60.0).await?;
    t.note(format!("looking down, moved {down:.2} in y"));
    t.check(down < -0.3, format!("swimming goes where it looks: y {down:.2}"));
    pool("lava").await?;
    t.secs(0.5).await?;
    let (wade, _) = sprint_speed(t.clone(), 0.0).await?;
    t.note(format!("sprinting in lava: {wade:.2} blocks/s"));
    t.check(wade < 1.8, format!("no swimming in lava: {wade:.2} blocks/s"));
    t.op("attr_mod me core:swim_anywhere testkit:swims 0 1 0").await?;
    t.secs(0.2).await?;
    let (suit, _) = sprint_speed(t.clone(), 0.0).await?;
    t.note(format!("with the attribute: {suit:.2} blocks/s"));
    t.check(suit > wade * 1.3, format!("the attribute swims in lava: {wade:.2} -> {suit:.2}"));
    t.op("attr_rm me core:swim_anywhere testkit:swims").await?;
    pool("air").await?;
    Ok(())
}

/// Modern combat: a hit right after the last does a fraction, a charged one all of it, and a
/// stronger hit during the target's recovery still adds the difference.
pub async fn attack_charge(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 iron_sword 1").await?;
    t.c().select(0);
    let f = t.feet();
    let spot = add(f, [0, 0, -2]);
    // Walled in with the player, or it would run off from the first hit.
    for (from, to) in [([-1, 0, -3], [-1, 2, 1]), ([1, 0, -3], [1, 2, 1]), ([0, 0, -3], [0, 2, -3]), ([0, 0, 1], [0, 2, 1])] {
        t.fill(add(f, from), add(f, to), "stone").await?;
    }
    let id = t.op(&format!("spawn base:cow {} {} {}", spot[0], spot[1], spot[2])).await?;
    let health = |t: T, id: String| async move {
        let s = t.op(&format!("entity {id}")).await?;
        Ok::<f32, String>(s.split_whitespace().last().and_then(|v| v.parse().ok()).unwrap_or(-1.0))
    };
    t.look_at(spot);
    // The sword just came to hand: nearly no charge, and the hand comes up over the next second.
    t.secs(0.2).await?;
    t.c().select(1);
    t.c().select(0);
    t.frames(2).await?;
    let before = health(t.clone(), id.clone()).await?;
    t.press("core:attack");
    t.secs(0.15).await?;
    let weak = before - health(t.clone(), id.clone()).await?;
    t.note(format!("uncharged: {weak:.2}"));
    t.check((0.1..2.5).contains(&weak), format!("a hit with no charge does little: {weak:.2} of 6"));
    t.secs(1.5).await?;
    let before = health(t.clone(), id.clone()).await?;
    t.press("core:attack");
    t.secs(0.15).await?;
    let full = before - health(t.clone(), id.clone()).await?;
    t.note(format!("charged: {full:.2}"));
    t.check((5.5..6.5).contains(&full), format!("a charged iron sword does 6: {full:.2}"));
    Ok(())
}

/// One level for everything: below what a command asks for it refuses and isn't offered, and a
/// player can't give out more than it has.
pub async fn permissions(t: T) -> R {
    t.flat().await?;
    t.check(t.cmd("/time query").await?.contains("time is"), "the host may do it all");
    t.op("put me 0 stone 1").await?;
    t.c().select(0);
    t.secs(0.1).await?;
    t.cmd("/op @s 0").await?;
    let no = t.cmd("/time set 1000").await?;
    t.check(no.contains("permission level 2"), format!("/time at level 0: {no}"));
    t.cmd("/rename shiny").await?;
    t.until("a command for everyone still works", Duration::from_secs(2), |t| t.c().inventory_slot(0).data != 0).await?;
    let up = t.cmd("/op @s 4").await?;
    t.check(up.contains("permission level 3"), format!("/op from level 0: {up}"));
    Ok(())
}

/// A sword doesn't break blocks in creative, as in modern; other things do.
pub async fn creative_sword(t: T) -> R {
    t.flat().await?;
    t.op("mode creative").await?;
    t.op("put me 0 iron_sword 1").await?;
    t.op("put me 1 iron_pickaxe 1").await?;
    t.c().select(0);
    let b = add(t.feet(), [2, 0, 0]);
    t.set(b, "stone").await?;
    t.until("the block on the client", Duration::from_secs(2), |t| t.c().block(b).is_some_and(|s| s != 0)).await?;
    t.look_at(b);
    t.frames(3).await?;
    t.press("core:attack");
    t.secs(0.4).await?;
    t.check_eq(t.block(b).await?, "base:stone".into(), "a sword leaves it");
    t.check(t.c().block(b).is_some_and(|s| s != 0), "and the client still shows it");
    t.c().select(1);
    t.secs(0.1).await?;
    t.press("core:attack");
    t.until("a pickaxe breaks it", Duration::from_secs(2), |t| t.c().block(b) == Some(0)).await
}

/// Advancements go by tags: new food and mobs count without the list knowing them.
pub async fn advancements_by_tags(t: T) -> R {
    use weft_sdk::Tag;
    fn made(t: &T) -> u64 {
        t.c().view().component(&base_api::ADVANCEMENTS).map_or(0, |a| a.done)
    }
    for food in ["base:cooked_beef", "base:cooked_porkchop", "base:cooked_chicken", "base:cooked_mutton"] {
        t.check(Item::lookup(food).is_some_and(|i| Tag::named("cooked_meat").on_item(i)), format!("{food} is cooked meat"));
    }
    t.flat().await?;
    t.cmd("/feed 5").await?;
    t.op("put me 0 cooked_mutton 1").await?;
    t.c().select(0);
    t.c().look(0.0, 60.0);
    t.secs(0.2).await?;
    let before = made(&t);
    t.hold("core:use", 2.5);
    t.until("eating mutton counts", Duration::from_secs(5), |t| made(t).count_ones() > before.count_ones()).await?;
    let f = t.feet();
    let chicken = t.op(&format!("spawn base:chicken {} {} {}", f[0] + 3, f[1], f[2])).await?;
    t.secs(0.2).await?;
    let before = made(&t);
    t.op(&format!("hurt {} 100", chicken.trim())).await?;
    t.until("killing a chicken counts", Duration::from_secs(3), |t| made(t).count_ones() > before.count_ones()).await
}

/// Every item and mob of the base game has its name in every language it comes with.
pub async fn names_translated(t: T) -> R {
    // From a process of several, the run folder is deeper down.
    let run = t.c().run_dir();
    let Some(game) = std::path::Path::new(&run).ancestors().find(|d| d.join("mods/base/assets/lang").is_dir()) else { return Err("no base mod sources".into()) };
    let root = game.join("mods/base/assets/lang").display().to_string();
    for lang in ["en_us", "ru_ru"] {
        let text = std::fs::read_to_string(format!("{root}/{lang}.lang")).map_err(|e| e.to_string())?;
        let has = |key: String| text.lines().any(|l| l.split(" = ").next() == Some(key.as_str()));
        let mut missing = Vec::new();
        for id in 1.. {
            let name = item_name(id);
            if name.is_empty() {
                break;
            }
            if let Some(n) = name.strip_prefix("base:").filter(|n| !n.ends_with("_spawn_egg")) {
                if !has(format!("item.base.{n}")) && !has(format!("block.base.{n}")) {
                    missing.push(name.to_string());
                }
            }
        }
        for id in 0.. {
            let name = name_of(weft_sdk::abi::entry_kind::ENTITY, id);
            if name.is_empty() {
                break;
            }
            if let Some(n) = name.strip_prefix("base:") {
                // Machines' entities are called what their block is.
                if !has(format!("entity.base.{n}")) && !has(format!("block.base.{n}")) {
                    missing.push(name.to_string());
                }
            }
        }
        t.check(missing.is_empty(), format!("no name in {lang}: {}", missing.join(" ")));
    }
    Ok(())
}

/// Air comes back in creative too, where nothing takes it.
pub async fn air_in_creative(t: T) -> R {
    fn air(t: &T) -> f32 {
        t.c().view().component(&base_api::HUNGER).map_or(0.0, |h| h.air)
    }
    t.flat().await?;
    let f = t.feet();
    t.fill(add(f, [-2, 0, -2]), add(f, [2, 4, 2]), "water").await?;
    t.secs(2.0).await?;
    t.check(air(&t) < 300.0, format!("air under water: {}", air(&t)));
    t.op("mode creative").await?;
    t.until("air back in creative, still under water", Duration::from_secs(5), |t| air(t) >= 300.0).await
}

/// Swimming doesn't bob the view, touching the bottom or not; walking does.
pub async fn swimming_no_bob(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    t.c().look(0.0, 0.0);
    t.hold("core:forward", 1.0);
    t.secs(0.4).await?;
    let (mut walked, mut swum, mut touching, mut samples) = (0.0f32, 0.0f32, 0, 0);
    for _ in 0..40 {
        t.frames(1).await?;
        walked = walked.max(t.state().bob);
    }
    t.check(walked > 0.02, format!("walking bobs: {walked}"));
    t.secs(1.0).await?;
    // Two deep, so swimming scrapes the bottom.
    t.fill(add(f, [-8, 0, -60]), add(f, [8, 1, 8]), "water").await?;
    t.secs(0.5).await?;
    let s = t.state().pos;
    t.c().teleport([s[0], f[1] as f64 + 0.1, f[2] as f64 + 0.5]);
    t.secs(0.3).await?;
    t.c().look(0.0, -30.0);
    t.hold("core:sprint", 4.0);
    t.hold("core:forward", 4.0);
    t.secs(1.0).await?;
    for _ in 0..120 {
        t.frames(1).await?;
        if let Some(me) = t.c().view().me().filter(|m| m.pose == abi::pose::SWIM) {
            samples += 1;
            touching += (me.on_ground != 0) as u32;
            swum = swum.max(t.state().bob);
        }
    }
    t.note(format!("swimming frames {samples}, touching the bottom {touching}, bob {swum}"));
    t.check(samples > 20, format!("swam for {samples} frames"));
    t.check(swum < 0.01, format!("swimming bobs: {swum}"));
    Ok(())
}

/// A toast goes with the GUI on F1, and doesn't follow the player out of the world.
pub async fn toast_with_the_gui(t: T) -> R {
    let toast = |s: &crate::runner::Shot| s.share(0.78, 0.0, 1.0, 0.09, |c| c[0] > 200 && c[1] > 170 && c[2] < 170);
    t.flat().await?;
    t.op("put me 0 iron_ingot 1").await?;
    t.op("put me 1 gold_ingot 1").await?;
    t.c().look(0.0, -40.0);
    t.secs(1.5).await?;
    let shown = toast(&t.shot("shown").await?);
    t.check(shown > 0.0005, format!("the toast shows: {shown}"));
    t.press("core:hide_hud");
    t.frames(3).await?;
    let hidden = toast(&t.shot("hidden").await?);
    t.check(hidden < 0.0001, format!("F1 hides it: {hidden}"));
    // Let go first, or the second press is the same one, and the GUI stays hidden for what's next.
    t.secs(0.2).await?;
    t.press("core:hide_hud");
    t.secs(0.2).await?;
    let back = toast(&t.shot("back").await?);
    t.check(back > 0.0005, format!("and it is back: {back}"));
    // The second is still waiting behind the first, and must not come in the next world.
    t.c().leave_world();
    t.screen("core:title").await?;
    t.frames(5).await?;
    let title = toast(&t.shot("title").await?);
    t.check(title < 0.0001, format!("none on the title screen: {title}"));
    t.c().open_world(&format!("test-{}", t.name()), 1);
    t.until("joined again", Duration::from_secs(60), |t| t.flag(client::client_flags::JOINED)).await?;
    t.loaded().await?;
    t.c().look(0.0, -40.0);
    t.frames(3).await?;
    let again = toast(&t.shot("again").await?);
    t.check(again < 0.0001, format!("none from the world before: {again}"));
    Ok(())
}

/// Every entity has an egg with nothing registered for it, a mod's too, unless it says otherwise.
/// One puts its mob on the side of the block it is used on, and is used up outside creative.
pub async fn spawn_eggs(t: T) -> R {
    t.check(Item::lookup("testmod:wisp_spawn_egg").is_some(), "a mod's mob has one");
    t.check(Item::lookup("base:arrow_spawn_egg").is_none(), "no_egg() has none");
    t.check(Item::lookup("base:furnace_spawn_egg").is_none(), "a machine's block entity has none");
    t.flat().await?;
    let f = t.feet();
    let ground = add(f, [2, -1, 0]);
    t.look_at(ground);
    t.op("put me 0 pig_spawn_egg 2").await?;
    t.c().select(0);
    t.secs(0.2).await?;
    t.press("core:use");
    t.secs(0.5).await?;
    t.check_eq(t.slot("me", 0).await?.1, 1, "one egg used up");
    t.laid_out(&["text:Pig Spawn Egg"]);
    t.op("mode creative").await?;
    t.secs(0.3).await?;
    t.press("core:use");
    t.secs(0.4).await?;
    t.check_eq(t.slot("me", 0).await?.1, 1, "none in creative");
    let near = t.op(&format!("near {} {} {} 8", f[0], f[1], f[2])).await?;
    t.check(near.matches("base:pig").count() == 2, format!("two pigs: {near}"));
    Ok(())
}

/// A game with no window, reached by address: nobody owns it by connecting first, `op` from its
/// console makes someone more, its whitelist keeps out who isn't on it, `stop` saves and leaves.
pub async fn dedicated_server(t: T) -> R {
    use std::io::Write;
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
        }
    }
    let port = 30000 + (std::process::id() % 20000) as u16;
    let dir = format!("{}/test-server", t.c().run_dir());
    let _ = std::fs::remove_dir_all(&dir);
    static SPARKS: Config<u32> = Config::named("testmod:wisp_sparks");
    let own_sparks = SPARKS.get();
    std::fs::create_dir_all(format!("{dir}/config")).map_err(|e| e.to_string())?;
    std::fs::write(format!("{dir}/config/testmod.txt"), format!("wisp_sparks = {}\n", own_sparks + 3)).map_err(|e| e.to_string())?;
    let start = || -> Result<Child, String> {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        std::process::Command::new(exe)
            .args(["--server", "--world", "w", "--seed", "1", "--view", "4", "--port", &port.to_string(), "--run-dir", &dir])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(Child)
            .map_err(|e| e.to_string())
    };
    let listening = |t: &T| {
        let _ = t;
        std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
    };
    let mut server = start()?;
    t.until("the server listening", Duration::from_secs(30), listening).await?;
    t.c().join(&format!("127.0.0.1:{port}"));
    t.until("joined by address", Duration::from_secs(30), |t| t.flag(client::client_flags::JOINED)).await?;
    t.loaded().await?;
    t.check_eq(SPARKS.get(), own_sparks + 3, "the server's setting counts while on it");
    let refused = t.cmd("/time query").await?;
    t.check(!refused.contains("time is"), format!("the first to connect owns nothing: {refused}"));
    static NAME: TextConfig = TextConfig::named("core:name");
    let mut console = server.0.stdin.take().ok_or("no console")?;
    writeln!(console, "op {}", NAME.get()).map_err(|e| e.to_string())?;
    let _ = console.flush();
    t.secs(0.5).await?;
    let allowed = t.cmd("/time query").await?;
    t.check(allowed.contains("time is"), format!("op from the console: {allowed}"));
    t.c().leave_world();
    t.screen("core:title").await?;
    t.check_eq(SPARKS.get(), own_sparks, "the player's own setting after leaving");
    writeln!(console, "stop").map_err(|e| e.to_string())?;
    let _ = console.flush();
    t.until("the server stopped", Duration::from_secs(20), |_| server.0.try_wait().ok().flatten().is_some()).await?;
    t.check(std::fs::read_dir(format!("{dir}/saves/w/players")).is_ok_and(|mut d| d.next().is_some()), "the player was saved");

    std::fs::create_dir_all(format!("{dir}/config")).map_err(|e| e.to_string())?;
    std::fs::write(format!("{dir}/config/whitelist.txt"), "somebody else\n").map_err(|e| e.to_string())?;
    std::fs::write(format!("{dir}/config/disabled_mods.txt"), "").map_err(|e| e.to_string())?;
    let _server = start()?;
    t.until("the server listening again", Duration::from_secs(30), listening).await?;
    t.c().join(&format!("127.0.0.1:{port}"));
    t.screen("core:message").await?;
    t.check(!t.flag(client::client_flags::IN_WORLD), "not on the whitelist, not in");
    t.c().close_screen();
    Ok(())
}
