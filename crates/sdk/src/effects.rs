//! Particles, text conditions, screen effects: things that only change what players see.
//!
//! ```ignore
//! static SPARK: Particle = Particle::named("spark");
//! r.particle(&SPARK).textures(&["glint_0", "glint_1"]).life(0.3, 0.6).fullbright();
//! w.spawn_particles(&SPARK, pos).count(12).speed(2.0);          // server: everyone nearby sees them
//! r.text_condition(&CHARGED, |v| v.player_component(&CHARGE).is_some_and(|c| *c > 0));
//! c.post_effect("mymod:underwater", 1.0);                  // client: until turned off
//! ```
//! Shaders for text (`{fx:..}`), GUI panels (`shader` in a class's `.gui`), particles, entities and screen
//! effects are assets: `shaders/fx|gui|particle|entity|post/<name>.wgsl` in the mod's folder.

use std::marker::PhantomData;

use crate::abi::{self, Slice, Str, particle_flags as pf};
use crate::{Client, ClientView, Dimension, Music, Particle, Registry, SkyObject, SoundCategory, TextCondition, World, callback, host, qualify};

pub struct ParticleBuilder<'a> {
    raw: *mut abi::Registrar,
    name: String,
    textures: Vec<String>,
    shader: String,
    handle: Option<&'static Particle>,
    desc: abi::ParticleDesc,
    _p: PhantomData<&'a mut Registry>,
}

impl ParticleBuilder<'_> {
    /// Frames over its life; relative names are under `particle/`. None: block dust (pieces of
    /// the burst's block).
    pub fn textures(mut self, names: &[&str]) -> Self {
        self.textures = names.iter().map(|t| crate::asset("particle", t)).collect();
        self
    }
    /// Seconds, picked between the two.
    pub fn life(mut self, min: f32, max: f32) -> Self {
        self.desc.life = [min, max];
        self
    }
    /// Edge length in blocks, picked between the two.
    pub fn size(mut self, min: f32, max: f32) -> Self {
        self.desc.scale = [min, max];
        self
    }
    /// Blocks per second squared, pulling down (negative rises).
    pub fn gravity(mut self, g: f32) -> Self {
        self.desc.gravity = g;
        self
    }
    /// Fraction of speed kept per tick (vanilla 0.98).
    pub fn drag(mut self, d: f32) -> Self {
        self.desc.drag = d;
        self
    }
    /// 0xRRGGBBAA at birth and at the end of its life.
    pub fn color(mut self, start: u32, end: u32) -> Self {
        (self.desc.color, self.desc.color_end) = (start, end);
        self
    }
    pub fn colliding(mut self) -> Self {
        self.desc.flags |= pf::COLLIDE;
        self
    }
    pub fn fullbright(mut self) -> Self {
        self.desc.flags |= pf::FULLBRIGHT;
        self
    }
    pub fn translucent(mut self) -> Self {
        self.desc.flags |= pf::TRANSLUCENT;
        self
    }
    pub fn shrinking(mut self) -> Self {
        self.desc.flags |= pf::SHRINK;
        self
    }
    /// Colors it with `shaders/particle/<name>.wgsl` (`fn shade(p: ParticleFrag) -> vec4<f32>`).
    pub fn shader(mut self, name: &str) -> Self {
        self.shader = crate::asset("", name);
        self
    }
}

impl Drop for ParticleBuilder<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let textures: Vec<Str> = self.textures.iter().map(|t| Str::new(t)).collect();
        let desc = abi::ParticleDesc {
            name: Str::new(&self.name),
            cell: self.handle.map_or(std::ptr::null(), |h| h.cell()),
            textures: Slice::new(&textures),
            shader: Str::new(&self.shader),
            ..self.desc
        };
        unsafe { (host().reg_particle)(self.raw, &desc) }
    }
}

/// A thing in a dimension's sky, see `Registry::sky_body`, `sky_stars`, `sky_dome`.
pub struct SkyBuilder<'a> {
    raw: *mut abi::Registrar,
    name: String,
    dimension: String,
    texture: String,
    shader: String,
    desc: abi::SkyDesc,
    _p: PhantomData<&'a mut Registry>,
}

impl SkyBuilder<'_> {
    /// Picture, relative names are under `textures/sky/`. A dome tells its layout by its shape:
    /// twice as wide as high is a panorama, three by two is the six faces of a box.
    pub fn texture(mut self, name: &str) -> Self {
        self.texture = crate::asset("sky", name);
        self
    }
    /// Colors it with `shaders/sky/<name>.wgsl` instead of the plain picture.
    pub fn shader(mut self, name: &str) -> Self {
        self.shader = crate::asset("", name);
        self
    }
    /// Ticks per turn around the sky, negative the other way. Without it, it follows the
    /// dimension's day like the sun does.
    pub fn orbit(mut self, ticks: f32) -> Self {
        self.desc.orbit = ticks;
        self
    }
    /// Stays where its phase, tilt and heading put it.
    pub fn still(self) -> Self {
        self.orbit(0.0)
    }
    /// Back to following the dimension's day, for changing another mod's object.
    pub fn follow_day(self) -> Self {
        self.orbit(f32::INFINITY)
    }
    /// Where on its orbit it starts, in turns: 0 rising in the east, 0.25 overhead, 0.5 setting.
    /// A moon opposite the sun is 0.5.
    pub fn phase(mut self, turns: f32) -> Self {
        self.desc.phase = turns;
        self
    }
    /// Degrees its orbit leans away from overhead, to the south.
    pub fn tilt(mut self, degrees: f32) -> Self {
        self.desc.tilt = degrees;
        self
    }
    /// Degrees its orbit turns around the vertical: 90 rises in the north instead of the east.
    pub fn heading(mut self, degrees: f32) -> Self {
        self.desc.heading = degrees;
        self
    }
    /// Degrees across the sky (vanilla's sun 33, moon 23); of each star, for stars.
    pub fn size(mut self, degrees: f32) -> Self {
        self.desc.size = degrees;
        self
    }
    /// Ticks per turn around its own middle.
    pub fn spin(mut self, ticks: f32) -> Self {
        self.desc.spin = ticks;
        self
    }
    /// How much of it shows by full day and by deep night, 0..1 (stars: 0 and 0.22).
    pub fn opacity(mut self, day: f32, night: f32) -> Self {
        self.desc.opacity = [day, night];
        self
    }
    /// 0xRRGGBB multiplied in.
    pub fn color(mut self, rgb: u32) -> Self {
        self.desc.color = rgb;
        self
    }
    /// The picture is a grid of frames, shown one after another: a moon's phases.
    pub fn frames(mut self, columns: u32, rows: u32) -> Self {
        self.desc.frames = [columns, rows];
        self
    }
    /// Ticks each frame lasts; by default a day of the dimension.
    pub fn frame_ticks(mut self, ticks: u32) -> Self {
        self.desc.frame_ticks = ticks;
        self
    }
    /// Lights the horizon while rising and setting, a sunset in this color (0xRRGGBB).
    pub fn glow(mut self, rgb: u32) -> Self {
        self.desc.glow = rgb;
        self
    }
    /// Adds its light to what is behind it. Without it, it covers what is behind it, so a moon
    /// with an opaque picture hides any other that passes behind it.
    pub fn additive(mut self) -> Self {
        self.desc.additive = 1;
        self
    }
    /// Covers what is behind it, for an object someone made `additive`.
    pub fn covers(mut self) -> Self {
        self.desc.additive = 0;
        self
    }
    /// Drawn low to high, one over the other; registration order among equals. Bodies start at
    /// 0, stars at -100, domes at -200.
    pub fn order(mut self, order: i32) -> Self {
        self.desc.order = order;
        self
    }
    /// The height of clouds' underside (only clouds use it, like `drift`).
    pub fn height(mut self, y: f32) -> Self {
        self.desc.height = y;
        self
    }
    /// Blocks per tick clouds drift along x, negative the other way.
    pub fn drift(mut self, blocks_per_tick: f32) -> Self {
        self.desc.drift = blocks_per_tick;
        self
    }
    /// How many stars (only stars use it, like `seed`).
    pub fn count(mut self, n: u32) -> Self {
        self.desc.count = n;
        self
    }
    /// What scatters the stars: another seed, another sky.
    pub fn seed(mut self, seed: u32) -> Self {
        self.desc.seed = seed;
        self
    }
}

impl Drop for SkyBuilder<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let desc = abi::SkyDesc { dimension: Str::new(&self.dimension), name: Str::new(&self.name), texture: Str::new(&self.texture), shader: Str::new(&self.shader), ..self.desc };
        unsafe { (host().reg_sky)(self.raw, &desc) }
    }
}

impl Registry {
    fn sky_object<'a>(&'a mut self, dimension: &Dimension, object: &SkyObject, kind: u32) -> SkyBuilder<'a> {
        SkyBuilder {
            raw: self.raw,
            name: object.name().to_string(),
            dimension: dimension.name().to_string(),
            texture: String::new(),
            shader: String::new(),
            desc: abi::SkyDesc { kind, ..abi::SkyDesc::DEFAULT },
            _p: PhantomData,
        }
    }

    /// A picture on an orbit, a sun or a moon, independent of everything else in the sky:
    ///
    /// ```ignore
    /// static PLANET: SkyObject = SkyObject::named("planet");
    /// r.sky_body(&OVERWORLD, &PLANET).texture("planet").size(6.0).orbit(96000.0).phase(0.1).tilt(20.0);
    /// ```
    ///
    /// Naming another mod's object changes only what you set: `r.sky_body(&OVERWORLD, &MOON).size(40.0)`.
    pub fn sky_body<'a>(&'a mut self, dimension: &Dimension, object: &SkyObject) -> SkyBuilder<'a> {
        self.sky_object(dimension, object, abi::sky_kind::BODY)
    }

    /// Points scattered over the sky and turning together, seen at night unless `opacity` says
    /// otherwise.
    pub fn sky_stars<'a>(&'a mut self, dimension: &Dimension, object: &SkyObject) -> SkyBuilder<'a> {
        self.sky_object(dimension, object, abi::sky_kind::STARS)
    }

    /// The whole sky at once, a skybox picture or a shader, standing still unless given an orbit.
    /// It starts out still, and over the day's colors; `opacity(0.0, 1.0)` keeps it to the night.
    pub fn sky_dome<'a>(&'a mut self, dimension: &Dimension, object: &SkyObject) -> SkyBuilder<'a> {
        self.sky_object(dimension, object, abi::sky_kind::DOME)
    }

    /// A layer of boxes in the air, 12 blocks each, wherever `texture`'s pixels are filled:
    /// `r.sky_clouds(&OVERWORLD, &CLOUDS).texture("clouds").height(300.0)`.
    pub fn sky_clouds<'a>(&'a mut self, dimension: &Dimension, object: &SkyObject) -> SkyBuilder<'a> {
        self.sky_object(dimension, object, abi::sky_kind::CLOUDS)
    }

    /// Takes something out of one dimension's sky, your own or another mod's (that mod has to load
    /// first: depend on it). The base game puts its sun, moon and stars in every dimension of its
    /// own, each to be taken out separately.
    pub fn remove_sky(&mut self, dimension: &Dimension, object: &SkyObject) {
        let mut b = self.sky_object(dimension, object, u32::MAX);
        b.desc.remove = 1;
    }

    /// A particle kind, see `ParticleBuilder`. Registering `core:block` or `core:poof` changes how
    /// the engine's own block dust and smoke look.
    pub fn particle<'a>(&'a mut self, particle: &'static Particle) -> ParticleBuilder<'a> {
        ParticleBuilder {
            raw: self.raw,

            name: particle.name().to_string(),
            textures: Vec::new(),
            shader: String::new(),
            handle: Some(particle),
            desc: abi::ParticleDesc::DEFAULT,
            _p: PhantomData,
        }
    }

    /// `{if:<ns>:<name>}...{/if}` in any text shows its part while `f` says so; asked on the
    /// client every frame such a text is on screen.
    pub fn text_condition(&mut self, condition: &TextCondition, f: impl Fn(&ClientView) -> bool + Send + Sync + 'static) {
        let cb = callback(move |a: &mut abi::TextConditionArgs| a.result = f(&ClientView::from_raw(a.client)) as u32);
        unsafe { (host().reg_text_condition)(self.raw, Str::new(condition.name()), cb) }
    }
}

enum Target {
    World(World),
    Client(Client),
}

/// Particles sent when this is dropped: `w.spawn_particles(&SPARK, pos).count(10).speed(1.5)`.
pub struct Burst {
    target: Target,
    burst: abi::ParticleBurst,
}

impl Burst {
    pub fn count(mut self, n: u32) -> Self {
        self.burst.count = n;
        self
    }
    /// Each particle starts up to this far from the position on every axis.
    pub fn spread(mut self, s: f32) -> Self {
        self.burst.spread = [s; 3];
        self
    }
    pub fn spread_xyz(mut self, s: [f32; 3]) -> Self {
        self.burst.spread = s;
        self
    }
    /// Starting velocity, blocks per second.
    pub fn velocity(mut self, v: [f32; 3]) -> Self {
        self.burst.velocity = v;
        self
    }
    /// Random extra velocity up to this, in any direction.
    pub fn speed(mut self, s: f32) -> Self {
        self.burst.speed = s;
        self
    }
    /// Each flies away from the middle of the burst, more or less, like the pieces of a broken block.
    pub fn outward(mut self) -> Self {
        self.burst.outward = 1;
        self
    }
    /// Block state the textureless kinds (block dust) take their texture from.
    pub fn block(mut self, state: u32) -> Self {
        self.burst.block = state;
        self
    }
}

impl Drop for Burst {
    fn drop(&mut self) {
        if std::thread::panicking() || self.burst.kind == u32::MAX {
            return;
        }
        match &self.target {
            Target::World(w) => unsafe { (host().particles)(w.raw(), &self.burst) },
            Target::Client(c) => unsafe { (host().client_particles)(c.raw(), &self.burst) },
        }
    }
}

fn burst(target: Target, kind: &Particle, pos: [f64; 3]) -> Burst {
    Burst { target, burst: abi::ParticleBurst { kind: kind.id(), pos, ..abi::ParticleBurst::DEFAULT } }
}

impl World {
    /// Particles every player near `pos` sees (one by default).
    pub fn spawn_particles(&self, kind: &Particle, pos: [f64; 3]) -> Burst {
        burst(Target::World(*self), kind, pos)
    }
}

impl Client {
    /// Particles only this client shows.
    pub fn spawn_particles(&self, kind: &Particle, pos: [f64; 3]) -> Burst {
        burst(Target::Client(*self), kind, pos)
    }

    /// Turns screen effect `name` (`shaders/post/<name>.wgsl`) on at `strength` (0 turns it off).
    pub fn post_effect(&self, name: &str, strength: f32) -> bool {
        let name = qualify(crate::mod_id(), name);
        unsafe { (host().client_command)(self.raw(), abi::client_op::POST_EFFECT, Str::new(&name), strength as f64) != 0 }
    }
}

pub struct SoundBuilder<'a> {
    raw: *mut abi::Registrar,
    name: String,
    files: Vec<String>,
    handle: Option<&'static crate::Sound>,
    desc: abi::SoundDesc,
    category_name: String,
    _p: PhantomData<&'a mut Registry>,
}

impl SoundBuilder<'_> {
    /// Variants picked at random: `sounds/<name>.ogg` or `.wav`, relative to this mod.
    pub fn files(mut self, names: &[&str]) -> Self {
        self.files = names.iter().map(|n| crate::asset("", n)).collect();
        self
    }
    /// Category for its volume setting: the engine's (`core:creatures` ...) or one registered
    /// with `Registry::sound_category` (any mod's).
    pub fn category(mut self, category: &SoundCategory) -> Self {
        self.category_name = category.name().to_string();
        self
    }
    pub fn volume(mut self, v: f32) -> Self {
        self.desc.volume = v;
        self
    }
    /// Playback speed picked between the two.
    pub fn pitch(mut self, min: f32, max: f32) -> Self {
        self.desc.pitch = [min, max];
        self
    }
    /// Blocks away it is still heard.
    pub fn range(mut self, blocks: f32) -> Self {
        self.desc.range = blocks;
        self
    }
}

impl Drop for SoundBuilder<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let files: Vec<Str> = self.files.iter().map(|f| Str::new(f)).collect();
        let desc = abi::SoundDesc { name: Str::new(&self.name), cell: self.handle.map_or(std::ptr::null(), |h| h.cell()), files: Slice::new(&files), category_name: Str::new(&self.category_name), ..self.desc };
        unsafe { (host().reg_sound)(self.raw, &desc) }
    }
}

pub struct MusicBuilder<'a> {
    raw: *mut abi::Registrar,
    name: String,
    tracks: Vec<String>,
    dimension: String,
    when: u32,
    _p: PhantomData<&'a mut Registry>,
}

impl MusicBuilder<'_> {
    /// Files streamed (`sounds/<name>.ogg|.wav`), relative to this mod.
    pub fn tracks(mut self, names: &[&str]) -> Self {
        self.tracks = names.iter().map(|n| crate::asset("", n)).collect();
        self
    }
    pub fn dimension(mut self, dimension: &crate::Dimension) -> Self {
        self.dimension = dimension.name().to_string();
        self
    }
    /// `abi::music_when` flags.
    pub fn when(mut self, flags: u32) -> Self {
        self.when = flags;
        self
    }
}

impl Drop for MusicBuilder<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            return;
        }
        let tracks: Vec<Str> = self.tracks.iter().map(|f| Str::new(f)).collect();
        let desc = abi::MusicDesc { name: Str::new(&self.name), tracks: Slice::new(&tracks), dimension: Str::new(&self.dimension), when: self.when, ..abi::MusicDesc::DEFAULT };
        unsafe { (host().reg_music)(self.raw, &desc) }
    }
}

impl Registry {
    /// A sound event, see `SoundBuilder`; blocks and entities name groups of them with `.sound_group()`.
    pub fn sound<'a>(&'a mut self, sound: &'static crate::Sound) -> SoundBuilder<'a> {
        SoundBuilder { raw: self.raw, name: sound.name().to_string(), files: Vec::new(), handle: Some(sound), desc: abi::SoundDesc::DEFAULT, category_name: String::new(), _p: PhantomData }
    }

    /// A sound category with its own volume setting (`<ns>:volume_<name>`, in the mod's
    /// settings); `world`: heard through caves and water like block sounds.
    pub fn sound_category(&mut self, category: &SoundCategory, world: bool) {
        unsafe { (host().reg_sound_category)(self.raw, Str::new(category.name()), world as u32) }
    }

    /// Background music, see `MusicBuilder`.
    pub fn music<'a>(&'a mut self, music: &Music) -> MusicBuilder<'a> {
        MusicBuilder { raw: self.raw, name: music.name().to_string(), tracks: Vec::new(), dimension: String::new(), when: 0, _p: PhantomData }
    }
}

impl World {
    /// Sound event `sound` at `pos` for the players in its range.
    pub fn play_sound(&self, sound: &crate::Sound, pos: [f64; 3], volume: f32) {
        unsafe { (host().play_sound)(self.raw(), sound.id(), &pos, volume, 0.0) }
    }
}

impl Client {
    /// Only this client hears it; `None` = not positioned (UI sounds).
    pub fn play_sound(&self, sound: &crate::Sound, pos: Option<[f64; 3]>, volume: f32) {
        let p = pos.as_ref().map_or(std::ptr::null(), |p| p as *const [f64; 3]);
        unsafe { (host().client_play_sound)(self.raw(), sound.id(), p, volume, 0.0) }
    }
}

/// The engine's sound categories, each with its volume setting.
pub mod sound_categories {
    use crate::SoundCategory;
    pub static MUSIC: SoundCategory = SoundCategory::named("core:music");
    pub static BLOCKS: SoundCategory = SoundCategory::named("core:blocks");
    pub static CREATURES: SoundCategory = SoundCategory::named("core:creatures");
    pub static PLAYERS: SoundCategory = SoundCategory::named("core:players");
    pub static AMBIENT: SoundCategory = SoundCategory::named("core:ambient");
    pub static UI: SoundCategory = SoundCategory::named("core:ui");
}
