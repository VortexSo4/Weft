//! What the audit of 2026-10-31 found, each as it was reproduced there.

use std::time::Duration;

use weft_sdk::*;

use super::add;
use crate::runner::{R, T};

async fn reopen(t: &T, seed: u64) -> R {
    let name = format!("test-{}", t.name());
    t.c().leave_world();
    t.screen("core:title").await?;
    t.c().open_world(&name, seed);
    t.until("joined again", Duration::from_secs(60), |t| t.flag(client::client_flags::JOINED)).await?;
    t.loaded().await?;
    Ok(())
}

fn at(p: [i32; 3]) -> String {
    format!("{} {} {}", p[0], p[1], p[2])
}

/// What left a cube doesn't come back from its copy on disk: neither a removed mob nor a taken item.
pub async fn saved_entities_stay_gone(t: T) -> R {
    t.flat().await?;
    let p = add(t.feet(), [3, 0, 3]);
    let pig = t.op(&format!("spawn base:pig {}", at(p))).await?;
    t.secs(0.2).await?;
    reopen(&t, 1).await?;
    t.check(t.op(&format!("near {} 16", at(p))).await?.contains("base:pig"), "the pig was saved");
    t.op(&format!("remove {}", pig.trim())).await?;
    t.secs(0.2).await?;
    reopen(&t, 1).await?;
    let near = t.op(&format!("near {} 16", at(p))).await?;
    t.check(!near.contains("base:pig"), format!("the removed pig came back: {near}"));
    Ok(())
}

/// Dead is dead: nothing heals it or lets it pick things up, and what was on the cursor falls too.
pub async fn death_is_a_state(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 diamond 5").await?;
    t.op("put me 1 gold_ingot 3").await?;
    t.press("core:inventory");
    t.screen("core:menu").await?;
    t.frames(3).await?;
    t.click_ui("slot:36", 0);
    t.until("on the cursor", Duration::from_secs(2), |t| t.c().inventory_slot(0).is_empty()).await?;
    t.cmd("/kill").await?;
    t.screen("core:death").await?;
    t.secs(5.0).await?;
    t.check_eq(t.op("health").await?, "0".to_string(), "still dead after a hunger pulse");
    t.check_eq(t.op("inv me").await?, String::new(), "picked up nothing, carried nothing");
    let f = t.feet();
    let near = t.op(&format!("near {} 8", at(f))).await?;
    t.check(near.matches("core:item").count() >= 2, format!("the cursor fell with the rest: {near}"));
    // And leaving doesn't undo it.
    reopen(&t, 1).await?;
    t.screen("core:death").await?;
    t.check_eq(t.op("health").await?, "0".to_string(), "dead after coming back");
    t.c().respawn();
    t.no_screen().await?;
    t.until("alive", Duration::from_secs(5), |t| t.state().health > 0.0).await
}

/// A respawn keeps what the game mode gives: creative players stay out of mobs' sight.
pub async fn respawn_keeps_mode(t: T) -> R {
    t.flat().await?;
    t.op("mode creative").await?;
    t.cmd("/kill").await?;
    t.screen("core:death").await?;
    t.c().respawn();
    t.no_screen().await?;
    t.secs(0.2).await?;
    let v = t.op("attr me core:visibility").await?;
    t.check(v.starts_with('0'), format!("visibility after respawning: {v}"));
    Ok(())
}

/// The void and `/kill` get through a hit's recovery, and nothing else is stopped by it.
pub async fn kill_and_void(t: T) -> R {
    t.flat().await?;
    t.cmd("/kill").await?;
    t.screen("core:death").await?;
    t.c().respawn();
    t.no_screen().await?;
    let s = t.state();
    t.c().teleport([s.pos[0], -300.0, s.pos[2]]);
    t.until("the void kills", Duration::from_secs(30), |t| t.state().health <= 0.0).await
}

/// Leaving with an open menu puts the cursor and the crafting grid back first.
pub async fn leaving_keeps_cursor(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 diamond 5").await?;
    t.press("core:inventory");
    t.screen("core:menu").await?;
    t.frames(3).await?;
    t.click_ui("slot:36", 0);
    t.until("on the cursor", Duration::from_secs(2), |t| t.c().inventory_slot(0).is_empty()).await?;
    reopen(&t, 1).await?;
    let inv = t.op("inv me").await?;
    t.check(inv.contains("base:diamond:5"), format!("the cursor came back: '{inv}'"));
    Ok(())
}

/// A chest gone closes its menu, so nothing goes into thin air, and `/kill @e` leaves it alone.
pub async fn containers(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let chest = add(f, [2, 0, 0]);
    t.set(chest, "chest").await?;
    t.op(&format!("insert {} diamond 5", at(chest))).await?;
    t.cmd("/kill @e[type=base:chest]").await?;
    t.secs(0.3).await?;
    t.op(&format!("menu base:chest {}", at(chest))).await?;
    t.screen("core:menu").await?;
    t.until("the diamonds survived /kill", Duration::from_secs(2), |t| !t.c().menu_slot(0).is_empty()).await?;
    t.set(chest, "air").await?;
    t.no_screen().await?;
    Ok(())
}

/// Liquids keep flowing after a reload, and when `/time` goes backwards.
pub async fn water_keeps_flowing(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let top = add(f, [4, 6, 4]);
    t.set(top, "water").await?;
    reopen(&t, 1).await?;
    t.secs(3.0).await?;
    t.check(t.block(add(top, [0, -1, 0])).await?.contains("water"), "water under the source after a reload");
    let other = add(f, [-4, 6, -4]);
    t.op("time 13000").await?;
    t.set(other, "water").await?;
    t.op("time 1000").await?;
    t.secs(3.0).await?;
    t.check(t.block(add(other, [0, -1, 0])).await?.contains("water"), "water under the source with the clock set back");
    Ok(())
}

/// Coming down a ladder doesn't hurt.
pub async fn ladder_no_fall(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let wall = add(f, [2, 0, 0]);
    t.fill(wall, add(wall, [0, 16, 0]), "stone").await?;
    t.fill(add(wall, [-1, 0, 0]), add(wall, [-1, 15, 0]), "ladder").await?;
    let s = t.state();
    t.c().teleport([f[0] as f64 + 1.5, f[1] as f64 + 15.0, s.pos[2]]);
    t.until("down", Duration::from_secs(15), |t| t.flag(client::client_flags::ON_GROUND)).await?;
    t.secs(0.5).await?;
    t.check(t.state().health >= 19.0, format!("health after a ladder: {}", t.state().health));
    Ok(())
}

/// Pouring a bucket into water of its own kind keeps the bucket; flowing water becomes a source.
pub async fn bucket_not_wasted(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let pool = add(f, [2, -1, 0]);
    t.set(pool, "water").await?;
    t.op("put me 0 water_bucket 1").await?;
    t.c().select(0);
    t.secs(0.2).await?;
    t.look_at(add(pool, [0, 0, 0]));
    t.frames(3).await?;
    t.press("core:use");
    t.secs(0.4).await?;
    t.check_eq(t.slot("me", 0).await?.0, "base:water_bucket".to_string(), "a source can't take more");
    Ok(())
}

/// Shift-clicking stays out of the crafting grid: from the hotbar it goes to the inventory.
pub async fn shift_click_skips_grid(t: T) -> R {
    t.flat().await?;
    t.op("put me 0 diamond 5").await?;
    t.press("core:inventory");
    t.screen("core:menu").await?;
    t.frames(3).await?;
    let Some([x, y, w, h]) = t.c().ui_rect("slot:36") else { return Err("no slot 36".into()) };
    t.c().cursor(x + w / 2.0, y + h / 2.0);
    t.frames(2).await?;
    t.hold("core:sprint", 0.3);
    t.frames(2).await?;
    t.c().click(0, 0.0);
    t.secs(0.3).await?;
    t.c().close_screen();
    t.no_screen().await?;
    let inv = t.op("inv me").await?;
    t.check(inv.contains("base:diamond:5"), format!("still in the inventory: '{inv}'"));
    Ok(())
}

/// A block can't be placed into a mob.
pub async fn no_block_into_mob(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let spot = add(f, [2, 0, 0]);
    t.fill(add(spot, [-1, 0, -1]), add(spot, [1, 2, -1]), "stone").await?;
    t.fill(add(spot, [-1, 0, 1]), add(spot, [1, 2, 1]), "stone").await?;
    t.fill(add(spot, [1, 0, 0]), add(spot, [1, 2, 0]), "stone").await?;
    t.op(&format!("spawn base:pig {}", at(spot))).await?;
    t.op("put me 0 stone 1").await?;
    t.c().select(0);
    t.secs(0.2).await?;
    t.look_at(add(spot, [1, 0, 0]));
    t.frames(3).await?;
    t.press("core:use");
    t.secs(0.4).await?;
    t.check_eq(t.block(spot).await?, "core:air".to_string(), "no stone in the pig");
    Ok(())
}

/// `/tp nan` is refused, and the player stays where it is.
pub async fn tp_nan(t: T) -> R {
    t.flat().await?;
    t.cmd("/tp nan nan nan").await?;
    t.secs(0.2).await?;
    let p = t.state().pos;
    t.check(p.iter().all(|v| v.is_finite()), format!("position after /tp nan: {p:?}"));
    Ok(())
}

/// Two changes to a cube in one tick, one of them a change of light, both reach the client.
pub async fn changes_in_one_tick(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let (torch, flower) = (add(f, [3, 0, 3]), add(f, [5, 0, 3]));
    t.c().chat(&format!("/tk set {} base:mossy_cobblestone", at(torch)));
    t.c().chat(&format!("/tk set {} base:dandelion", at(flower)));
    t.secs(1.0).await?;
    t.check_eq(t.block(flower).await?, "base:dandelion".to_string(), "the server has the flower");
    t.check(t.c().block(torch).is_some_and(|s| s != 0), format!("torch on the client: {:?}, on the server {}", t.c().block(torch), t.block(torch).await?));
    t.check(t.c().block(flower).is_some_and(|s| s != 0), format!("flower on the client: {:?}", t.c().block(flower)));
    Ok(())
}

/// A light taken away at a cube's edge is gone on the client as well.
pub async fn no_ghost_light(t: T) -> R {
    t.flat().await?;
    let f = t.feet();
    let cube_edge = [f[0].div_euclid(16) * 16 + 15, f[1] + 1, f[2]];
    t.set(cube_edge, "torch").await?;
    t.secs(0.5).await?;
    t.set(cube_edge, "air").await?;
    t.secs(1.0).await?;
    let probe = add(cube_edge, [1, 0, 0]);
    let server: u64 = t.op(&format!("light {}", at(probe))).await?.trim().parse().unwrap_or(0);
    let client = t.c().light(probe).unwrap_or(0) as u64;
    t.check_eq(client, server, "the client's light past the edge matches the server's");
    Ok(())
}

/// Down in the chat keeps what is being typed.
pub async fn chat_down_keeps_text(t: T) -> R {
    t.flat().await?;
    t.cmd("/time query").await?;
    t.press("core:chat");
    t.screen("core:chat").await?;
    t.frames(2).await?;
    t.c().type_text("hello");
    t.frames(2).await?;
    t.c().key("ArrowDown", 0.0);
    t.frames(3).await?;
    t.check_eq(t.c().input_text(), "hello".to_string(), "the text is still there");
    t.c().close_screen();
    t.no_screen().await
}

async fn stays_on_client(t: &T, b: [i32; 3], secs: f32) -> R {
    let start = std::time::Instant::now();
    while start.elapsed().as_secs_f32() < secs {
        t.frames(1).await?;
        t.check(t.c().block(b).is_some_and(|s| s != 0), "the client showed air for a moment");
    }
    Ok(())
}

/// A refused block doesn't flicker: said at the start of mining, the client never shows air.
pub async fn break_refused(t: T) -> R {
    t.flat().await?;
    let b = add(t.feet(), [2, 0, 0]);
    t.set(b, "dirt").await?;
    t.op(&format!("protect {}", at(b))).await?;
    t.until("the block on the client", Duration::from_secs(2), |t| t.c().block(b).is_some_and(|s| s != 0)).await?;
    t.look_at(b);
    t.frames(3).await?;
    t.hold("core:attack", 1.5);
    stays_on_client(&t, b, 1.7).await?;
    t.check_eq(t.block(b).await?, "base:dirt".to_string(), "still there on the server");
    t.op("protect off").await?;
    t.hold("core:attack", 1.5);
    t.until("broken once allowed", Duration::from_secs(3), |t| t.c().block(b).is_some_and(|s| s == 0)).await
}

/// A sword in creative is known on both sides: nothing flickers, whatever the button does.
pub async fn creative_sword_no_flicker(t: T) -> R {
    t.flat().await?;
    t.op("mode creative").await?;
    t.op("put me 0 iron_sword 1").await?;
    t.c().select(0);
    let b = add(t.feet(), [2, 0, 0]);
    t.set(b, "stone").await?;
    t.until("the block on the client", Duration::from_secs(2), |t| t.c().block(b).is_some_and(|s| s != 0)).await?;
    t.look_at(b);
    t.frames(3).await?;
    t.press("core:attack");
    stays_on_client(&t, b, 0.3).await?;
    t.hold("core:attack", 0.8);
    stays_on_client(&t, b, 1.0).await
}

/// Reach and mining speed are attributes, and client and server read the same ones.
pub async fn reach_and_mining_attributes(t: T) -> R {
    t.flat().await?;
    t.op("mode creative").await?;
    let far = add(t.feet(), [8, 0, 0]);
    t.set(far, "stone").await?;
    t.until("the block on the client", Duration::from_secs(2), |t| t.c().block(far).is_some_and(|s| s != 0)).await?;
    t.look_at(far);
    t.frames(3).await?;
    t.press("core:attack");
    t.secs(0.3).await?;
    t.check_eq(t.block(far).await?, "base:stone".to_string(), "out of reach at first");
    t.op("attr_mod me core:block_reach reach 0 10 0").await?;
    t.secs(0.2).await?;
    t.press("core:attack");
    t.until("reached", Duration::from_secs(2), |t| t.c().block(far).is_some_and(|s| s == 0)).await?;
    t.secs(0.2).await?;
    t.check_eq(t.block(far).await?, "core:air".to_string(), "and the server agrees");
    t.op("attr_rm me core:block_reach reach").await?;

    t.op("mode survival").await?;
    let near = add(t.feet(), [2, 0, 0]);
    t.set(near, "dirt").await?;
    t.until("the block on the client", Duration::from_secs(2), |t| t.c().block(near).is_some_and(|s| s != 0)).await?;
    t.look_at(near);
    t.frames(3).await?;
    t.op("attr_mod me core:mining_speed haste 0 49 0").await?;
    t.secs(0.2).await?;
    // Dirt takes 0.75 s by hand, and the server must not call this too soon.
    t.hold("core:attack", 0.3);
    t.until("mined at once", Duration::from_millis(280), |t| t.c().block(near).is_some_and(|s| s == 0)).await?;
    t.secs(0.2).await?;
    t.check_eq(t.block(near).await?, "core:air".to_string(), "not refused as too soon");
    Ok(())
}

/// The surface counts: above a source's top is not in it, and mods can ask.
pub async fn liquid_surface(t: T) -> R {
    t.flat().await?;
    let p = add(t.feet(), [3, 2, 0]);
    t.set(p, "water").await?;
    t.check_eq(t.op(&format!("in_liquid {} {} {}", p[0] as f64 + 0.5, p[1] as f64 + 0.5, p[2] as f64 + 0.5)).await?, "1".to_string(), "inside");
    t.check_eq(t.op(&format!("in_liquid {} {} {}", p[0] as f64 + 0.5, p[1] as f64 + 0.95, p[2] as f64 + 0.5)).await?, "0".to_string(), "above its surface, in its block");
    Ok(())
}

/// `near_bodies` leaves out what can't be hit.
pub async fn bodies_only(t: T) -> R {
    t.flat().await?;
    let p = add(t.feet(), [4, 0, 0]);
    t.op(&format!("spawn base:pig {}", at(p))).await?;
    t.op("drops 1").await?;
    t.secs(0.1).await?;
    let near = t.op(&format!("near_bodies {} 16", at(p))).await?;
    t.check(near.contains("base:pig"), format!("the pig counts: {near}"));
    t.check(!near.contains("core:item"), format!("a dropped item doesn't: {near}"));
    Ok(())
}
