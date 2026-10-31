mod controls;
mod receive;
pub mod ctm;
pub mod entity_render;
pub mod gizmo;
pub mod looks;
pub mod mesher;
pub mod models;
pub mod modui;
pub mod particles;
pub mod sky;
pub mod terrain;
pub mod terrain_gpu;
pub mod lod;
pub mod textures;
pub mod world;
pub mod world_render;

use std::time::Instant;

use glam::{DVec3, IVec3, Vec3};
use rustc_hash::{FxHashMap, FxHashSet};
use weft_abi::{ItemStack, block_flags as bf};
use weft_render::{Backend, BufferId, CommandList, FrameInfo};
use crate::gpu_layout::Scene;

use crate::content::content;
use crate::input::Input;
use crate::net::{ClientConn, ToClient, ToServer};
use crate::physics::{self, Hit};
use crate::world::{GameMode, Pose};
use crate::ui::text::{Arg, Msg};
use entity_render::EntityRenderer;
use textures::GameTextures;
use world::ClientWorld;
use world_render::{Camera, WorldRenderer};

pub struct LocalPlayer {
    pub pos: DVec3,
    pub vel: DVec3,
    /// Position at the previous physics tick.
    pub prev: DVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    /// Touched the ground at any physics step since the last move sent: a quick re-jump can
    /// leave the ground before the next send, and the server would add up the falls.
    pub landed: bool,
    pub flying: bool,
    /// Spectator: moves through blocks.
    pub noclip: bool,
    /// Flying speed multiplier, changed with the scroll wheel in spectator mode.
    pub fly_speed: f32,
    pub in_liquid: bool,
    /// Physics steps until the next jump while the key stays held (vanilla's 10 ticks), so a
    /// two-block gap cannot be used to bounce at full speed.
    pub jump_cooldown: u32,
    /// Smoothed eye height (lower while sneaking) and field of view multiplier (wider while
    /// sprinting or flying).
    pub eye: f64,
    pub fov_mul: f32,
    /// Lower box and eyes. Stays on under a low ceiling until there is room for more.
    pub pose: Pose,
    /// Actually sprinting, after the conditions: moving forward, not sneaking, not wading.
    pub sprinting: bool,
    /// Holding the use button on an item that takes time.
    pub using: bool,
    /// Can swim in liquids that are not swimmable (`core:swim_anywhere`).
    pub swim_anywhere: bool,
    pub can_sprint: bool,
    /// The solid body it rides.
    pub support: Option<u64>,
    /// Its box standing (width, height, eye): the player kind's, or what a mod gave it.
    pub size: [f64; 3],
    /// How far rides have carried it in all, now and at the previous tick: motion in the frame
    /// of what it stands on is the position less this (walking, bobbing).
    pub carried: DVec3,
    pub carried_prev: DVec3,
    /// Riding a body: its id and the position relative to it at the previous and the last
    /// step. Drawn on the body as shown, so a rider and its deck never drift apart.
    pub riding: Option<(u64, DVec3, DVec3)>,
    /// Walking speed and jump multipliers from the server (attributes).
    pub speed_mul: f64,
    pub jump_mul: f64,
    /// Sneaking now and wanting to sprint now.
    pub sneak: bool,
    pub sprint: bool,
}

pub struct ClientEntity {
    pub kind: u32,
    pub pos: DVec3,
    pub target: DVec3,
    pub yaw: f32,
    pub target_yaw: f32,
    pub pitch: f32,
    pub item: ItemStack,
    pub health: f32,
    pub hurt: f32,
    pub worn: Vec<u32>,
    /// Accumulated walk distance and current swing amplitude, for limb animation.
    pub walk: f32,
    pub swing: f32,
    pub age: f32,
    pub burning: bool,
    /// Seconds since it died: it falls over and turns red, then disappears in a puff.
    pub dying: Option<f32>,
    /// Walk distance at which its next step sounds.
    pub next_step: f32,
    /// Keyframe animation the server started: name, age it started at, looping.
    pub anim: Option<(String, f32, bool)>,
    /// How a player holds itself: its box and how the model stands.
    pub pose: Pose,
    /// Where the body faces: follows the head with a lag, and where it walks.
    pub body_yaw: f32,
    /// Arm swing progress 0..1 while swinging.
    pub arm_swing: Option<f32>,
    /// Seconds left of a white flash.
    pub flash: f32,
    /// Solid bodies: where the server had them at its last ticks (tick, position), oldest
    /// first; they are shown `INTERP_TICKS` behind the server (`pose`).
    pub track: std::collections::VecDeque<(f64, DVec3)>,
    /// A picked up item flying to who took it: where it started, the taker, seconds so far.
    pub pulled: Option<(DVec3, u64, f32)>,
    /// Its own box (width, height, eye) and the kind it looks like, when a mod set them.
    pub size: Option<[f32; 3]>,
    pub look: Option<u32>,
    /// The solid body it stands on (0: the world), and its position relative to that body:
    /// shown and as the server last had it. Riders are drawn where the body is drawn.
    pub frame: u64,
    pub rel: DVec3,
    pub rel_target: Option<DVec3>,
    /// A moving structure's blocks, in its own coordinates (meshed like the world's).
    pub structure: Option<Box<ClientWorld>>,
}

/// A picked up item reaches the taker's feet in three ticks (vanilla's pickup animation).
const PULL_SECS: f32 = 0.15;

impl LocalPlayer {
    /// Half width, height and eye height now (crouching lowers them).
    pub fn dims(&self) -> (f64, f64, f64) {
        let [w, h, eye] = self.size;
        let (h, eye) = physics::posed(self.pose, h, eye);
        (w / 2.0, h, eye)
    }
}

/// `u32` and `f32` values of an engine field component (`registry::HEALTH` ...).
fn words(bytes: &[u8]) -> impl Iterator<Item = [u8; 4]> + '_ {
    bytes.chunks_exact(4).map(|w| [w[0], w[1], w[2], w[3]])
}


impl ClientEntity {
    /// What its kind's client tick gets (`client` filled in by the caller).
    pub fn args(&self, id: u64, dt: f32) -> weft_abi::ClientEntityArgs {
        weft_abi::ClientEntityArgs { client: std::ptr::null_mut(), entity: id, kind: self.kind, pos: self.pos.to_array(), yaw: self.yaw, walked: self.walk, dying: self.dying.unwrap_or(-1.0), age: self.age, dt }
    }

    /// Half width, height and eye height of its box, like the server's `Entity::dims`.
    pub fn dims(&self) -> (f64, f64, f64) {
        let k = &content().game.entities[self.kind as usize];
        let [w, h, eye] = self.size.unwrap_or([k.width, k.height, k.eye]).map(|v| v as f64);
        let (h, eye) = physics::posed(self.pose, h, eye);
        (w / 2.0, h, eye)
    }

    /// The kind whose model, texture and animations draw it.
    pub fn look(&self) -> u32 {
        self.look.unwrap_or(self.kind)
    }

    /// Where a solid body was at server tick `tick` (fractional): between the two positions
    /// around it; past the newest one it goes on as it last moved for at most a tick (the
    /// server sends bodies every tick, so a stop shows as two equal positions, no overshoot).
    pub fn pose(&self, tick: f64) -> DVec3 {
        let n = self.track.len();
        let Some(&(last_t, last)) = self.track.back() else { return self.target };
        if tick >= last_t {
            return match n {
                1 => last,
                _ => {
                    let (pt, prev) = self.track[n - 2];
                    last + (last - prev) / (last_t - pt).max(1.0) * (tick - last_t).min(1.0)
                }
            };
        }
        let mut before = self.track[0];
        for &(t, p) in &self.track {
            if tick < t {
                return if t > before.0 { before.1.lerp(p, ((tick - before.0) / (t - before.0)).max(0.0)) } else { p };
            }
            before = (t, p);
        }
        last
    }
}


pub struct ClientMenu {
    pub kind: u32,
    pub window: u32,
    /// The entity it belongs to, 0 for none.
    pub owner: u64,
    /// The block state it was opened on (0: none) and the own name of what it belongs to.
    pub block: u32,
    pub name: String,
    pub slots: Vec<ItemStack>,
    pub cursor: ItemStack,
}

pub enum ClientEvent {
    OpenMenu,
    CloseMenu,
    Died,
}

/// Player physics runs at vanilla's tick rate; the camera is interpolated between ticks.
const STEP: f64 = crate::world::TICK;
/// How far behind the newest server tick bodies are shown (Sable, under Create: Aeronautics,
/// uses 1.5): late moves still find two positions to show between.
const INTERP_TICKS: f64 = 1.5;

/// While loading, a frame takes finished terrain and uploads meshes for what is left of this
/// after the rest of the last frame (half each), at least `MIN_BUDGET` each: loading goes as
/// fast as the frame allows, down to about 60 frames a second.
const FRAME_TARGET: f64 = 0.016;
const MIN_BUDGET: f64 = 0.002;
const MAX_BUDGET: f64 = 0.005;

pub struct Client {
    conn: ClientConn,
    pub world: ClientWorld,
    pub render: WorldRenderer,
    pub entity_render: EntityRenderer,
    pub player: LocalPlayer,
    pub joined: bool,
    pub player_id: u64,
    /// Keyframe animation the server started on us: (name, start, looping).
    self_anim: Option<(String, f32, bool)>,
    accum: f64,
    tick_accum: f64,
    /// Seconds of physics steps taken, the clock solid bodies are shown by.
    body_clock: f64,
    /// Client clock (`body_clock`) less server time (ticks x `STEP`): maps the client's clock
    /// onto the server's timeline bodies are shown on. It follows `offset_target` (arrival
    /// times, averaged) by stretching time at most 5%, so that timeline only ever goes on
    /// evenly: following each arrival's jitter would step bodies back and forth.
    tick_offset: Option<f64>,
    offset_target: f64,
    /// Structures whose cubes are uploaded for drawing.
    drawn_frames: FxHashSet<u64>,
    start: Instant,
    /// Ticks counted since the server last said the time (`Scene` ticks).
    since_time: u32,
    /// Particle kind per block state shown around the player (`ambient`), `u32::MAX` none.
    ambient_kinds: Option<Vec<Vec<u32>>>,
    /// The frame's `Scene`, one buffer per frame in flight.
    scene_bufs: Vec<BufferId>,
    pub target: Option<Hit>,
    pub target_entity: Option<u64>,
    pub selected: usize,
    pub disconnected: Option<String>,
    /// Seconds until holding a button repeats its action. A new press never waits.
    break_delay: f32,
    use_delay: f32,
    pub view_radius: i32,
    /// Makes the world in view from the seed (`terrain`); the server's cubes go through it.
    pub terrain: terrain::TerrainHandle,
    pub gpu_terrain: terrain_gpu::GpuTerrain,
    /// Distant terrain beyond the view.
    lod: lod::Lod,
    seed: u64,
    /// Resets sent to the terrain thread: batches of an older one are dropped.
    terrain_resets: u64,
    terrain_view: (IVec3, i32),
    /// The rest of a batch from the terrain thread cut at the frame's budget.
    terrain_left: Option<terrain::Batch>,
    /// Time each of taking terrain and uploading meshes may take this frame (`FRAME_TARGET`),
    /// and how long they took in the last one.
    budget: std::time::Duration,
    spent: f64,
    /// The rest of a frame (seconds), smoothed.
    rest: f64,
    /// Finished meshes not uploaded yet (the frame's budget ran out).
    meshes: std::collections::VecDeque<world::MeshResult>,
    pub fov: f32,
    /// Brightness setting, 0 moody .. 1 bright.
    pub brightness: f32,
    pub entities: FxHashMap<u64, ClientEntity>,
    pub inventory: Vec<ItemStack>,
    pub health: f32,
    pub max_health: f32,
    pub mode: GameMode,
    /// When jump was last pressed, for double-tap flight toggling.
    /// How sneak, sprint and flight are being asked for.
    controls: controls::Controls,
    /// 0 first person, 1 third person from behind, 2 from the front.
    pub view: u8,
    /// F1: no hand and no block outline.
    pub hide_gui: bool,
    /// Arm swing progress 0..1 while swinging.
    hand_swing: Option<f32>,
    /// The stack the hand shows: a new one only comes up after the old one went down.
    hand_stack: ItemStack,
    /// How far the hand is up, 0..1.
    hand_height: f32,
    /// Seconds since the last swing or change of items, and the attribute multiplying its speed.
    swing_since: f32,
    /// This player's attributes, by id, as the server says.
    pub(crate) attributes: Vec<f32>,
    /// What the server refused to have mined; the button let go or another block lifts it.
    refused_break: Option<(u64, IVec3)>,
    /// Where the hand's view is, easing after the camera: pitch, yaw.
    hand_view: (f32, f32),
    /// Where the own body faces in third person.
    self_body: f32,
    was_on_ground: bool,
    was_wet: bool,
    /// Blocks per tick it was falling at.
    was_falling: f32,
    /// Own walk distance and swing amplitude, for the third person model and view bobbing.
    self_walk: f32,
    self_swing: f32,
    /// Vanilla's bobbing state: distance walked (x0.6) and the smoothed amplitude.
    walk_dist: f32,
    bob: f32,
    last_smooth: DVec3,
    /// Held item and when it became held: its name shows above the hotbar for a moment.
    pub held_name: (ItemStack, Instant),
    /// Names of the players online, for completion.
    pub players: Vec<String>,
    /// Lines this player sent, oldest first (chat history).
    pub sent: Vec<String>,
    /// Mouse button and slots of a drag in progress in a menu.
    pub drag: Option<(u8, Vec<u32>)>,
    /// Slots shift-clicked by sweeping over them with shift and the left button held.
    pub sweep: Vec<u32>,
    /// The last slot pressed and when, for double clicks.
    pub last_click: Option<(u32, u8, Instant)>,
    /// Seconds left of the red screen flash after taking damage.
    pub hurt_flash: f32,
    pub particles: particles::Particles,
    pub sky: sky::Sky,
    /// Scratch: taken items and where they fly to this frame.
    pulls: Vec<(u64, DVec3)>,
    /// Scratch: solid bodies and where they are drawn this frame.
    frames: Vec<(u64, DVec3)>,
    /// Another kind the local player looks like (a mod's `set_look`).
    self_look: Option<u32>,
    /// Commands recorded early but drawn after the opaque ones: translucent blocks and blended
    /// particles, then cracks, gizmo fills and the hand.
    late: CommandList,
    hand_late: CommandList,
    /// Seconds until the next dust piece from the block being hit, and until its next knock.
    hit_dust: f32,
    hit_sound: f32,
    /// Sounds to play this frame (from the server and from here).
    pub sounds: Vec<crate::audio::Play>,
    /// Own walk distance at which the next step sounds.
    next_step: f32,
    rng: weft_noise::Rng,
    pub dead: bool,
    /// Raining, and thundering, as the server says.
    weather: (bool, bool),
    /// How much of it there is by now, 0..1 over about five seconds.
    rain: f32,
    /// Seconds left of a lightning's flash.
    lightning: f32,
    /// Holding the use button on an item that takes time: ticks left, ticks in all.
    using: Option<(f32, f32)>,
    /// On fire, as the server says.
    burning: bool,
    /// Who else is mining what, and how far along.
    others_mining: Vec<(u64, IVec3, f32)>,
    /// What the server was last told this player mines.
    told_mining: Option<(u64, IVec3)>,
    /// What the server says this player may do (`abi::permission`).
    pub permission: u32,
    /// Seconds since it died, while it is.
    dead_for: Option<f32>,
    pub components: FxHashMap<u32, Vec<u8>>,
    pub menu: Option<ClientMenu>,
    pub events: Vec<ClientEvent>,
    /// Mod channel messages waiting for their client handler.
    pub packets: Vec<(u32, Vec<u8>)>,
    /// Components of other entities the server shares.
    pub entity_components: FxHashMap<(u64, u32), Vec<u8>>,
    /// Formatted when drawn, in the language shown then.
    pub chat: Vec<(Msg, Instant)>,
    /// Answers to commands run from code, by their id, until read.
    pub command_replies: rustc_hash::FxHashMap<u32, Vec<Msg>>,
    pub next_command: u32,
    pub gizmos: gizmo::Gizmos,
    /// This frame's gizmo shapes, from `collect_gizmos`.
    pub gizmo_lines: Vec<gizmo::Line>,
    pub gizmo_fills: Vec<gizmo::Fill>,
    pub gizmo_labels: Vec<gizmo::Label>,
    /// The block being broken (frame, block as in `Hit`) and how far.
    pub breaking: Option<(u64, IVec3, f32)>,
    /// Hotbar contents last frame, so the HUD knows when to redraw.
    pub hotbar: Vec<u32>,
}

/// The player's position, sent at most once a frame ahead of an action: the server judges actions
/// by where the player looks, which it otherwise hears only 20 times a second.
struct LookOnce(Option<ToServer>);

impl LookOnce {
    fn send(&mut self, conn: &mut ClientConn) {
        if let Some(m) = self.0.take() {
            conn.send(m);
        }
    }
}

impl Client {
    pub fn toggle_gizmo(&mut self, group: &str) -> bool {
        let on = self.gizmos.toggle(group);
        self.send(ToServer::Gizmos(self.gizmos.enabled.clone()));
        on
    }

    /// `/gizmo [group]`: lists groups or switches one; mods' groups are `modid:name`.
    pub fn gizmo_command(&mut self, group: &str) -> Vec<Msg> {
        let text = |s: &str| Arg::Text(s.to_string());
        if group.is_empty() {
            let mut out = vec![Msg::new("command.gizmo.groups", Vec::new())];
            out.extend(gizmo::BUILTIN.iter().map(|g| Msg::new("command.gizmo.group", vec![text(g), Arg::Key(format!("gizmo.{g}"))])));
            out.push(match self.gizmos.enabled.is_empty() {
                true => Msg::new("command.gizmo.none_on", Vec::new()),
                false => Msg::new("command.gizmo.on_list", vec![text(&self.gizmos.enabled.join(", "))]),
            });
            return out;
        }
        let on = self.toggle_gizmo(group);
        vec![Msg::new(if on { "command.gizmo.on" } else { "command.gizmo.off" }, vec![text(group)])]
    }

    /// Builds the gizmo lines and labels for this frame; labels behind opaque blocks are left out.
    pub fn collect_gizmos(&mut self) {
        let mut g = std::mem::take(&mut self.gizmos);
        let (lines, fills, mut labels) = g.collect(self);
        self.gizmos = g;
        let c = content();
        let eye = self.render_camera().pos;
        let opaque = |p: IVec3| self.world.block_or_air(p).map(|s| if c.states[s as usize].flags & bf::OPAQUE != 0 { s } else { crate::content::AIR });
        labels.retain(|(pos, ..)| {
            let d = *pos - eye;
            let len = d.length();
            len < 24.0 && (len < 0.2 || physics::raycast(c, &opaque, eye, d / len, len - 0.1, false).is_none())
        });
        (self.gizmo_lines, self.gizmo_fills, self.gizmo_labels) = (lines, fills, labels);
    }

    /// Shows a line in the chat as if the server had sent it.
    pub fn chat_line(&mut self, line: Msg) {
        self.chat.push((line, Instant::now()));
        let extra = self.chat.len().saturating_sub(50);
        self.chat.drain(..extra);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new(gpu: &mut dyn Backend, assets: &crate::assets::Assets, textures: &GameTextures, mut conn: ClientConn, name: &str, key: &str, view_radius: i32, skin: Option<Vec<u8>>) -> Result<Client, String> {
        let c = content();
        let render = WorldRenderer::new(gpu, c, textures, assets)?;
        let entity_render = EntityRenderer::new(gpu, assets)?;
        let particles = particles::Particles::new(gpu, assets);
        let sky = sky::Sky::new(gpu, assets);
        conn.send(ToServer::Join { name: name.to_string(), key: key.to_string() });
        conn.send(ToServer::View(view_radius as u32));
        if let Some(png) = skin {
            conn.send(ToServer::Skin(png));
        }
        let (gpu_tx, gpu_rx) = crossbeam_channel::unbounded();
        let gpu_on = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let gpu_terrain = terrain_gpu::GpuTerrain::new(gpu, assets, gpu_rx, gpu_on.clone());
        Ok(Client {
            conn,
            world: ClientWorld::new(0),
            render,
            entity_render,
            player: LocalPlayer {
                pos: DVec3::new(0.0, 80.0, 0.0),
                vel: DVec3::ZERO,
                prev: DVec3::new(0.0, 80.0, 0.0),
                yaw: 0.0,
                pitch: 0.0,
                on_ground: false,
                landed: false,
                jump_cooldown: 0,
                eye: 1.62,
                fov_mul: 1.0,
                pose: Pose::Stand,
                sprinting: false,
                using: false,
                swim_anywhere: false,
                can_sprint: true,
                support: None,
                size: { let k = &c.game.entities[c.game.player_kind as usize]; [k.width, k.height, k.eye].map(|v| v as f64) },
                carried: DVec3::ZERO,
                carried_prev: DVec3::ZERO,
                riding: None,
                flying: false,
                noclip: false,
                fly_speed: 1.0,
                in_liquid: false,
                speed_mul: 1.0,
                jump_mul: 1.0,
                sneak: false,
                sprint: false,
            },
            joined: false,
            player_id: 0,
            self_anim: None,
            accum: 0.0,
            tick_accum: 0.0,
            body_clock: 0.0,
            tick_offset: None,
            offset_target: 0.0,
            drawn_frames: FxHashSet::default(),
            start: Instant::now(),
            since_time: 0,
            ambient_kinds: None,
            scene_bufs: Vec::new(),
            target: None,
            target_entity: None,
            selected: 0,
            disconnected: None,
            break_delay: 0.0,
            use_delay: 0.0,
            view_radius,
            terrain: terrain::spawn(gpu_tx, gpu_on.clone()),
            lod: lod::Lod::new(gpu_terrain.lod_tx.clone()),
            gpu_terrain,
            seed: 0,
            terrain_resets: 0,
            terrain_view: (IVec3::MAX, 0),
            meshes: std::collections::VecDeque::new(),
            terrain_left: None,
            budget: std::time::Duration::from_secs_f64(MIN_BUDGET),
            spent: 0.0,
            rest: 0.0,
            fov: 70.0,
            brightness: 0.5,
            entities: FxHashMap::default(),
            inventory: vec![ItemStack::EMPTY; content().game.player_layout.slots as usize],
            health: 20.0,
            max_health: 20.0,
            hurt_flash: 0.0,
            particles,
            sky,
            pulls: Vec::new(),
            frames: Vec::new(),
            self_look: None,
            late: CommandList::default(),
            hand_late: CommandList::default(),
            hit_dust: 0.0,
            hit_sound: 0.0,
            sounds: Vec::new(),
            next_step: 0.0,
            rng: weft_noise::Rng(0x9E37_79B9_7F4A_7C15),
            drag: None,
            sweep: Vec::new(),
            last_click: None,
            players: Vec::new(),
            sent: Vec::new(),
            held_name: (ItemStack::EMPTY, Instant::now()),
            mode: GameMode::Survival,
            controls: controls::Controls::default(),
            view: 0,
            hide_gui: false,
            hand_swing: None,
            hand_stack: ItemStack::default(),
            hand_height: 1.0,
            swing_since: 1000.0,
            attributes: content().game.attributes.iter().map(|a| a.default as f32).collect(),
            refused_break: None,
            hand_view: (0.0, 0.0),
            self_body: 0.0,
            was_on_ground: true,
            was_wet: false,
            was_falling: 0.0,
            self_walk: 0.0,
            self_swing: 0.0,
            walk_dist: 0.0,
            bob: 0.0,
            last_smooth: DVec3::ZERO,
            dead: false,
            permission: 0,
            weather: (false, false),
            rain: 0.0,
            lightning: 0.0,
            using: None,
            burning: false,
            others_mining: Vec::new(),
            told_mining: None,
            dead_for: None,
            components: FxHashMap::default(),
            menu: None,
            events: Vec::new(),
            packets: Vec::new(),
            entity_components: FxHashMap::default(),
            chat: Vec::new(),
            command_replies: Default::default(),
            next_command: 0,
            gizmos: Default::default(),
            gizmo_lines: Vec::new(),
            gizmo_fills: Vec::new(),
            gizmo_labels: Vec::new(),
            breaking: None,
            hotbar: vec![0; 9],
        })
    }

    pub fn send(&mut self, m: ToServer) {
        self.conn.send(m);
    }

    pub fn leave(&mut self) {
        self.conn.send(ToServer::Leave);
    }

    fn attribute(&self, id: u32) -> f32 {
        self.attributes.get(id as usize).copied().unwrap_or(1.0)
    }

    /// How charged the next swing is, 0..1, as the server will judge it.
    pub fn swing_strength(&self) -> f32 {
        crate::content::swing_charge(content(), self.hand_stack.item, self.swing_since * crate::world::TPS as f32, self.attribute(crate::server::extend::ATTACK_SPEED))
    }

    pub fn held(&self) -> ItemStack {
        self.inventory.get(self.selected).copied().unwrap_or_default()
    }

    /// Starts the terrain over for dimension `dim` (joined, or moved to another dimension).
    fn reset_terrain(&mut self, dim: u32, fwd: &mut Vec<terrain::In>) {
        self.world.clear();
        self.meshes.clear();
        self.world.dim = dim;
        self.terrain_resets += 1;
        self.terrain_view = (IVec3::MAX, 0);
        fwd.push(terrain::In::Reset { dim, seed: self.seed });
    }

    /// Distant terrain: tiles shown, made, their quads, asked for and not back.
    pub fn lod_stats(&self) -> (usize, usize, usize, usize) {
        let (made, quads) = self.render.lod_counts();
        (self.render.lod_draw.len(), made, quads, self.lod.pending())
    }

    /// Meshes still to come or to upload.
    pub fn pending_meshes(&self) -> usize {
        self.world.pending_meshes() + self.meshes.len()
    }

    pub fn close_menu(&mut self) {
        self.drag = None;
        if self.menu.take().is_some() {
            self.conn.send(ToServer::CloseMenu);
        }
    }

    pub fn update(&mut self, gpu: &mut dyn Backend, textures: &GameTextures, input: &Input, dt: f64) {
        self.take_messages(dt);
        self.read_look(input);
        self.read_controls(input);
        self.select_slot(input);
        self.ease_camera(input, dt);
        let moved = self.animate_player(input, dt);
        if input.pressed("core:drop") && self.mode != GameMode::Spectator {
            self.conn.send(ToServer::Drop { all: input.held("core:sprint") });
        }
        if input.pressed("core:inventory") && input.captured {
            self.conn.send(ToServer::OpenInventory);
        }
        self.step_physics(input, dt);
        let look = self.aim(input);
        self.act(input, dt, look);
        self.send_move(dt);
        self.animate_entities(dt);
        self.chip_block(dt);
        self.particles.update(dt as f32, &self.world);
        self.creature_sounds(dt as f32, moved);
        self.hurt_flash = (self.hurt_flash - dt as f32).max(0.0);
        self.dead_for = self.dead.then(|| self.dead_for.unwrap_or(0.0) + dt as f32);
        self.hotbar = self.inventory[..content().game.player_layout.hotbar as usize].iter().map(|s| if s.is_empty() { 0 } else { s.item | (s.count.min(255) << 24) }).collect();

        self.upload_meshes(gpu, textures);
    }

    fn take_messages(&mut self, dt: f64) {
        // Smoothed: from one frame alone, a quick frame gave the next a long budget, which made
        // it slow, and so on.
        self.rest += (dt - self.spent - self.rest) * 0.1;
        self.budget = std::time::Duration::from_secs_f64(((FRAME_TARGET - self.rest) / 2.0).clamp(MIN_BUDGET, MAX_BUDGET));
        let t = Instant::now();
        self.receive();
        self.spent = t.elapsed().as_secs_f64();
        if let Some(o) = &mut self.tick_offset {
            *o += (self.offset_target - *o).clamp(-0.05 * dt, 0.05 * dt);
        }
    }

    fn read_look(&mut self, input: &Input) {
        if input.captured {
            let sens = 0.0022 * input.sensitivity;
            self.player.yaw -= input.mouse_delta.x * sens;
            let look = input.mouse_delta.y * sens * if input.invert_y { -1.0 } else { 1.0 };
            self.player.pitch = (self.player.pitch - look).clamp(-1.55, 1.55);
        }
    }

    fn read_controls(&mut self, input: &Input) {
        let creative = self.mode == GameMode::Creative;
        let i = self.controls.read(&controls::Keys::of(input), self.player.flying, creative, Instant::now());
        (self.player.sneak, self.player.sprint, self.player.flying) = (i.sneak, i.sprint, i.flying);
    }

    fn select_slot(&mut self, input: &Input) {
        let old = self.selected;
        let spectator = self.mode == GameMode::Spectator;
        // In a screen the number keys belong to it (swapping slots with the hotbar), and with F3
        // down to the debug groups.
        if !spectator && input.captured && !input.held("core:debug") {
            for (i, action) in crate::input::HOTBAR.into_iter().enumerate().take(content().game.player_layout.hotbar as usize) {
                if input.pressed(action) {
                    self.selected = i;
                }
            }
        }
        if input.scroll != 0.0 {
            if spectator {
                self.player.fly_speed = (self.player.fly_speed * if input.scroll > 0.0 { 1.25 } else { 0.8 }).clamp(0.1, 10.0);
            } else {
                self.selected = (self.selected as i32 - input.scroll.signum() as i32).rem_euclid(content().game.player_layout.hotbar.max(1) as i32) as usize;
            }
        }
        if old != self.selected {
            self.conn.send(ToServer::Select(self.selected as u32));
        }
        let held = self.inventory.get(self.selected).copied().unwrap_or_default();
        if (held.item, held.data) != (self.held_name.0.item, self.held_name.0.data) || old != self.selected {
            self.held_name = (held, Instant::now());
        }
    }

    /// Eye and view widen or drop over a few frames like vanilla, not in one jump.
    fn ease_camera(&mut self, input: &Input, dt: f64) {
        let k = (dt * 12.0).min(1.0);
        let sneaking = self.player.sneak && !self.player.flying;
        let eye_target = self.player.dims().2;
        self.player.eye += (eye_target - self.player.eye) * k;
        let moving = input.held("core:forward");
        let fov_target = if self.player.flying { 1.1 } else { 1.0 } + if self.player.sprint && moving && !sneaking { 0.15 } else { 0.0 };
        self.player.fov_mul += (fov_target - self.player.fov_mul) * k as f32;
        if input.pressed("core:perspective") && input.captured {
            self.view = (self.view + 1) % 3;
        }
    }

    /// Walking (limbs, bobbing, steps) is motion in the frame of what the player stands on.
    /// Returns how far it walked.
    fn animate_player(&mut self, input: &Input, dt: f64) -> f32 {
        let spectator = self.mode == GameMode::Spectator;
        let t = (self.accum / STEP).clamp(0.0, 1.0);
        let now = self.smooth_pos() - self.player.carried_prev.lerp(self.player.carried, t);
        let moved = (now - self.last_smooth).with_y(0.0).length().min(2.0) as f32;
        let walked_by = now - self.last_smooth;
        self.last_smooth = now;
        self.self_body = turn_body(self.self_body, self.player.yaw, walked_by, dt as f32);
        let speed = moved / (dt as f32).max(1e-4);
        self.self_walk += moved;
        let k_swing = (dt * 10.0).min(1.0) as f32;
        // Limbs swing with horizontal speed in the air too, like vanilla.
        self.self_swing += ((speed / 5.0).min(1.0) - self.self_swing) * k_swing;
        let walking = self.player.on_ground && !self.player.flying;
        // Vanilla: amplitude follows the per-tick speed capped at 0.1, eased by 0.4 per tick.
        self.walk_dist += moved * 0.6;
        let target = if walking { (speed / 20.0).min(0.1) } else { 0.0 };
        self.bob += (target - self.bob) * (1.0 - 0.6f32.powf(dt as f32 * crate::world::TPS as f32));
        if let Some(p) = &mut self.hand_swing {
            *p += dt as f32 / 0.3;
            if *p >= 1.0 {
                self.hand_swing = None;
            }
        }
        // A use swings only when the server says it did something (`ToClient::Swing`).
        if input.captured && !spectator && (input.pressed("core:attack") || (input.held("core:attack") && self.breaking.is_some())) {
            self.swing_arm();
        }
        // Vanilla's 0.4 per tick: the old item goes down, the new one comes up. More of the same
        // item (a count or damage change) is not a change.
        let held = self.held();
        let same = held.is_empty() == self.hand_stack.is_empty() && (held.is_empty() || held.item == self.hand_stack.item);
        self.swing_since += dt as f32;
        self.player.using = self.using.is_some();
        self.body_particles(dt as f32);
        self.weather_effects(dt as f32);
        let step = dt as f32 * 0.4 * crate::world::TPS as f32;
        // The hand dips after a swing and comes back up as it charges (vanilla's strength cubed).
        let up = if same { self.swing_strength().powi(3) } else { 0.0 };
        self.hand_height += (up - self.hand_height).clamp(-step, step);
        if same || self.hand_height < 0.1 {
            if !same {
                self.swing_since = 0.0;
            }
            self.hand_stack = held;
        }
        let k_lag = 1.0 - 0.5f32.powf(dt as f32 * crate::world::TPS as f32);
        self.hand_view.0 += (self.player.pitch - self.hand_view.0) * k_lag;
        self.hand_view.1 += angle_diff(self.player.yaw, self.hand_view.1) * k_lag;
        // Both ease forever otherwise, and a still picture should be still.
        if (self.player.pitch - self.hand_view.0).abs() < 1e-5 {
            self.hand_view.0 = self.player.pitch;
        }
        if angle_diff(self.player.yaw, self.hand_view.1).abs() < 1e-5 {
            self.hand_view.1 = self.player.yaw;
        }
        if target == 0.0 && self.bob < 1e-5 {
            self.bob = 0.0;
        }
        moved
    }

    fn step_physics(&mut self, input: &Input, dt: f64) {
        let c = content();
        let world = &self.world;
        let block = |p: IVec3| world.block_or_air(p);
        // Walking waits for the ground under the player to arrive; flying goes on, taking cubes
        // that are not here yet for air (the next ones are usually on their way).
        let flying = self.player.flying || self.player.noclip;
        let ready = self.joined && !self.dead && (flying || block(self.player.pos.floor().as_ivec3()).is_some());
        // The clock bodies are predicted by runs on while the player waits, or they would stop.
        let mut ambient_steps = 0;
        if self.joined {
            let air_if_missing = |p: IVec3| block(p).or(Some(crate::content::AIR));
            self.accum += dt.min(0.25);
            while self.accum >= STEP {
                self.accum -= STEP;
                self.since_time = (self.since_time + 1).min(20);
                ambient_steps += 1;
                // A rider is where it is on its body: if moves arrived late and the body's past
                // changed, the rider moves with it, not over the deck.
                if let Some(e) = self.player.riding.and_then(|(b, _, now)| self.entities.get(&b).map(|e| e.pose(self.body_tick(self.body_clock)) + now)) {
                    self.player.pos = e;
                }
                self.player.prev = self.player.pos;
                self.player.carried_prev = self.player.carried;
                let flying = self.player.flying || self.player.noclip;
                let gravity = c.dims[world.dim as usize].gravity as f64;
                // Solid bodies at this step (predicted like the server moves them) and soft
                // ones nudging the player aside.
                self.body_clock += STEP;
                let at = self.body_tick(self.body_clock);
                let mut bodies = Vec::new();
                let mut push = DVec3::ZERO;
                for (&id, e) in &self.entities {
                    let k = &c.game.entities[e.kind as usize];
                    if k.is(weft_abi::entity_flags::SOLID) {
                        let (now, delta) = (e.pose(at), e.pose(at) - e.pose(at - 1.0));
                        let aabb = physics::Aabb::around(now, k.width as f64 / 2.0, k.height as f64);
                        let built = e.structure.as_deref();
                        let aabb = match built {
                            Some(s) => {
                                let Some((lo, hi)) = s.bounds else { continue };
                                let o = crate::server::structure_origin(now);
                                physics::Aabb { min: o + lo.as_dvec3(), max: o + (hi + IVec3::ONE).as_dvec3() }
                            }
                            None => aabb,
                        };
                        let blocks = built.map(|s| (s as &dyn physics::Blocks, crate::server::structure_origin(now)));
                        bodies.push(physics::Body { id, aabb, delta, carry: k.carry_height as f64, crushes: false, blocks });
                    } else if k.is(weft_abi::entity_flags::PUSHABLE) && e.item.is_empty() && e.dying.is_none() {
                        let d = self.player.pos - e.pos;
                        let ((ehw, eh, _), (hw, h, _)) = (e.dims(), self.player.dims());
                        let reach = ehw + hw;
                        if d.x.abs() < reach && d.z.abs() < reach && d.y < eh && -d.y < h {
                            push += physics::nudge(d);
                        }
                    }
                }
                if !ready {
                    continue;
                }
                if !flying {
                    self.player.vel += push;
                }
                if flying {
                    step_player(&mut self.player, input, &air_if_missing, gravity, &bodies);
                } else {
                    step_player(&mut self.player, input, &block, gravity, &bodies);
                }
                let entities = &self.entities;
                let p = &self.player;
                self.player.riding = p.support.and_then(|b| entities.get(&b).map(|e| (b, p.prev - e.pose(at - 1.0), p.pos - e.pose(at))));
                if crate::log::on(crate::log::Channel::Physics, log::Level::Trace) {
                    let p = &self.player;
                    let feet = p.pos.floor().as_ivec3() - glam::IVec3::Y;
                    log::trace!(
                        target: "physics",
                        "pos {:.4} {:.4} {:.4} vel {:.4} {:.4} {:.4} ground {} liquid {} crouch {} fly {} below {}",
                        p.pos.x, p.pos.y, p.pos.z, p.vel.x, p.vel.y, p.vel.z, p.on_ground, p.in_liquid, p.pose == Pose::Crouch, p.flying,
                        block(feet).map_or("unloaded".to_string(), |s| c.state_name(s))
                    );
                }
            }
        }
        for _ in 0..ambient_steps {
            self.ambient();
        }
    }

    /// What the crosshair is on. Returns where the eye looks.
    fn aim(&mut self, input: &Input) -> DVec3 {
        let c = content();
        let spectator = self.mode == GameMode::Spectator;
        // From the eye as drawn, the moment structures are drawn at too, so a block on a
        // structure the player rides stays the one aimed at.
        let eye = self.camera().pos;
        let look = self.camera().forward().as_dvec3();
        use crate::server::extend::{BLOCK_REACH, ENTITY_REACH};
        let reach = self.attribute(BLOCK_REACH) as f64;
        self.target = if spectator { None } else { self.raycast(eye, look, reach) };
        if input.captured && !spectator && input.pressed("core:pick") {
            if let Some(item) = self.target.and_then(|h| c.pick_item(h.state)) {
                let hotbar = &self.inventory[..content().game.player_layout.hotbar as usize];
                // Already in the hotbar: select it. Otherwise creative fills an empty slot first.
                let slot = hotbar.iter().position(|s| s.item == item && !s.is_empty()).or_else(|| {
                    (self.mode == GameMode::Creative && !hotbar[self.selected].is_empty()).then(|| hotbar.iter().position(|s| s.is_empty())).flatten()
                });
                if let Some(i) = slot.filter(|&i| i != self.selected) {
                    self.selected = i;
                    self.conn.send(ToServer::Select(i as u32));
                }
                if !hotbar.iter().any(|s| s.item == item && !s.is_empty()) {
                    self.conn.send(ToServer::Pick { item });
                }
            }
        }
        let block_dist = self.target.map_or(f64::INFINITY, |h| (h.block.as_dvec3() + DVec3::splat(0.5) - eye).length());
        self.target_entity = if spectator { None } else { self.pick_entity(eye, look, block_dist.min(self.attribute(ENTITY_REACH) as f64)) };
        look
    }

    fn move_message(&self, landed: bool) -> ToServer {
        ToServer::Move {
            frame: self.player.riding.map(|(b, _, now)| (b, now)),
            pos: self.player.pos,
            yaw: self.player.yaw,
            pitch: self.player.pitch,
            on_ground: landed || self.player.on_ground || self.player.flying,
            pose: self.player.pose,
            sprint: self.player.sprinting,
            flying: self.player.flying,
        }
    }

    fn act(&mut self, input: &Input, dt: f64, look: DVec3) {
        self.break_delay -= dt as f32;
        self.use_delay -= dt as f32;
        // The server judges actions by where the player looks, which it only hears 20 times a
        // second: a turn made just before a click must arrive first.
        let mut send_look = LookOnce(self.joined.then(|| self.move_message(self.player.landed)));
        if !input.captured || self.dead {
            self.breaking = None;
            return;
        }
        self.attack(input, dt, &mut send_look);
        self.use_held(input, dt, look, &mut send_look);
    }

    fn attack(&mut self, input: &Input, dt: f64, send_look: &mut LookOnce) {
        let c = content();
        let target = self.target;
        self.refused_break = self.refused_break.filter(|&r| input.held("core:attack") && target.is_some_and(|h| (h.frame, h.block) == r));
        let keeps = self.mode == GameMode::Creative && c.no_creative_break(self.held().item);
        let refused = keeps || self.refused_break.is_some();
        if input.held("core:attack") {
            if let Some(id) = self.target_entity {
                // A hit is a press, as often as you like; holding hits nothing. How much it
                // does depends on how charged the swing was.
                if input.pressed("core:attack") {
                    send_look.send(&mut self.conn);
                    self.conn.send(ToServer::Attack(id));
                    self.swing_since = 0.0;
                }
                self.breaking = None;
            } else if refused {
                self.breaking = None;
            } else if let Some(hit) = self.target {
                let held = self.held().item;
                let time = crate::server::mining_time(c, hit.state, held, self.eye_liquid().is_some(), self.player.on_ground, self.attribute(crate::server::extend::MINING_SPEED));
                let time = if self.mode == GameMode::Creative { 0.0 } else { time };
                let done = match self.breaking {
                    Some((f, p, v)) if (f, p) == (hit.frame, hit.block) => v,
                    _ => 0.0,
                };
                let progress = if time <= 0.0 { 1.0 } else { done + dt as f32 / time };
                if progress >= 1.0 && (input.pressed("core:attack") || self.break_delay <= 0.0) {
                    send_look.send(&mut self.conn);
                    self.conn.send(ToServer::Break { pos: hit.block, frame: hit.frame });
                    match hit.frame {
                        0 => self.world.set_block(hit.block, crate::content::AIR),
                        f => {
                            if let Some(s) = self.entities.get_mut(&f).and_then(|e| e.structure.as_mut()) {
                                s.set_block(hit.block, crate::content::AIR);
                            }
                        }
                    }
                    self.breaking = None;
                    self.break_delay = 0.25;
                } else if time > 0.0 {
                    self.breaking = Some((hit.frame, hit.block, progress.min(1.0)));
                } else {
                    // Instant breaks (creative, grass) show no cracks while the next waits.
                    self.breaking = None;
                }
            } else {
                self.breaking = None;
            }
        } else {
            self.breaking = None;
        }
        // Hitting nothing still spends the swing.
        if input.pressed("core:attack") && self.target_entity.is_none() && self.target.is_none() {
            self.swing_since = 0.0;
        }
        // The server times mining too, and shows the cracks to the others.
        let mining = self.breaking.map(|(f, p, _)| (f, p));
        if mining != self.told_mining {
            self.told_mining = mining;
            self.conn.send(match mining {
                Some((frame, pos)) => ToServer::StartBreak { pos, frame },
                None => ToServer::StopBreak,
            });
        }
    }

    fn use_held(&mut self, input: &Input, dt: f64, look: DVec3, send_look: &mut LookOnce) {
        if let Some((left, _)) = &mut self.using {
            *left = (*left - dt as f32 * crate::world::TPS as f32).max(0.0);
            if !input.held("core:use") {
                self.using = None;
                self.conn.send(ToServer::StopUsing);
            }
        } else if input.pressed("core:use") || (input.held("core:use") && self.use_delay <= 0.0) {
            send_look.send(&mut self.conn);
            if let Some(id) = self.target_entity {
                self.conn.send(ToServer::Interact(id));
            } else if let Some(hit) = self.target {
                let sneak = self.player.sneak;
                self.conn.send(ToServer::Use { pos: hit.block, face: hit.face, look: look.as_vec3().to_array(), sneak, frame: hit.frame });
            } else {
                self.conn.send(ToServer::UseItem { target: None });
            }
            self.use_delay = 0.2;
        }
    }

    fn send_move(&mut self, dt: f64) {
        self.tick_accum += dt;
        if self.tick_accum >= 0.05 {
            // Keeps the fraction, so moves go out 20 times a second at any frame rate.
            self.tick_accum = (self.tick_accum - 0.05).min(0.05);
            if self.joined && !self.dead {
                let landed = std::mem::replace(&mut self.player.landed, false);
                self.conn.send(self.move_message(landed));
            }
        }
    }

    fn animate_entities(&mut self, dt: f64) {
        let c = content();
        let k = (dt * 15.0).min(1.0);
        // Solid bodies show where the physics has them, one step behind like the player.
        let shown = self.body_clock - STEP + self.accum;
        let mut pulls = std::mem::take(&mut self.pulls);
        pulls.clear();
        pulls.extend(self.entities.iter().filter_map(|(&id, e)| {
            let (_, by, _) = e.pulled?;
            let to = if by == self.player_id { self.smooth_pos() } else { self.entities.get(&by).map_or(e.pos, |t| t.pos) };
            Some((id, to))
        }));
        for &(id, to) in &pulls {
            let Some(e) = self.entities.get_mut(&id) else { continue };
            let Some((from, _, t)) = &mut e.pulled else { continue };
            *t += dt as f32;
            e.pos = from.lerp(to, (*t / PULL_SECS).min(1.0) as f64);
            e.target = e.pos;
            if *t >= PULL_SECS {
                self.entities.remove(&id);
            }
        }
        self.pulls = pulls;
        // Bodies first: riders are drawn relative to where their body is drawn.
        let mut frames = std::mem::take(&mut self.frames);
        frames.clear();
        let at = self.body_tick(shown);
        frames.extend(self.entities.iter().filter(|(_, e)| c.game.entities[e.kind as usize].is(weft_abi::entity_flags::SOLID)).map(|(&id, e)| (id, e.pose(at))));
        for e in self.entities.values_mut().filter(|e| e.pulled.is_none()) {
            let before = e.pos;
            let body = e.rel_target.and_then(|t| frames.iter().find(|f| f.0 == e.frame).map(|f| (f.1, t)));
            let mut walked = None;
            e.pos = if c.game.entities[e.kind as usize].is(weft_abi::entity_flags::SOLID) {
                e.pose(at)
            } else if let Some((at, rel_target)) = body {
                let rel_before = e.rel;
                e.rel = e.rel.lerp(rel_target, k);
                walked = Some(e.rel - rel_before);
                at + e.rel
            } else {
                e.pos.lerp(e.target, k)
            };
            let mut dyaw = e.target_yaw - e.yaw;
            dyaw = (dyaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            e.yaw += dyaw * k as f32;
            e.body_yaw = turn_body(e.body_yaw, e.yaw, walked.unwrap_or(e.pos - before), dt as f32);
            e.arm_swing = e.arm_swing.map(|p| p + dt as f32 / 0.3).filter(|&p| p < 1.0);
            e.flash = (e.flash - dt as f32).max(0.0);
            let speed = (walked.unwrap_or(e.pos - before).with_y(0.0).length() / dt.max(1e-4)) as f32;
            e.walk += speed * dt as f32;
            e.swing += ((speed / 5.0).min(1.0) - e.swing) * (k as f32);
            e.hurt = (e.hurt - dt as f32).max(0.0);
            e.age += dt as f32;
            if let Some(d) = &mut e.dying {
                *d += dt as f32;
                e.hurt = 0.4;
                e.burning = false;
            }
        }
        self.frames = frames;
    }

    /// Hitting a block chips pieces off the face being hit, one per tick like vanilla.
    fn chip_block(&mut self, dt: f64) {
        let c = content();
        self.hit_dust -= dt as f32;
        if let (Some((frame, pos, _)), Some(hit)) = (self.breaking, self.target) {
            if self.hit_dust <= 0.0 && (hit.frame, hit.block) == (frame, pos) {
                self.hit_dust = 0.05;
                if let Some(&kind) = c.game.particle_names.get(weft_abi::particle::BLOCK) {
                    let normal = crate::world::DIRS[hit.face as usize].as_vec3();
                    let at = pos.as_dvec3() + self.frame_origin(frame) + DVec3::splat(0.5) + (normal * 0.55).as_dvec3();
                    let spread = (glam::Vec3::splat(0.45) * (glam::Vec3::ONE - normal.abs())).to_array();
                    let velocity = (normal * 0.6).to_array();
                    self.particles.spawn(&weft_abi::ParticleBurst { kind, pos: at.to_array(), spread, velocity, speed: 0.4, count: 1, block: hit.state, ..weft_abi::ParticleBurst::DEFAULT }, &self.world);
                }
            }
        }
    }

    fn upload_meshes(&mut self, gpu: &mut dyn Backend, textures: &GameTextures) {
        let center: IVec3 = (self.player.pos.floor().as_ivec3()) >> 4;
        // Wide enough for every column the server keeps sent (view radius + 2).
        let reach = self.view_radius + 3;
        if self.render.set_tint_tiles(gpu, (reach * 2 + 1) as u32) {
            self.world.retint_all();
        }
        for (col, tints) in self.world.take_tints(glam::IVec2::new(center.x, center.z), reach) {
            self.render.upload_tints(gpu, col, &tints);
        }
        if self.joined && self.terrain_view != (center, self.view_radius) {
            self.terrain_view = (center, self.view_radius);
            let _ = self.terrain.tx.send(terrain::In::View { center, radius: self.view_radius });
        }
        // Enough jobs that meshing keeps up at 50 frames per second too (results are taken once
        // a frame), not only at hundreds.
        self.world.schedule_meshing(center, crate::meshing().current_num_threads() * 512);
        self.render.update_solid(gpu, &self.world, center);
        self.world.set_eye(center.y);
        // Uploads too, oldest first; the rest wait for the next frame (dropped if their cube
        // changed or left meanwhile). Edits and what is around the player go right away.
        let (now, later) = self.world.take_meshes(center);
        for r in now {
            self.render.upload(gpu, textures, (0, r.pos), &r.mesh);
            self.world.mark_shown(r.pos);
        }
        self.meshes.extend(later);
        let start = Instant::now();
        while start.elapsed() < self.budget {
            let Some(r) = self.meshes.pop_front() else { break };
            if self.world.current(&r) {
                self.render.upload(gpu, textures, (0, r.pos), &r.mesh);
                self.world.mark_shown(r.pos);
            }
        }
        self.spent += start.elapsed().as_secs_f64();
        // Structures: meshed in their own coordinates, a changed cube at a time.
        for (&id, e) in self.entities.iter_mut() {
            let Some(s) = e.structure.as_mut() else { continue };
            s.schedule_meshing(IVec3::ZERO, 64);
            let (now, later) = s.take_meshes(IVec3::ZERO);
            for r in now.into_iter().chain(later) {
                self.render.upload(gpu, textures, (id, r.pos), &r.mesh);
            }
            for pos in std::mem::take(&mut s.removed) {
                self.render.remove((id, pos));
            }
            self.drawn_frames.insert(id);
        }
        let entities = &self.entities;
        let gone: Vec<u64> = self.drawn_frames.iter().copied().filter(|id| !entities.get(id).is_some_and(|e| e.structure.is_some())).collect();
        for id in gone {
            self.render.remove_frame(id);
            self.drawn_frames.remove(&id);
        }
        if self.joined {
            let cam = self.render_camera().pos;
            self.lod.update(gpu, &mut self.render, &mut self.world, textures, self.seed, cam, self.view_radius);
        }
        self.render.set_textures(gpu, textures);
    }

    fn chance(&mut self) -> f32 {
        self.rng.float()
    }

    /// What the player's own moving kicks up: a landing's dust, a sprint's, a splash going into
    /// water and bubbles while under it.
    fn body_particles(&mut self, dt: f32) {
        let c = content();
        let pos = self.player.pos;
        let name = |n: &str| c.game.particle_names.get(n).copied();
        let under = self.world.block_or_air((pos - DVec3::Y * 0.2).floor().as_ivec3()).unwrap_or(0);
        let (w, on_ground) = (self.player.size[0] as f32 / 2.0, self.player.on_ground);
        let fell = self.was_falling;
        if on_ground && !self.was_on_ground && fell > 0.3 && under != 0 {
            if let Some(kind) = name(weft_abi::particle::BLOCK) {
                let count = (fell * 25.0).clamp(4.0, 60.0) as u32;
                self.particles.spawn(&weft_abi::ParticleBurst { kind, block: under, pos: pos.to_array(), count, spread: [w, 0.1, w], speed: 1.5, ..weft_abi::ParticleBurst::DEFAULT }, &self.world);
            }
        }
        if on_ground && self.player.sprinting && under != 0 && self.chance() < dt * 20.0 {
            if let Some(kind) = name(weft_abi::particle::BLOCK) {
                self.particles.spawn(&weft_abi::ParticleBurst { kind, block: under, pos: pos.to_array(), count: 1, spread: [w, 0.0, w], speed: 1.0, ..weft_abi::ParticleBurst::DEFAULT }, &self.world);
            }
        }
        if self.player.in_liquid && !self.was_wet && fell > 0.1 {
            for (n, count) in [(weft_abi::particle::SPLASH, 24), (weft_abi::particle::BUBBLE, 12)] {
                if let Some(kind) = name(n) {
                    self.particles.spawn(&weft_abi::ParticleBurst { kind, pos: pos.to_array(), count, spread: [w * 2.0, 0.2, w * 2.0], speed: 3.0, velocity: [0.0, 3.0, 0.0], ..weft_abi::ParticleBurst::DEFAULT }, &self.world);
                }
            }
        }
        if self.eye_liquid().is_some() && self.chance() < dt * 1.5 {
            if let Some(kind) = name(weft_abi::particle::BUBBLE) {
                let at = pos + DVec3::Y * self.player.eye;
                self.particles.spawn(&weft_abi::ParticleBurst { kind, pos: at.to_array(), count: 2, spread: [0.2, 0.1, 0.2], speed: 0.5, ..weft_abi::ParticleBurst::DEFAULT }, &self.world);
            }
        }
        self.was_on_ground = on_ground;
        self.was_wet = self.player.in_liquid;
        self.was_falling = (-self.player.vel.y as f32).max(0.0);
    }

    /// What goes over the screen: the inside of a liquid, of a block, a fire.
    fn overlays(&self, cam: &Camera) -> Vec<sky::Overlay> {
        let c = content();
        let mut out = Vec::new();
        if self.view != 0 || self.hide_gui {
            return out;
        }
        let light = self.daylight().max(0.25);
        if let Some(l) = self.eye_liquid().map(|l| &c.liquids[l as usize]).filter(|l| l.overlay_opacity > 0.0) {
            let st = &c.states[l.first_state as usize];
            let own = c.textures.get(st.textures[0] as usize).map_or("", String::as_str);
            let tint = if st.tint != 0 { sky::rgb(crate::content::item_tint(st.tint)) } else { Vec3::ONE };
            out.push(sky::Overlay {
                picture: if l.overlay.is_empty() { own.to_string() } else { l.overlay.clone() },
                color: (tint * light).powf(2.2).extend(l.overlay_opacity).to_array(),
                tiles: 4.0,
                slide: [-cam.yaw.to_degrees() / 64.0, cam.pitch.to_degrees() / 64.0],
                lower: false,
                frame: 0,
            });
        }
        let head = (self.smooth_pos() + DVec3::Y * self.player.eye).floor().as_ivec3();
        if !self.player.noclip {
            if let Some(st) = self.world.block_or_air(head).map(|s| s as usize).filter(|&s| c.states[s].flags & weft_abi::block_flags::SOLID != 0 && c.tight[s].opacity >= 15) {
                if let Some(picture) = c.textures.get(c.states[st].textures[1] as usize) {
                    out.push(sky::Overlay { picture: picture.clone(), color: [0.01, 0.01, 0.01, 1.0], tiles: 1.0, slide: [0.0, 0.0], lower: false, frame: 0 });
                }
            }
        }
        if self.burning {
            if let Some(picture) = c.textures.get(c.fire_texture as usize) {
                let frame = (self.start.elapsed().as_secs_f32() * crate::world::TPS as f32) as u32;
                out.push(sky::Overlay { picture: picture.clone(), color: [1.0, 1.0, 1.0, 0.9], tiles: 2.0, slide: [0.0, 0.0], lower: true, frame });
            }
        }
        out
    }

    /// Like vanilla's block display ticks: random blocks around the player that name a particle
    /// (`abi::comp::AMBIENT_PARTICLE`) show one, each about every three seconds. The server has
    /// no part in it.
    fn ambient(&mut self) {
        const TRIES: usize = 667;
        let c = content();
        let kinds = self.ambient_kinds.get_or_insert_with(|| {
            let comp = c.components.id(weft_abi::comp::AMBIENT_PARTICLE);
            (0..c.states.len() as u32)
                .map(|s| comp.and_then(|id| c.components.text(crate::components::BLOCK, s, id)).map_or(Vec::new(), |names| names.split(',').filter_map(|n| c.game.particle_names.get(n).copied()).collect()))
                .collect()
        });
        let drip = c.game.particle_names.get(weft_abi::particle::DRIP).copied();
        let center = self.player.pos.floor().as_ivec3();
        let mut bursts: Vec<weft_abi::ParticleBurst> = Vec::new();
        for _ in 0..TRIES {
            let r = self.rng.next();
            let p = center + IVec3::new((r % 33) as i32 - 16, ((r >> 8) % 33) as i32 - 16, ((r >> 16) % 33) as i32 - 16);
            let state = self.world.block(p);
            for &kind in kinds.get(state as usize).map_or(&[][..], |k| &k[..]) {
                let pos = [p.x as f64 + 0.5, p.y as f64 + 0.9, p.z as f64 + 0.5];
                bursts.push(weft_abi::ParticleBurst { kind, pos, spread: [0.1, 0.1, 0.1], speed: 0.1, count: 1, ..weft_abi::ParticleBurst::DEFAULT });
            }
            // Water through a ceiling: a solid block under water, air under that.
            if let Some(kind) = drip.filter(|_| c.states.get(state as usize).is_some_and(|s| s.liquid.is_some() && (r >> 24) % 8 == 0)) {
                let (below, air) = (self.world.block(p - IVec3::Y), self.world.block(p - IVec3::Y * 2));
                if c.states[below as usize].flags & weft_abi::block_flags::SOLID != 0 && air == 0 {
                    let pos = [p.x as f64 + 0.5, p.y as f64 - 1.05, p.z as f64 + 0.5];
                    bursts.push(weft_abi::ParticleBurst { kind, pos, spread: [0.4, 0.0, 0.4], count: 1, ..weft_abi::ParticleBurst::DEFAULT });
                }
            }
        }
        for b in bursts {
            self.particles.spawn(&b, &self.world);
        }
    }

    /// The engine's cosmetics of one entity, its kind's client tick unless a mod gave it one:
    /// steps by distance walked, idle noises about every twelve seconds (vanilla's ambient
    /// timer), and a puff of smoke when its death animation ends (vanilla's 20 poof particles).
    pub fn entity_effects(&mut self, a: &weft_abi::ClientEntityArgs) {
        use crate::audio::Play;
        let c = content();
        let k = &c.game.entities[a.kind as usize];
        let pos = DVec3::from_array(a.pos);
        if a.dying >= 1.0 {
            if let Some(&kind) = c.game.particle_names.get(weft_abi::particle::POOF) {
                let (w, h) = (k.width * 0.5, k.height * 0.5);
                let at = (pos + DVec3::Y * h as f64).to_array();
                self.particles.spawn(&weft_abi::ParticleBurst { kind, pos: at, spread: [w, h, w], velocity: [0.0, 0.4, 0.0], speed: 0.8, count: 20, ..weft_abi::ParticleBurst::DEFAULT }, &self.world);
            }
            return;
        }
        if a.dying >= 0.0 {
            return;
        }
        let Some(e) = self.entities.get_mut(&a.entity) else { return };
        if a.walked >= e.next_step {
            e.next_step = a.walked + 1.6;
            let sound = if k.sounds[3] != u32::MAX { k.sounds[3] } else { self.world.block_or_air((pos - DVec3::Y * 0.2).floor().as_ivec3()).map_or(u32::MAX, |s| c.states[s as usize].sounds[2]) };
            if sound != u32::MAX {
                self.sounds.push(Play { sound, pos: Some(pos), volume: 0.2, pitch: 0.0 });
            }
        }
        if k.sounds[2] != u32::MAX && self.chance() < a.dt / 12.0 {
            self.sounds.push(Play { sound: k.sounds[2], pos: Some(pos + DVec3::Y * k.eye as f64), volume: 1.0, pitch: 0.0 });
        }
    }

    /// The player's own steps and the knock of the block being mined; decided here, nothing
    /// travels for them.
    fn creature_sounds(&mut self, dt: f32, moved: f32) {
        use crate::audio::Play;
        let c = content();
        let below = |w: &world::ClientWorld, p: DVec3| w.block_or_air((p - DVec3::Y * 0.2).floor().as_ivec3()).map_or(u32::MAX, |s| c.states[s as usize].sounds[2]);
        if self.player.on_ground && !self.player.flying && self.mode != GameMode::Spectator {
            self.next_step -= moved;
            if self.next_step <= 0.0 {
                // Vanilla: a step every 1/0.6 blocks walked.
                self.next_step = 1.6;
                let sound = below(&self.world, self.player.pos);
                self.sounds.push(Play { sound, pos: Some(self.player.pos), volume: 0.25, pitch: 0.0 });
            }
        }
        self.hit_sound -= dt;
        if let Some(hit) = self.target.filter(|h| self.breaking.is_some_and(|(f, p, _)| (f, p) == (h.frame, h.block))) {
            if self.hit_sound <= 0.0 {
                // Vanilla knocks every four ticks while mining.
                self.hit_sound = 0.2;
                let sound = c.states[hit.state as usize].sounds[3];
                self.sounds.push(Play { sound, pos: Some(hit.block.as_dvec3() + self.frame_origin(hit.frame) + DVec3::splat(0.5)), volume: 0.4, pitch: 0.0 });
            }
        }
        self.sounds.retain(|p| p.sound != u32::MAX);
    }

    /// What the ears need from the camera this frame.
    pub fn listener(&self) -> crate::audio::Listener {
        let c = content();
        let cam = self.camera();
        let eye = cam.pos.floor().as_ivec3();
        let liquid = self.world.block_or_air(eye).and_then(|s| c.states[s as usize].liquid).map(|l| c.liquids[l as usize].temperature);
        crate::audio::Listener {
            pos: cam.pos,
            yaw: cam.yaw,
            pitch: cam.pitch,
            underwater: liquid.is_some_and(|t| t < 1000),
            enclosed: self.world.light(eye) & 255 < 6 && c.dims[self.world.dim as usize].sky_light,
            dim: self.world.dim,
            day: self.daylight() > 0.5,
            creative: self.mode == GameMode::Creative,
        }
    }

    fn pick_entity(&self, eye: DVec3, dir: DVec3, max: f64) -> Option<u64> {
        let mut best: Option<(f64, u64)> = None;
        for (&id, e) in &self.entities {
            // Structures are aimed at by their blocks.
            if !e.item.is_empty() || id == self.player_id || e.structure.is_some() {
                continue;
            }
            let (hw, h, _) = e.dims();
            let b = physics::Aabb::around(e.pos, hw, h);
            if let Some(t) = physics::ray_aabb(eye, dir, &b, max) {
                if best.is_none_or(|(bt, _)| t < bt) {
                    best = Some((t, id));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Copies an engine field component into the entity (or the own player) that shows it.
    fn engine_field(&mut self, id: u64, comp: u32, bytes: &[u8]) {
        let c = content();
        let [health, burning, worn, shape] = c.game.field_comps;
        let f: Vec<f32> = words(bytes).map(f32::from_le_bytes).collect();
        let shaped = || {
            let size = (f.len() == 4 && f[0] > 0.0).then(|| [f[0], f[1], f[2]]);
            let look = words(bytes).nth(3).map(u32::from_le_bytes).filter(|&l| l != u32::MAX);
            (size, look)
        };
        if id == self.player_id {
            if comp == burning {
                self.burning = bytes.first().is_some_and(|&b| b != 0);
            }
            if comp == health && f.len() == 2 {
                // Lower health out of a lower maximum, or on joining, isn't a hit.
                if f[0] < self.health && f[1] >= self.max_health && self.health > 0.0 {
                    self.hurt_flash = 0.5;
                }
                (self.health, self.max_health) = (f[0], f[1]);
                if f[0] > 0.0 {
                    self.dead = false;
                }
            } else if comp == shape {
                let (size, look) = shaped();
                let k = &c.game.entities[c.game.player_kind as usize];
                self.player.size = size.unwrap_or([k.width, k.height, k.eye]).map(|v| v as f64);
                self.self_look = look;
            }
            return;
        }
        let Some(e) = self.entities.get_mut(&id) else { return };
        if comp == health && f.len() == 2 {
            if f[0] < e.health {
                e.hurt = 0.4;
            }
            e.health = f[0];
        } else if comp == burning {
            e.burning = bytes.first().is_some_and(|&b| b != 0);
        } else if comp == worn {
            e.worn = words(bytes).map(u32::from_le_bytes).collect();
        } else if comp == shape {
            (e.size, e.look) = shaped();
        }
    }

    /// Items the local player wears.
    pub fn worn(&self) -> Vec<u32> {
        let c = content();
        let w = &c.game.player_layout.worn;
        let slots = self.inventory.get(w.start as usize..w.end as usize).unwrap_or_default();
        slots.iter().filter(|s| !s.is_empty() && c.worn(s.item).is_some()).map(|s| s.item).collect()
    }

    /// Where the block (0, 0, 0) of `frame`'s space is drawn: a structure's pose, the world at 0.
    pub fn frame_origin(&self, frame: u64) -> DVec3 {
        match frame {
            0 => DVec3::ZERO,
            f => self.entities.get(&f).map_or(DVec3::ZERO, |e| crate::server::structure_origin(e.pos)),
        }
    }

    /// The block looked at: the world's or a structure's, whichever the ray enters first.
    fn raycast(&self, eye: DVec3, look: DVec3, reach: f64) -> Option<Hit> {
        let world = &self.world;
        let now = self.body_tick(self.body_clock - STEP + self.accum);
        let structures: Vec<(u64, DVec3, &dyn physics::Blocks)> = self
            .entities
            .iter()
            .filter_map(|(&id, e)| e.structure.as_deref().map(|s| (id, crate::server::structure_origin(e.pose(now)), s as &dyn physics::Blocks)))
            .collect();
        physics::pick(content(), &|p| world.block_or_air(p), &structures, eye, look, reach, false)
    }

    /// Server tick (fractional) bodies are shown at when the client clock reads `clock`.
    fn body_tick(&self, clock: f64) -> f64 {
        (clock - self.tick_offset.unwrap_or(clock)) / STEP - INTERP_TICKS
    }

    /// Position drawn this frame, between the last two physics ticks; on a body, on the body as
    /// it is shown now.
    pub fn smooth_pos(&self) -> DVec3 {
        let t = (self.accum / STEP).clamp(0.0, 1.0);
        if let Some((b, before, now)) = self.player.riding {
            if let Some(e) = self.entities.get(&b) {
                return e.pose(self.body_tick(self.body_clock - STEP + self.accum)) + before.lerp(now, t);
            }
        }
        self.player.prev.lerp(self.player.pos, t)
    }

    pub fn camera(&self) -> Camera {
        // From inside a liquid everything looks closer, vanilla's 60 degrees for 70.
        let inside = if self.eye_liquid().is_some() { 6.0 / 7.0 } else { 1.0 };
        Camera { pos: self.smooth_pos() + DVec3::Y * self.player.eye, yaw: self.player.yaw, pitch: self.player.pitch, fov: self.fov * self.player.fov_mul * inside, bob: (Vec3::ZERO, 0.0, 0.0) }
    }

    /// The liquid the eyes are in.
    fn eye_liquid(&self) -> Option<u16> {
        let world = &self.world;
        physics::liquid_at(content(), &|p| world.block_or_air(p), self.smooth_pos() + DVec3::Y * self.player.eye)
    }

    /// The camera with view bobbing, for drawing (aiming uses the steady one).
    fn bobbed(&self, mut cam: Camera) -> Camera {
        use std::f32::consts::PI;
        let (f, g) = (-self.walk_dist * PI, self.bob);
        // Vanilla's bobHurt: a jolt to the side when hit, and falling over when dead.
        let hit = (self.hurt_flash / 0.5).clamp(0.0, 1.0);
        let jolt = (hit.powi(4) * PI).sin() * 14.0;
        let fallen = self.dead_for.map_or(0.0, |t| 40.0 - 8000.0 / ((t * crate::world::TPS as f32).min(20.0) + 200.0));
        cam.bob = (
            Vec3::new(f.sin() * g * 0.5, -(f.cos() * g).abs(), 0.0),
            (f.sin() * g * 3.0 - jolt + fallen).to_radians(),
            ((f - 0.2).cos() * g).abs().to_radians() * 5.0,
        );
        cam
    }

    /// Vanilla's swing: starts over once past halfway, which is the quick arm of holding a block
    /// being mined.
    fn swing_arm(&mut self) {
        if self.hand_swing.is_none_or(|p| p >= 0.5) {
            self.hand_swing = Some(0.0);
        }
    }

    /// The camera drawn from: the eye, or up to 4 blocks behind or in front of it in third person,
    /// pulled in where a block is in the way.
    pub fn render_camera(&self) -> Camera {
        let eye = self.camera();
        if self.view == 0 {
            return self.bobbed(eye);
        }
        let front = self.view == 2;
        let mut cam = eye;
        if front {
            cam.yaw += std::f32::consts::PI;
            cam.pitch = -cam.pitch;
        }
        let back = -cam.forward().as_dvec3();
        let c = content();
        let world = &self.world;
        let block = |p: IVec3| world.block_or_air(p);
        let dist = physics::raycast(c, &block, eye.pos, back, 4.0, false).map_or(4.0, |h| ((h.block.as_dvec3() + DVec3::splat(0.5) - eye.pos).length() - 0.8).clamp(0.3, 4.0));
        cam.pos = eye.pos + back * dist;
        cam
    }

    /// The local player as an entity, for third person.
    fn self_entity(&self) -> ClientEntity {
        let w = &content().game.player_layout.worn;
        let worn: Vec<u32> = self.inventory.get(w.start as usize..w.end as usize).map_or(Vec::new(), |s| s.iter().filter(|s| !s.is_empty()).map(|s| s.item).collect());
        ClientEntity {
            kind: content().game.player_kind,
            pos: self.smooth_pos(),
            target: self.smooth_pos(),
            yaw: self.player.yaw,
            target_yaw: self.player.yaw,
            body_yaw: self.self_body,
            arm_swing: self.hand_swing,
            flash: 0.0,
            pitch: self.player.pitch,
            item: ItemStack::EMPTY,
            health: self.health,
            hurt: self.hurt_flash,
            worn,
            walk: self.self_walk,
            swing: self.self_swing,
            age: self.start.elapsed().as_secs_f32(),
            burning: self.burning,
            dying: None,
            next_step: 0.0,
            anim: self.self_anim.clone(),
            pose: self.player.pose,
            track: Default::default(),
            pulled: None,
            size: None,
            look: self.self_look,
            frame: 0,
            rel: DVec3::ZERO,
            rel_target: None,
            structure: None,
        }
    }

    pub fn set_view_distance(&mut self, r: i32) {
        self.view_radius = r;
        self.send(ToServer::View(r as u32));
    }

    pub fn destroy(&mut self, gpu: &mut dyn Backend) {
        for b in self.scene_bufs.drain(..) {
            gpu.destroy_buffer(b);
        }
        self.render.destroy(gpu);
        self.entity_render.destroy(gpu);
        self.particles.destroy(gpu);
        self.sky.destroy(gpu);
        self.gpu_terrain.destroy(gpu);
    }

    /// Fraction of full daylight for the current dimension and time.
    pub fn daylight(&self) -> f32 {
        // Rain takes away a third of the day, a lightning brings it back for a moment.
        let day = content().dims[self.world.dim as usize].daylight(self.world.time) * (1.0 - self.rain * 0.3125);
        if self.lightning > 0.0 { (day + 0.5).min(1.0) } else { day }
    }

    /// Rain or snow falling around the player, and now and then a lightning's flash.
    fn weather_effects(&mut self, dt: f32) {
        let target = if self.weather.0 { 1.0 } else { 0.0 };
        self.rain += (target - self.rain).clamp(-dt / 5.0, dt / 5.0);
        self.lightning = (self.lightning - dt).max(0.0);
        if self.weather.1 && self.chance() < dt / 12.0 {
            self.lightning = 0.15;
        }
        if self.rain <= 0.0 {
            return;
        }
        let c = content();
        let (rain, snow) = (c.game.particle_names.get(weft_abi::particle::RAIN).copied(), c.game.particle_names.get(weft_abi::particle::SNOW).copied());
        let snowy = c.ids.snowy;
        let center = self.player.pos;
        for _ in 0..(self.rain * dt * 500.0).ceil() as u32 {
            let r = self.rng.next();
            let (x, z) = (((r & 0xFFFF) as f64 / 65535.0 - 0.5) * 32.0, (((r >> 16) & 0xFFFF) as f64 / 65535.0 - 0.5) * 32.0);
            // Not right in front of the eyes, where a drop would be as wide as a block.
            if x * x + z * z < 9.0 {
                continue;
            }
            let at = center + DVec3::new(x, 12.0 + ((r >> 32) & 7) as f64, z);
            let cell = at.floor().as_ivec3();
            // Only under open sky: caves and houses stay dry.
            if crate::world::light::sky_of(self.world.light(cell)) < 15 {
                continue;
            }
            let biome = self.world.cube(cell >> 4i32).map_or(0, |cube| cube.biome((cell.x & 15) as usize, (cell.z & 15) as usize));
            let cold = snowy.is_some_and(|t| c.components.has(crate::components::BIOME, biome as u32, t));
            let (kind, fall) = if cold { (snow, 2.0) } else { (rain, 20.0) };
            if let Some(kind) = kind {
                self.particles.spawn(&weft_abi::ParticleBurst { kind, pos: at.to_array(), count: 1, velocity: [0.0, -fall, 0.0], ..weft_abi::ParticleBurst::DEFAULT }, &self.world);
            }
        }
    }

    /// Game ticks now, with the fraction of the current one.
    fn smooth_time(&self) -> f64 {
        let fraction = if self.since_time < 20 { (self.accum / STEP).clamp(0.0, 1.0) } else { 0.0 };
        self.world.time as f64 + self.since_time as f64 + fraction
    }

    /// Fog color (linear): the liquid's from inside one; else vanilla's dimming towards night,
    /// warmed while looking at a sunset.
    fn fog_linear(&self) -> Vec3 {
        if let Some(l) = self.eye_liquid() {
            return self.sky.liquid_color(l);
        }
        let d = &content().dims[self.world.dim as usize];
        let l = self.daylight();
        let c = sky::rained(sky::rgb(d.fog_color) * Vec3::new(l * 0.94 + 0.06, l * 0.94 + 0.06, l * 0.91 + 0.09), self.rain, 0.5);
        let forward = self.camera().forward();
        sky::warmed_by_glows(self.world.dim as usize, d, self.smooth_time(), c, forward).powf(2.2)
    }

    /// The frame as every world pass sees it (`gpu_layout::Scene`).
    fn scene(&self, cam: &Camera, frame: &FrameInfo) -> Scene {
        // Fog ends where the terrain does: the view, or the distant terrain beyond it.
        let view_distance = (self.view_radius * 16).max(lod::Lod::distance()) as f32;
        // Vanilla's water fog starts behind the eye, lava's right at it.
        let (fog_start, fog_end) = match self.eye_liquid().map(|l| content().liquids[l as usize].fog_distance) {
            Some(end) => (if end >= 16.0 { -8.0 } else { end * 0.25 }, end),
            None => (view_distance * 0.6, view_distance),
        };
        let aspect = frame.width as f32 / frame.height.max(1) as f32;
        let view = cam.view();
        let inv = view.inverse();
        // Animated textures follow the game's ticks: between two from the server the client
        // counts on for a second at most, so they stop with the game.
        let ticks = (self.smooth_time() % (1 << 20) as f64) as f32;
        Scene {
            view_proj: (cam.proj(aspect) * view).to_cols_array_2d(),
            right: inv.transform_vector3(Vec3::X).extend(0.0).to_array(),
            up: inv.transform_vector3(Vec3::Y).extend(0.0).to_array(),
            fog_color: self.fog_color(),
            params: [fog_start, fog_end, self.daylight(), crate::shaders::time()],
            light: [content().dims[self.world.dim as usize].ambient as f32 / 15.0, self.brightness, ticks, 0.0],
            sun: self.sun(),
        }
    }

    /// Toward the sun: it rises in +x, is highest at noon and leans a little south so that
    /// shadows are never straight down.
    fn sun(&self) -> [f32; 4] {
        let d = &content().dims[self.world.dim as usize];
        if !d.sky_light {
            return [0.0, 1.0, 0.0, 0.0];
        }
        let a = d.day_turn(self.world.time as f64) * std::f64::consts::TAU;
        let v = Vec3::new(a.cos() as f32, a.sin() as f32, 0.35).normalize();
        [v.x, v.y, v.z, v.y.max(0.0)]
    }

    pub fn render(&mut self, gpu: &mut dyn Backend, list: &mut CommandList, frame: &FrameInfo, textures: &GameTextures) {
        self.entity_render.local = self.player_id;
        for (entity, png) in std::mem::take(&mut self.entity_render.skin_pending) {
            self.entity_render.set_skin(gpu, entity, &png);
        }
        let cam = self.render_camera();
        let scene = self.scene(&cam, frame);
        while self.scene_bufs.len() < gpu.frames_in_flight() {
            self.scene_bufs.push(gpu.create_buffer(size_of::<Scene>() as u64, weft_render::usage::STORAGE | weft_render::usage::HOST, "scene"));
        }
        let scene_buf = self.scene_bufs[frame.slot];
        gpu.mapped(scene_buf)[..size_of::<Scene>()].copy_from_slice(bytemuck::bytes_of(&scene));
        // Opaque blocks, then entities and cutout particles, then what is seen through:
        // translucent blocks, cracks and gizmo fills, blended particles, and the hand.
        let frames: Vec<(u64, DVec3)> = self.entities.iter().filter(|(_, e)| e.structure.is_some()).map(|(&id, e)| (id, crate::server::structure_origin(e.pos))).collect();
        self.render.draw(gpu, list, frame, &cam, scene_buf, &frames);
        list.push(weft_render::Cmd::Mark);
        let mut late = std::mem::take(&mut self.late);
        late.clear();
        // The sky fills what the opaque world left, and translucent things are seen against it.
        let overlays = self.overlays(&cam);
        self.sky.draw(gpu, &mut late, frame, &cam, scene_buf, self.world.dim as usize, self.smooth_time(), self.camera().pos.y, self.eye_liquid().is_some(), self.rain, &overlays);
        self.render.draw_translucent(&mut late);
        let me = (self.view != 0 && !self.dead && self.mode != GameMode::Spectator).then(|| self.self_entity());
        let hand = (self.view == 0 && !self.dead && self.mode != GameMode::Spectator && !self.hide_gui).then(|| entity_render::Hand {
            stack: self.hand_stack,
            swing: self.hand_swing,
            lowered: 1.0 - self.hand_height,
            lag: (self.player.pitch - self.hand_view.0, angle_diff(self.player.yaw, self.hand_view.1)),
            using: self.using,
            anim: self.self_anim.clone(),
            age: self.start.elapsed().as_secs_f32(),
        });
        let world = &self.world;
        let light = |p: IVec3| world.light(p);
        let mut hand_late = std::mem::take(&mut self.hand_late);
        hand_late.clear();
        let mut cracks: Vec<(IVec3, f32, DVec3)> = self.others_mining.iter().map(|&(_, p, v)| (p, v, DVec3::ZERO)).collect();
        cracks.extend(self.breaking.map(|(f, p, v)| (p, v, self.frame_origin(f))));
        self.entity_render.draw(gpu, list, frame, &cam, &self.entities, me.as_ref(), hand, textures, &light, &scene, scene_buf, self.target.filter(|_| !self.hide_gui).map(|h| (h, self.frame_origin(h.frame))), &cracks, &self.gizmo_lines, &self.gizmo_fills, &mut hand_late);
        self.particles.draw(gpu, list, &mut late, frame, &cam, textures, scene_buf);
        self.sky.draw_overlay(&mut hand_late, scene_buf);
        list.append(&mut late);
        list.append(&mut hand_late);
        (self.late, self.hand_late) = (late, hand_late);
        list.push(weft_render::Cmd::Mark);
    }

    pub fn fog_color(&self) -> [f32; 4] {
        self.fog_linear().extend(1.0).to_array()
    }
}

/// Vanilla's tickHeadTurn: the body turns towards where it walks (or stays put), and the head
/// drags it along only once it has turned more than 50 degrees away. Radians, `moved` this frame.
pub(crate) fn turn_body(body: f32, head: f32, moved: DVec3, dt: f32) -> f32 {
    let mut body = body;
    let flat = moved.with_y(0.0);
    if flat.length_squared() > (0.05 * dt as f64 * 20.0).powi(2) * 0.0025 {
        // Backing up, the body faces the way the head does.
        let mut toward = (-flat.x as f32).atan2(-flat.z as f32);
        if angle_diff(toward, head).abs() > 95f32.to_radians() {
            toward += std::f32::consts::PI;
        }
        body += angle_diff(toward, body) * (1.0 - 0.7f32.powf(dt * crate::world::TPS as f32));
    }
    let off = angle_diff(head, body);
    let limit = 50f32.to_radians();
    if off.abs() > limit {
        body += off - limit * off.signum();
    }
    // Keep it near the head, so their difference never needs wrapping where it is used.
    head - angle_diff(head, body)
}

/// `a - b` the short way around, radians.
fn angle_diff(a: f32, b: f32) -> f32 {
    (a - b + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

/// One 20 Hz tick of player movement with vanilla's numbers (blocks per tick): acceleration
/// scaled by ground slipperiness, 0.91 air drag, 0.98 vertical drag, 0.08 gravity, 0.42 jump,
/// so jumps, sprint-jumps and parkur distances match.
fn step_player(p: &mut LocalPlayer, input: &Input, block: &impl Fn(IVec3) -> Option<u32>, gravity: f64, bodies: &[physics::Body]) {
    let c = content();
    // The body the player rides moves it first, like on the server.
    let (hw, _, _) = p.dims();
    if !p.noclip {
        let h = p.dims().1;
        let ride = physics::ride(p.pos, hw, &mut p.support, bodies);
        if ride.carry != DVec3::ZERO {
            let moved = physics::move_box(c, block, p.pos, hw, h, ride.carry, 0.0).pos;
            // The carry is part of the step, so the camera glides along with the body (drawn a
            // step behind, like the player).
            p.carried += moved - p.pos;
            p.pos = moved;
        }
        if let Some(d) = ride.released {
            p.vel += d;
        }
        let (shift, _) = physics::push(c, p.pos, hw, h, bodies, 0, p.support);
        if shift != DVec3::ZERO {
            p.pos = physics::move_box(c, block, p.pos, hw, h, shift, 0.0).pos;
        }
    } else {
        p.support = None;
    }
    let mut fwd_in = 0.0f64;
    let mut side_in = 0.0f64;
    if input.held("core:forward") {
        fwd_in += 1.0;
    }
    if input.held("core:back") {
        fwd_in -= 1.0;
    }
    if input.held("core:left") {
        side_in -= 1.0;
    }
    if input.held("core:right") {
        side_in += 1.0;
    }
    let sneak = p.sneak;
    let jump = input.held("core:jump");
    // Sprinting needs forward movement and stops when walking backwards or sneaking.
    let sprint = p.sprint && fwd_in > 0.0 && !sneak && !p.using && (p.can_sprint || p.flying);
    // Sprinting swims once the eyes are in a liquid that can be swum in, and keeps at it while the
    // body is. In a liquid it is no use otherwise.
    let can_swim = |at: DVec3| physics::liquid_at(c, block, at).is_some_and(|l| p.swim_anywhere || c.liquids[l as usize].swimmable);
    let feet_wet = physics::in_liquid(c, block, p.pos, 0.0);
    let start_at = p.pos + DVec3::Y * if matches!(p.pose, Pose::Swim | Pose::Crawl) { 0.1 } else { p.dims().2 };
    let swimming = sprint && !p.flying && !p.noclip && can_swim(start_at);
    let sprint = sprint && (swimming || !feet_wet);
    p.sprinting = sprint;
    // A pose where it fits, and if nothing else fits it crawls (a swimming box under a trapdoor).
    let want = if swimming {
        Pose::Swim
    } else if sneak && !p.flying && !p.noclip {
        Pose::Crouch
    } else {
        Pose::Stand
    };
    let fits = |pose: Pose| p.noclip || !physics::collides(c, block, physics::Aabb::around(p.pos, hw - 1e-3, physics::posed(pose, p.size[1], p.size[2]).0));
    p.pose = if fits(want) {
        want
    } else if fits(Pose::Crouch) {
        Pose::Crouch
    } else if feet_wet {
        Pose::Swim
    } else {
        Pose::Crawl
    };
    if !p.flying && matches!(p.pose, Pose::Crouch | Pose::Crawl) {
        fwd_in *= 0.3;
        side_in *= 0.3;
    }
    // Eating and the like: modern's fifth of the speed.
    if p.using {
        fwd_in *= 0.2;
        side_in *= 0.2;
    }
    let (fwd_in, side_in) = (fwd_in * 0.98, side_in * 0.98);
    let (sy, cy) = (p.yaw as f64).sin_cos();
    let forward = DVec3::new(-sy, 0.0, -cy);
    let right = DVec3::new(cy, 0.0, -sy);
    // Vanilla's moveRelative input: normalized only when longer than 1.
    let wish = forward * fwd_in + right * side_in;
    let g = 0.08 * gravity / 32.0;

    let height = p.dims().1;
    let before = p.pos;

    if p.flying || p.noclip {
        let fly = 0.05 * p.fly_speed as f64 * if sprint { 2.0 } else { 1.0 };
        if jump {
            p.vel.y += fly * 3.0;
        }
        if sneak {
            p.vel.y -= fly * 3.0;
        }
        let len2 = wish.length_squared();
        if len2 > 1e-7 {
            p.vel += wish * (fly / len2.sqrt().max(1.0));
        }
        if p.noclip {
            p.pos += p.vel;
            p.on_ground = false;
        } else {
            let r = physics::move_box_with(c, block, bodies, 0, p.pos, hw, height, p.vel, 0.0);
            p.pos = r.pos;
            for i in 0..3 {
                if r.hit[i] {
                    p.vel[i] = 0.0;
                }
            }
            p.on_ground = r.on_ground;
            p.landed |= p.on_ground;
            // Flying creative players stop flying when they come down onto the ground.
            if r.on_ground && r.hit[1] {
                p.flying = false;
            }
        }
        p.in_liquid = physics::in_liquid(c, block, p.pos, 0.0);
        p.vel.y *= 0.6;
        p.vel.x *= 0.91;
        p.vel.z *= 0.91;
        return;
    }

    // Walking is the same step mobs take; the player only brings its input.
    p.jump_cooldown = p.jump_cooldown.saturating_sub(1);
    if !jump {
        p.jump_cooldown = 0;
    }
    let ctl = physics::Control {
        wish,
        ground: if sprint { 0.13 } else { 0.1 } * p.speed_mul,
        air: if sprint { 0.026 } else { 0.02 },
        jump: if jump && p.jump_cooldown == 0 { 0.42 * p.jump_mul } else { 0.0 },
        jump_boost: if sprint { forward * 0.2 } else { DVec3::ZERO },
        swim: jump,
        careful: sneak,
        sprint: swimming,
        swim_look: swimming.then(|| (p.pitch as f64).sin()),
    };
    let frame = physics::Frame { half_width: hw, height, step: 0.6, gravity: g };
    let w = physics::walk(c, block, bodies, 0, &mut p.pos, &mut p.vel, p.on_ground, &ctl, &frame);
    if w.jumped {
        p.jump_cooldown = 10;
    }
    p.in_liquid = w.in_liquid;
    if w.on_ground {
        if let Some(b) = physics::standing_on(c, p.pos, hw, bodies, 0) {
            p.support = Some(b);
        }
    }
    p.on_ground = w.on_ground || (p.pos == before && p.on_ground);
    p.landed |= p.on_ground;
}
