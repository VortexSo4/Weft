//! Beds: two blocks long, the foot where it is placed and the head away from the player. Using
//! one sets the player's respawn point there (kept through death); at night the player lies
//! down, and once every player of the dimension is asleep the night is skipped to the morning.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Instant;

use weft_sdk::*;

pub use base_api::{BED_SPAWN, BedSpawn};

pub static BED: Block = Block::named("bed");
static BED_ITEM: Item = Item::named("bed");

value! {
    /// Lying in the bed at `bed` (its foot) since world time `since`, during night number `night`
    /// of its dimension's `Nights`; `asleep` 0 when not. Not saved: a player who leaves wakes up.
    struct Sleep { bed: [i32; 3], since: u64, night: u64, asleep: u32 }
}
static SLEEP: Component<Sleep> = Component::named("sleep");

value! {
    /// Sent to their bed after dying (`on_way` 1), from `from` in dimension `dim`: where a bed far
    /// away stands can only be seen once there, and without one they go back.
    struct Arriving { from: [f64; 3], dim: u32, on_way: u32 }
}
static ARRIVING: Component<Arriving> = Component::named("bed_arriving");

/// Below this daylight it is night enough to sleep (dusk to dawn).
const NIGHT: f32 = 0.2;
/// Ticks every player has to lie in bed before the night is skipped.
const FALL_ASLEEP: u64 = 100;
/// Monsters this close keep the player awake, across and up or down (vanilla's box).
const MONSTERS: [f64; 2] = [8.0, 5.0];
/// Everything that counts as a monster, a mod's too.
static HOSTILE: Tag = Tag::named("hostile");

/// What the regions of a dimension know together, by its ticks: they all run them, whatever each
/// one takes, and each sees only its own players. The night is skipped by whichever finds
/// everybody asleep.
#[derive(Default)]
struct Nights {
    /// Players some region saw lately: when, and whether they have been asleep long enough.
    players: HashMap<u64, (u64, bool)>,
    /// Nights skipped so far: sleepers of an earlier night wake up.
    skipped: u64,
    skipped_at: u64,
}
static NIGHTS: Mutex<BTreeMap<u32, Nights>> = Mutex::new(BTreeMap::new());

/// Server to client: a message (`MESSAGES`), and lying down (1) or getting up (0).
static MESSAGE: Channel<u32> = Channel::named("bed_message");
static LYING: Channel<u32> = Channel::named("bed_lying");
/// Client to server: the player got up (left the sleep screen).
static GET_UP: Channel<()> = Channel::named("bed_get_up");

const MESSAGES: [&str; 6] = ["bed.base.spawn_set", "bed.base.not_night", "bed.base.monsters", "bed.base.occupied", "bed.base.missing", "bed.base.obstructed"];
const SPAWN_SET: u32 = 0;
const NOT_NIGHT: u32 = 1;
const MONSTERS_NEAR: u32 = 2;
const OCCUPIED: u32 = 3;
const MISSING: u32 = 4;
const OBSTRUCTED: u32 = 5;

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

/// Where a player stands on the bed with its foot at `foot`.
fn on_bed(foot: [i32; 3]) -> [f64; 3] {
    [foot[0] as f64 + 0.5, foot[1] as f64 + 0.5625, foot[2] as f64 + 0.5]
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
        // An explosion takes one half without breaking it: the other follows.
        .on_neighbor(|e| e.world().schedule(e.pos(), 1))
        .on_scheduled(|e| {
            let w = e.world();
            if BED.is(e.state()) && !BED.is(w.get(other_half(e.pos(), e.state()))) {
                w.set(e.pos(), 0);
            }
        })
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

    r.entity_component(&BED_SPAWN, BedSpawn { bed: [0; 3], dim: 0, set: 0 }, component_flags::SAVE | component_flags::KEEP | component_flags::SYNC_OWNER);
    r.attach(&PLAYER, &BED_SPAWN);
    r.entity_component(&SLEEP, Sleep { bed: [0; 3], since: 0, night: 0, asleep: 0 }, 0);
    r.attach(&PLAYER, &SLEEP);
    r.entity_component(&ARRIVING, Arriving { from: [0.0; 3], dim: 0, on_way: 0 }, 0);
    r.attach(&PLAYER, &ARRIVING);
    r.on(&PLAYER_RESPAWNED, |b| {
        let w = b.world();
        for e in b.iter() {
            let Some(spawn) = w.component_mut(e.player, &BED_SPAWN).filter(|s| s.set != 0) else { continue };
            let (Some(st), Some(arriving)) = (w.entity(e.player), w.component_mut(e.player, &ARRIVING)) else { continue };
            // Its mod may be gone by now.
            let dim = name_of(abi::entry_kind::DIMENSION, spawn.dim);
            if dim.is_empty() {
                spawn.set = 0;
                w.send(e.player, &MESSAGE, &MISSING);
                continue;
            }
            *arriving = Arriving { from: st.pos, dim: w.dimension(), on_way: 1 };
            w.teleport(e.player, Some(Dimension::generated(dim.to_string())), on_bed(spawn.bed));
        }
    });
    r.on(&ENTITY_HURT, |b| {
        let w = b.world();
        for e in b.iter() {
            get_up(&w, e.entity);
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
    // Vanilla's order: nothing is set on a bed that can't be used at all.
    if w.players().into_iter().any(|o| o != player && w.component(o, &SLEEP).is_some_and(|s| s.asleep != 0 && s.bed == foot)) {
        w.send(player, &MESSAGE, &OCCUPIED);
        return true;
    }
    // A player stays standing in bed, so as much room as that takes.
    if [1, 2].iter().any(|dy| w.flags([foot[0], foot[1] + dy, foot[2]]) & abi::block_flags::SOLID != 0) {
        w.send(player, &MESSAGE, &OBSTRUCTED);
        return true;
    }
    let dim = w.dimension();
    if let Some(spawn) = w.component_mut(player, &BED_SPAWN) {
        if spawn.set == 0 || spawn.bed != foot || spawn.dim != dim {
            *spawn = BedSpawn { bed: foot, dim, set: 1 };
            w.send(player, &MESSAGE, &SPAWN_SET);
        }
    }
    if w.daylight() >= NIGHT {
        w.send(player, &MESSAGE, &NOT_NIGHT);
    } else if monsters_near(&w, foot) {
        w.send(player, &MESSAGE, &MONSTERS_NEAR);
    } else if let Some(s) = w.component_mut(player, &SLEEP) {
        let night = NIGHTS.lock().unwrap().get(&dim).map_or(0, |n| n.skipped);
        *s = Sleep { bed: foot, since: w.time(), night, asleep: 1 };
        w.teleport(player, None, on_bed(foot));
        w.send(player, &LYING, &1);
    }
    true
}

fn monsters_near(w: &World, foot: [i32; 3]) -> bool {
    let c = [foot[0] as f64 + 0.5, foot[1] as f64 + 0.5, foot[2] as f64 + 0.5];
    let reach = (2.0 * MONSTERS[0] * MONSTERS[0] + MONSTERS[1] * MONSTERS[1]).sqrt();
    w.near_bodies(c, reach).into_iter().filter_map(|id| w.entity(id)).any(|e| {
        (e.pos[0] - c[0]).abs() <= MONSTERS[0] && (e.pos[2] - c[2]).abs() <= MONSTERS[0] && (e.pos[1] - c[1]).abs() <= MONSTERS[1] && HOSTILE.on_entity(e.kind)
    })
}

/// A player sent to their bed, once where it should be is loaded: without a bed, back they go.
fn arrive(w: &World, id: u64) {
    let (Some(a), Some(spawn)) = (w.component_mut(id, &ARRIVING).filter(|a| a.on_way != 0), w.component_mut(id, &BED_SPAWN)) else { return };
    if w.dimension() != spawn.dim {
        return;
    }
    let Some(state) = w.get_loaded(spawn.bed) else { return };
    a.on_way = 0;
    if !BED.is(state) {
        spawn.set = 0;
        w.send(id, &MESSAGE, &MISSING);
        w.teleport(id, Some(Dimension::generated(name_of(abi::entry_kind::DIMENSION, a.dim).to_string())), a.from);
    }
}

fn get_up(w: &World, player: u64) {
    if let Some(s) = w.component_mut(player, &SLEEP).filter(|s| s.asleep != 0) {
        s.asleep = 0;
        w.send(player, &LYING, &0);
    }
}

/// Wakes sleepers whose night was skipped or who left their bed, notes who is asleep, and skips
/// the night once everybody has been asleep a while. Spectators and the dead aren't asked.
fn watch_sleepers(w: &World) {
    let (now, dim) = (w.time(), w.dimension());
    let night = NIGHTS.lock().unwrap().get(&dim).map_or(0, |n| n.skipped);
    let mut here_asleep = false;
    let mut seen = Vec::new();
    for id in w.players() {
        arrive(w, id);
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
        if w.entity(id).is_some_and(|e| w.near_bodies(e.pos, 0.1).contains(&id)) {
            seen.push((id, ready));
        }
    }
    let mut all = NIGHTS.lock().unwrap();
    let nights = all.entry(dim).or_default();
    for (id, ready) in seen {
        nights.players.insert(id, (now, ready));
    }
    // Not seen by any region for a couple of ticks (either way: another world's ticks): not here.
    nights.players.retain(|_, (at, _)| at.abs_diff(now) <= 2);
    if !here_asleep || !nights.players.values().all(|&(_, ready)| ready) || nights.skipped_at.abs_diff(now) < FALL_ASLEEP {
        return;
    }
    nights.skipped_at = now;
    nights.skipped += 1;
    // To the next morning; ticks run on as before.
    w.set_day_time((w.day_time() / 24000 + 1) * 24000);
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
