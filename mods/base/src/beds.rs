//! Beds: two blocks long, the foot where it is placed and the head away from the player. Using
//! one sets the player's respawn point there (kept through death); at night the player lies
//! down, and once every player is asleep the night is skipped to the morning.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use weft_sdk::*;

use crate::mobs::{SPIDER, ZOMBIE};

pub use base_api::{BED_SPAWN, BedSpawn};

pub static BED: Block = Block::named("bed");
static BED_ITEM: Item = Item::named("bed");

value! {
    /// Lying in the bed at `bed` (its foot) since world time `since`, during night number `night`
    /// of `NIGHTS`; `asleep` 0 when not. Not saved: a player who leaves wakes up.
    struct Sleep { bed: [i32; 3], since: u64, night: u64, asleep: u32 }
}
static SLEEP: Component<Sleep> = Component::named("sleep");

/// Below this daylight it is night enough to sleep (dusk to dawn).
const NIGHT: f32 = 0.2;
/// Ticks every player has to lie in bed before the night is skipped.
const FALL_ASLEEP: u64 = 100;
/// Monsters this close keep the player awake.
const MONSTERS: f64 = 8.0;

/// Nights skipped so far: sleepers of an earlier night wake up.
static NIGHTS: AtomicU64 = AtomicU64::new(0);
/// Every player some region saw lately: when, and whether they have been asleep long enough.
/// Regions tick on their own, so the night is skipped by whichever finds everybody asleep.
static ROSTER: Mutex<Option<HashMap<u64, (Instant, bool)>>> = Mutex::new(None);
static LAST_SKIP: Mutex<Option<Instant>> = Mutex::new(None);

/// Server to client: a message (`MESSAGES`), and lying down (1) or getting up (0).
static MESSAGE: Channel<u32> = Channel::named("bed_message");
static LYING: Channel<u32> = Channel::named("bed_lying");
/// Client to server: the player got up (left the sleep screen).
static GET_UP: Channel<()> = Channel::named("bed_get_up");

const MESSAGES: [&str; 5] = ["bed.base.spawn_set", "bed.base.not_night", "bed.base.monsters", "bed.base.occupied", "bed.base.missing"];
const SPAWN_SET: u32 = 0;
const NOT_NIGHT: u32 = 1;
const MONSTERS_NEAR: u32 = 2;
const OCCUPIED: u32 = 3;
const MISSING: u32 = 4;

/// The other half of the bed half `state` at `pos`: the head lies away from where the bed faces
/// (the player who placed it), the foot toward it.
fn other_half(pos: [i32; 3], state: u32) -> [i32; 3] {
    let d = state_prop(state, "facing").and_then(Dir::from_name).unwrap_or(Dir::North).offset();
    let s = if state_prop(state, "part") == Some("head") { 1 } else { -1 };
    [pos[0] + d[0] * s, pos[1], pos[2] + d[2] * s]
}

fn foot_of(pos: [i32; 3], state: u32) -> [i32; 3] {
    if state_prop(state, "part") == Some("head") { other_half(pos, state) } else { pos }
}

pub fn register(r: &mut Registry) {
    r.block(&BED)
        .sound_group("wood")
        .facing(Facings::HORIZONTAL, Toward::Player)
        .prop("part", ["foot", "head"])
        .states(|p, s| {
            let head = p.is("part", "head");
            let mut f = ["bed_side"; 6];
            f[2] = "planks_oak";
            // The pillow at the head's far end; the texture shows it up at 0 degrees.
            f[3] = if head {
                match p.facing() {
                    Dir::South => "bed_top_head_0",
                    Dir::West => "bed_top_head_90",
                    Dir::North => "bed_top_head_180",
                    _ => "bed_top_head_270",
                }
            } else {
                "bed_top_foot"
            };
            s.faces(f).cutout().model_box([0.0, 0.0, 0.0], [16.0, 9.0, 16.0])
        })
        .hardness(0.2)
        .mineable("axe")
        .drops_nothing()
        .set(&crate::items::FUEL, 300)
        .on_use(use_bed)
        .on_placed(|w, c| {
            let head = other_half(c.pos, c.state);
            if let Some(s) = state_with(c.state, "part", "head") {
                w.set(head, s);
            }
        })
        .on_broken(|w, c| {
            let other = other_half(c.pos, c.state);
            if BED.is(w.get(other)) {
                w.set(other, 0);
            }
            if c.player != 0 && !w.creative(c.player) {
                let p = foot_of(c.pos, c.state);
                w.spawn_item([p[0] as f64 + 0.5, p[1] as f64 + 0.5, p[2] as f64 + 0.5], ItemStack::new(BED_ITEM.id(), 1));
            }
        })
        .tab(&crate::tabs::FUNCTIONAL);
    // Room for the head, or no bed.
    r.intercept(&question::PLACE_BLOCK, |q| {
        if !BED.is(q.state()) {
            return Verdict::Abstain;
        }
        let head = other_half(q.pos(), q.state());
        let there = q.world().get(head);
        if there == 0 || state_flags(there) & abi::block_flags::REPLACEABLE != 0 { Verdict::Abstain } else { Verdict::Deny }
    });

    r.entity_component(&BED_SPAWN, BedSpawn { bed: [0; 3], set: 0 }, component_flags::SAVE | component_flags::KEEP | component_flags::SYNC_OWNER);
    r.attach(&PLAYER, &BED_SPAWN);
    r.entity_component(&SLEEP, Sleep { bed: [0; 3], since: 0, night: 0, asleep: 0 }, 0);
    r.attach(&PLAYER, &SLEEP);
    r.on(&PLAYER_RESPAWNED, |b| {
        let w = b.world();
        for e in b.iter() {
            let Some(spawn) = w.component_mut(e.player, &BED_SPAWN) else { continue };
            if spawn.set == 0 {
                continue;
            }
            let p = spawn.bed;
            if BED.is(w.get(p)) {
                w.teleport(e.player, None, [p[0] as f64 + 0.5, p[1] as f64 + 0.5625, p[2] as f64 + 0.5]);
            } else {
                spawn.set = 0;
                w.send(e.player, &MESSAGE, &MISSING);
            }
        }
    });
    r.world_tick(watch_sleepers);
    r.packet(&GET_UP, |p, ()| get_up(&p.world(), p.player()));

    client(r);
}

fn use_bed(u: &mut Use) -> bool {
    let w = u.world();
    let player = u.player();
    let foot = foot_of(u.pos(), u.state());
    if let Some(spawn) = w.component_mut(player, &BED_SPAWN) {
        if spawn.set == 0 || spawn.bed != foot {
            *spawn = BedSpawn { bed: foot, set: 1 };
            w.send(player, &MESSAGE, &SPAWN_SET);
        }
    }
    let center = [foot[0] as f64 + 0.5, foot[1] as f64 + 0.5, foot[2] as f64 + 0.5];
    if w.daylight() >= NIGHT {
        w.send(player, &MESSAGE, &NOT_NIGHT);
    } else if [&ZOMBIE, &SPIDER].iter().any(|k| !w.near(center, MONSTERS, Some(k)).is_empty()) {
        w.send(player, &MESSAGE, &MONSTERS_NEAR);
    } else if w.players().into_iter().any(|o| o != player && w.component(o, &SLEEP).is_some_and(|s| s.asleep != 0 && s.bed == foot)) {
        w.send(player, &MESSAGE, &OCCUPIED);
    } else if let Some(s) = w.component_mut(player, &SLEEP) {
        *s = Sleep { bed: foot, since: w.time(), night: NIGHTS.load(Ordering::Relaxed), asleep: 1 };
        w.teleport(player, None, [center[0], foot[1] as f64 + 0.5625, center[2]]);
        w.send(player, &LYING, &1);
    }
    true
}

fn get_up(w: &World, player: u64) {
    if let Some(s) = w.component_mut(player, &SLEEP).filter(|s| s.asleep != 0) {
        s.asleep = 0;
        w.send(player, &LYING, &0);
    }
}

/// Wakes sleepers whose night was skipped or who left their bed, notes who is asleep, and skips
/// the night once everybody has been asleep a while.
fn watch_sleepers(w: &World) {
    let night = NIGHTS.load(Ordering::Relaxed);
    let now = Instant::now();
    let mut here_asleep = false;
    let mut seen = Vec::new();
    for id in w.players() {
        let Some(s) = w.component(id, &SLEEP).copied() else { continue };
        let mut ready = false;
        if s.asleep != 0 {
            let left = w.entity(id).is_none_or(|e| {
                let d = [e.pos[0] - (s.bed[0] as f64 + 0.5), e.pos[2] - (s.bed[2] as f64 + 0.5)];
                d[0] * d[0] + d[1] * d[1] > 2.5 * 2.5
            });
            if s.night < night || left || !BED.is(w.get(s.bed)) {
                get_up(w, id);
            } else {
                ready = w.time().saturating_sub(s.since) >= FALL_ASLEEP;
                here_asleep |= ready;
            }
        }
        seen.push((id, ready));
    }
    let mut roster = ROSTER.lock().unwrap();
    let roster = roster.get_or_insert_with(HashMap::new);
    for (id, ready) in seen {
        roster.insert(id, (now, ready));
    }
    // Players no region saw for two seconds left.
    roster.retain(|_, (at, _)| now.duration_since(*at).as_secs_f32() < 2.0);
    if !here_asleep || !roster.values().all(|&(_, ready)| ready) {
        return;
    }
    let mut last = LAST_SKIP.lock().unwrap();
    if last.is_some_and(|t| now.duration_since(t).as_secs_f32() < 5.0) {
        return;
    }
    *last = Some(now);
    w.set_time((w.time() / 24000 + 1) * 24000);
    NIGHTS.fetch_add(1, Ordering::Relaxed);
}

static SLEEP_SCREEN: Screen = Screen::named("sleeping");
/// Darkens the picture while lying in bed (`shaders/post/sleep.wgsl`).
const SLEEP_EFFECT: &str = "sleep";

/// The client's side: since when it lies in bed, and the last message with when it came.
static LYING_SINCE: Mutex<Option<Instant>> = Mutex::new(None);
static SHOWN: Mutex<Option<(u32, Instant)>> = Mutex::new(None);

fn client(r: &mut Registry) {
    r.client_packet(&MESSAGE, |_, m| *SHOWN.lock().unwrap() = Some((m, Instant::now())));
    r.client_packet(&LYING, |c, lying| {
        if lying != 0 {
            *LYING_SINCE.lock().unwrap() = Some(Instant::now());
            c.open_screen(&SLEEP_SCREEN);
        } else if LYING_SINCE.lock().unwrap().take().is_some() {
            c.post_effect(SLEEP_EFFECT, 0.0);
            if c.screen() == SLEEP_SCREEN.name() {
                c.close_screen();
            }
        }
    });
    // Leaving the screen (Escape, the button) gets the player up.
    r.client_screen(|c, screen| {
        if screen != SLEEP_SCREEN.name() && LYING_SINCE.lock().unwrap().take().is_some() {
            c.post_effect(SLEEP_EFFECT, 0.0);
            c.send(&GET_UP, &());
        }
    });
    r.client_frame(|c| {
        if let Some(since) = *LYING_SINCE.lock().unwrap() {
            c.post_effect(SLEEP_EFFECT, (since.elapsed().as_secs_f32() / 2.5).clamp(0.01, 1.0));
        }
        let mut shown = SHOWN.lock().unwrap();
        if let Some((m, at)) = *shown {
            let t = at.elapsed().as_secs_f32();
            if t > 3.0 {
                *shown = None;
            } else {
                let alpha = ((3.0 - t) / 0.5).clamp(0.0, 1.0);
                c.screen_text(0.5, 0.72, MESSAGES[m as usize % MESSAGES.len()], &[], 0xffffff00 | (alpha * 255.0) as u32, 1.0);
            }
        }
    });
    r.screen(&SLEEP_SCREEN, |ui| {
        ui.column(S::col().gap(12.0).center(), |ui| {
            ui.text_styled(S::col().class("title").scale(2.0).no_wrap().self_align(abi::ui_align::CENTER), "bed.base.sleeping", &[]);
            ui.button("bed.base.leave", S::col().min_w(200.0).self_align(abi::ui_align::CENTER), |c| {
                c.client().close_screen();
            });
        });
    })
    .closes_on_respawn();
}
