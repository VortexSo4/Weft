//! Weather in the overworld. Spells of rain come and go with its time, so they need no saving
//! and every player has the same; `/weather` overrides them for a while. What falls is up to the
//! biome: rain, snow where it is cold or high up, nothing over the desert. The server tells each
//! player what falls where they are (`base:weather`), and their client grays the sky and lets
//! rain or snow fall where it is open above.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use weft_sdk::*;

pub use base_api::{WEATHER, Weather, precipitation};

/// Ticks of one spell (ten minutes), and the share of spells that rain.
const SPELL: u64 = 12000;
const RAINY_PERCENT: u64 = 30;
/// Ticks it takes rain to set in or clear where a rainy spell meets a dry one.
const RAMP: f32 = 600.0;
/// From this height up it snows anywhere.
const SNOW_LINE: i32 = 150;

static COLD: Tag = Tag::named("cold");
static DRY: Tag = Tag::named("dry");
static RAIN: Particle = Particle::named("rain");
static SNOW: Particle = Particle::named("snow");

/// `/weather`'s override: until that time, this strength. Kept while the game runs.
static OVERRIDE: Mutex<Option<(u64, f32)>> = Mutex::new(None);

fn rainy(spell: u64) -> bool {
    // splitmix64: neighboring spells are unrelated.
    let mut z = spell.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    (z ^ (z >> 31)) % 100 < RAINY_PERCENT
}

/// The schedule's strength at `time`: 0 or 1, ramping where a rainy spell meets a dry one.
fn scheduled(time: u64) -> f32 {
    let spell = time / SPELL;
    if !rainy(spell) {
        return 0.0;
    }
    let into = (time % SPELL) as f32;
    let start = if spell > 0 && rainy(spell - 1) { 1.0 } else { into / RAMP };
    let end = if rainy(spell + 1) { 1.0 } else { (SPELL as f32 - into) / RAMP };
    start.min(end).clamp(0.0, 1.0)
}

fn strength(time: u64) -> f32 {
    let mut o = OVERRIDE.lock().unwrap();
    match *o {
        Some((until, s)) if time < until => s,
        Some(_) => {
            *o = None;
            scheduled(time)
        }
        None => scheduled(time),
    }
}

pub fn register(r: &mut Registry) {
    r.entity_component(&WEATHER, Weather { strength: 0.0, kind: precipitation::NONE }, component_flags::SYNC_OWNER);
    r.attach(&PLAYER, &WEATHER);
    r.world_tick(tell_players);
    r.command("weather", "[kind:word(clear,rain)] [minutes:int]", |cmd| {
        let w = cmd.world();
        let minutes = cmd.int("minutes").unwrap_or(10).clamp(1, 24 * 60);
        let strength = match cmd.text("kind") {
            Some("clear") => 0.0,
            Some("rain") => 1.0,
            _ => {
                let now = strength(w.time());
                cmd.reply("command.base.weather.query", &[Arg::Int((now * 100.0).round() as i64)]);
                return;
            }
        };
        *OVERRIDE.lock().unwrap() = Some((w.time() + minutes as u64 * 1200, strength));
        let key = if strength > 0.0 { "command.base.weather.rain" } else { "command.base.weather.clear" };
        cmd.reply(key, &[Arg::Int(minutes)]);
    });

    r.particle(&RAIN).textures(&["rain"]).life(0.55, 0.65).size(0.45, 0.6).gravity(0.0).drag(1.0).color(0xFFFFFFC0, 0xFFFFFFC0).colliding().translucent();
    r.particle(&SNOW).textures(&["snow"]).life(5.5, 6.5).size(0.1, 0.16).gravity(0.0).drag(1.0).colliding().translucent();
    r.client_frame(fall);
}

/// What falls where each player is, twice a second; written only when it changes, so it is
/// sent only then.
fn tell_players(w: &World) {
    for id in w.players() {
        let Some(st) = w.entity(id) else { continue };
        if st.age % 10 != 0 {
            continue;
        }
        let s = strength(w.time());
        let pos = [st.pos[0].floor() as i32, st.pos[1].floor() as i32, st.pos[2].floor() as i32];
        let biome = w.biome(pos);
        let kind = if s <= 0.0 || DRY.on_biome(biome) {
            precipitation::NONE
        } else if COLD.on_biome(biome) || pos[1] >= SNOW_LINE {
            precipitation::SNOW
        } else {
            precipitation::RAIN
        };
        let now = Weather { strength: s, kind };
        if let Some(c) = w.component_mut(id, &WEATHER).filter(|c| **c != now) {
            *c = now;
        }
    }
}

/// The client's weather as shown: it follows the server's over a few seconds.
static SHOWN: Mutex<f32> = Mutex::new(0.0);
/// Drops owed to the next frames, and the state of the random numbers placing them.
static OWED: Mutex<f32> = Mutex::new(0.0);
static SEED: AtomicU64 = AtomicU64::new(0x2545_F491_4F6C_DD1D);

fn random() -> f32 {
    let mut x = SEED.load(Ordering::Relaxed);
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    SEED.store(x, Ordering::Relaxed);
    (x >> 40) as f32 / (1u64 << 24) as f32
}

/// Blocks around the player drops fall in, and how far they fall before they land.
const RADIUS: f32 = 14.0;
const RAIN_FALL: f64 = 12.0;
const SNOW_FALL: f64 = 10.5;

fn fall(c: &Client) {
    let st = c.state();
    let here = c.view().component(&WEATHER).filter(|_| st.dim == crate::OVERWORLD.id()).unwrap_or(Weather { strength: 0.0, kind: precipitation::NONE });
    let shown = {
        let mut shown = SHOWN.lock().unwrap();
        *shown += (here.strength - *shown) * (1.0 - (-st.dt / 2.5).exp());
        *shown
    };
    c.overcast(shown * 0.9);
    if here.kind == precipitation::NONE || shown < 0.05 {
        return;
    }
    let (particle, per_second, height, velocity, drift) = if here.kind == precipitation::SNOW {
        (&SNOW, 350.0, SNOW_FALL, [0.0, -1.8, 0.0], 0.35)
    } else {
        (&RAIN, 900.0, RAIN_FALL, [0.0, -20.0, 0.0], 0.0)
    };
    let n = {
        let mut owed = OWED.lock().unwrap();
        *owed += per_second * shown * st.dt;
        let n = owed.floor().min(80.0);
        *owed -= owed.floor();
        n as u32
    };
    let [px, py, pz] = st.shown;
    for _ in 0..n {
        let x = (px + ((random() * 2.0 - 1.0) * RADIUS) as f64).floor() as i32;
        let z = (pz + ((random() * 2.0 - 1.0) * RADIUS) as f64).floor() as i32;
        // The highest block of the column near the player, landing only where the sky is open
        // above it; nothing in caves, under roofs or over unloaded ground.
        let Some(ground) = (py as i32 - 30..=py as i32 + 20).rev().find(|&y| c.block([x, y, z]).is_some_and(|b| b != 0)) else { continue };
        if c.light([x, ground + 1, z]).is_none_or(|l| l & 0xff < 15) {
            continue;
        }
        let pos = [x as f64 + random() as f64, ground as f64 + 1.0 + height, z as f64 + random() as f64];
        drop(c.spawn_particles(particle, pos).velocity(velocity).speed(drift));
    }
}
