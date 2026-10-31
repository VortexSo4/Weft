//! What the server said and what the terrain thread finished, taken once a frame.

use super::*;

impl Client {
    pub(super) fn receive(&mut self) {
        // World messages go to the terrain thread, all of a frame together.
        let mut fwd = Vec::new();
        while let Some(msg) = self.conn.recv() {
            self.receive_message(msg, &mut fwd);
        }
        if !fwd.is_empty() {
            let _ = self.terrain.tx.send(terrain::In::Many(fwd));
        }
        self.take_terrain();
        if self.conn.is_closed() && self.disconnected.is_none() {
            self.disconnected = Some("connection lost".into());
        }
        for pos in std::mem::take(&mut self.world.removed) {
            self.render.remove((0, pos));
        }
    }

    fn receive_message(&mut self, msg: ToClient, fwd: &mut Vec<terrain::In>) {
        match msg {
            ToClient::Welcome { player, dim, pos, seed, look, selected, flying } => {
                self.player.flying = flying;
                self.player_id = player;
                (self.player.yaw, self.player.pitch) = (look[0], look[1]);
                self.selected = selected as usize;
                self.joined = true;
                self.player.pos = pos;
                self.player.prev = pos;
                self.seed = seed;
                self.reset_terrain(dim, fwd);
            }
            ToClient::Cube { dim, pos, cube, edit } if dim == self.world.dim => fwd.push(terrain::In::Cube { pos, cube, edit }),
            ToClient::Unload { dim, pos } if dim == self.world.dim => fwd.push(terrain::In::Unload(pos)),
            ToClient::Blocks { dim, changes } if dim == self.world.dim => fwd.push(terrain::In::Blocks(changes)),
            ToClient::Saved { dim, cubes } if dim == self.world.dim => fwd.push(terrain::In::Saved(cubes)),
            ToClient::Edits { dim, specs, all } if dim == self.world.dim => fwd.push(terrain::In::Edits { specs, all }),
            ToClient::Time { dim, time } if dim == self.world.dim => {
                self.world.time = time;
                self.since_time = 0;
            }
            ToClient::Teleport { dim, pos, frame } => self.teleported(dim, pos, frame, fwd),
            ToClient::Spawn(list) => self.spawn_entities(list),
            ToClient::Moves { tick, moves } => self.receive_moves(tick, moves),
            ToClient::Structure { id, cubes, full } => {
                let dim = self.world.dim;
                if let Some(e) = self.entities.get_mut(&id) {
                    if full || e.structure.is_none() {
                        e.structure = Some(Box::new(ClientWorld::structure(dim)));
                        self.render.remove_frame(id);
                    }
                    let s = e.structure.as_mut().unwrap();
                    for (pos, cube) in cubes {
                        s.insert(pos, cube, Default::default());
                    }
                    s.scan_bounds();
                }
            }
            ToClient::Swing { used_item } => {
                self.swing_arm();
                if used_item && !self.held().is_empty() {
                    self.hand_height = 0.0;
                }
            }
            ToClient::Taken(list) => {
                for (item, by) in list {
                    if let Some(e) = self.entities.get_mut(&item) {
                        e.pulled = Some((e.pos, by, 0.0));
                    }
                }
            }
            ToClient::Despawn(ids) => {
                self.entity_components.retain(|(e, _), _| !ids.iter().any(|g| g.0 == *e));
                for (id, killed) in ids {
                    // Killed mobs stay a moment for the death animation, taken items until
                    // they reach the taker.
                    match self.entities.get_mut(&id) {
                        Some(e) if e.pulled.is_some() => {}
                        Some(e) if (killed || e.health <= 0.0) && e.item.is_empty() && e.dying.is_none() => e.dying = Some(0.0),
                        _ => {
                            self.entities.remove(&id);
                        }
                    }
                }
            }
            ToClient::Players(names) => self.players = names,
            ToClient::GameMode(mode) => {
                self.mode = mode;
                self.player.flying = mode == GameMode::Spectator || (mode == GameMode::Creative && self.player.flying);
                self.player.noclip = mode == GameMode::Spectator;
                self.player.fly_speed = 1.0;
            }
            ToClient::Inventory(slots) => {
                for (i, s) in slots {
                    if let Some(dst) = self.inventory.get_mut(i as usize) {
                        *dst = s;
                    }
                }
            }
            ToClient::Components(list) => {
                for (k, v) in list {
                    self.engine_field(self.player_id, k, &v);
                    self.components.insert(k, v);
                }
            }
            ToClient::OpenMenu { kind, window, slots, owner, block, name } => {
                self.menu = Some(ClientMenu { kind, window, owner, block, name, slots: vec![ItemStack::EMPTY; slots as usize], cursor: ItemStack::EMPTY });
                self.events.push(ClientEvent::OpenMenu);
            }
            ToClient::MenuSlots(list) => {
                if let Some(m) = &mut self.menu {
                    for (i, s) in list {
                        if let Some(dst) = m.slots.get_mut(i as usize) {
                            *dst = s;
                        }
                    }
                }
            }
            ToClient::Cursor(s) => {
                if let Some(m) = &mut self.menu {
                    m.cursor = s;
                }
            }
            ToClient::CloseMenu => {
                self.menu = None;
                self.drag = None;
                self.events.push(ClientEvent::CloseMenu);
            }
            ToClient::Attributes(list) => {
                use crate::server::extend as a;
                for (id, v) in list {
                    if let Some(dst) = self.attributes.get_mut(id as usize) {
                        *dst = v;
                    }
                }
                self.player.speed_mul = self.attributes[a::MOVEMENT_SPEED as usize] as f64;
                self.player.jump_mul = self.attributes[a::JUMP_STRENGTH as usize] as f64;
                self.player.swim_anywhere = self.attributes[a::SWIM_ANYWHERE as usize] >= 1.0;
                self.player.can_sprint = self.attributes[a::CAN_SPRINT as usize] >= 1.0;
            }
            ToClient::BreakRefused { frame, pos } => {
                self.refused_break = Some((frame, pos));
                self.breaking = None;
            }
            ToClient::Permission(level) => self.permission = level,
            ToClient::Using(ticks) => self.using = ticks.map(|t| (t as f32, t as f32)),
            ToClient::Weather { raining, thunder } => self.weather = (raining, thunder),
            ToClient::Flash(id) => {
                if let Some(e) = self.entities.get_mut(&id) {
                    e.flash = 0.1;
                }
            }
            ToClient::EntitySwing(id) => {
                if let Some(e) = self.entities.get_mut(&id) {
                    if e.arm_swing.is_none_or(|p| p >= 0.5) {
                        e.arm_swing = Some(0.0);
                    }
                }
            }
            ToClient::Cracks(list) => self.others_mining = list,
            ToClient::StopSprint => self.controls.stop_sprint(),
            ToClient::Push((add, halve)) => {
                let v = &mut self.player.vel;
                if halve {
                    *v /= 2.0;
                }
                *v += DVec3::new(add[0] as f64, add[1] as f64, add[2] as f64);
                v.y = (v.y + 0.4).min(0.4);
            }
            ToClient::Died => {
                self.dead = true;
                self.events.push(ClientEvent::Died);
            }
            ToClient::Chat(line) => {
                log::info!("[chat] {} {:?}", line.key, line.args);
                self.chat_line(line);
            }
            ToClient::CommandReply { id, lines } => {
                self.command_replies.insert(id, lines);
            }
            ToClient::Disconnect(reason) => self.disconnected = Some(reason),
            ToClient::Gizmos(shapes) => self.gizmos.receive(shapes),
            ToClient::Packet { channel, data } => self.packets.push((channel, data)),
            ToClient::EntityComponents(list) => {
                for (id, comp, bytes) in list {
                    self.engine_field(id, comp, &bytes);
                    self.entity_components.insert((id, comp), bytes);
                }
            }
            // The same values in one process; a remote server's win while joined.
            ToClient::Configs(list) => {
                for (id, v) in list {
                    crate::config::put_from_server(id, v);
                }
            }
            ToClient::Skin { entity, png } => self.entity_render.skin_pending.push((entity, png)),
            ToClient::Animate(list) => {
                for (id, name, looping) in list {
                    if id == self.player_id {
                        self.self_anim = (!name.is_empty()).then(|| (name, self.start.elapsed().as_secs_f32(), looping));
                    } else if let Some(e) = self.entities.get_mut(&id) {
                        e.anim = (!name.is_empty()).then(|| (name, e.age, looping));
                    }
                }
            }
            ToClient::Sounds(list) => {
                self.sounds.extend(list.into_iter().map(|(sound, pos, volume, pitch)| crate::audio::Play { sound, pos: Some(pos), volume, pitch }));
            }
            ToClient::Particles(list) => {
                for b in &list {
                    self.particles.spawn(b, &self.world);
                }
            }
            _ => {}
        }
    }

    fn teleported(&mut self, dim: u32, pos: DVec3, frame: Option<(u64, DVec3)>, fwd: &mut Vec<terrain::In>) {
        if dim != self.world.dim {
            self.reset_terrain(dim, fwd);
            self.entities.clear();
            self.particles.clear();
            self.entity_components.clear();
        }
        let at = self.body_tick(self.body_clock);
        let on = frame.and_then(|(id, rel)| self.entities.get(&id).map(|b| (id, b.pose(at) + rel)));
        let pos = on.map_or(pos, |o| o.1);
        self.player.support = on.map(|o| o.0);
        // Off whatever it rode, or the next step would put it back there.
        if on.is_none() {
            self.player.riding = None;
        }
        self.player.pos = pos;
        self.player.prev = pos;
        self.player.vel = DVec3::ZERO;
    }

    fn spawn_entities(&mut self, list: Vec<crate::net::EntityView>) {
        for e in list {
            self.entities.insert(
                e.id,
                ClientEntity {
                    kind: e.kind,
                    pos: e.pos,
                    target: e.pos,
                    yaw: e.yaw,
                    target_yaw: e.yaw,
                    body_yaw: e.yaw,
                    arm_swing: None,
                    flash: 0.0,
                    pitch: e.pitch,
                    item: e.item,
                    // Arrives as a component right after: no hurt flash for the first.
                    health: f32::NAN,
                    hurt: 0.0,
                    worn: Vec::new(),
                    walk: 0.0,
                    swing: 0.0,
                    age: 0.0,
                    burning: false,
                    dying: None,
                    next_step: 0.0,
                    anim: None,
                    pose: Pose::Stand,
                    track: Default::default(),
                    pulled: None,
                    size: None,
                    look: None,
                    frame: 0,
                    rel: DVec3::ZERO,
                    rel_target: None,
                    structure: None,
                },
            );
        }
    }

    fn receive_moves(&mut self, tick: u64, list: Vec<crate::net::Move>) {
        // A long stall (or the first move) starts the timeline over.
        let sample = self.body_clock - STEP + self.accum - tick as f64 * STEP;
        match self.tick_offset {
            Some(o) if (sample - o).abs() < 0.25 => self.offset_target += (sample - self.offset_target) * 0.1,
            _ => (self.tick_offset, self.offset_target) = (Some(sample), sample),
        }
        let solid = |k: u32| content().game.entities[k as usize].is(weft_abi::entity_flags::SOLID);
        for &(id, pos, yaw, pitch, pose, frame) in &list {
            if let Some(e) = self.entities.get_mut(&id) {
                e.pose = pose;
                e.target = pos;
                e.target_yaw = yaw;
                e.pitch = pitch;
                e.frame = frame;
                if solid(e.kind) {
                    // A paused server keeps sending the same tick: one sample per
                    // tick, or two at the same time make an endless speed.
                    let t = tick as f64;
                    match e.track.back_mut() {
                        Some(last) if last.0 == t => last.1 = pos,
                        Some(last) if last.0 > t => {
                            e.track.clear();
                            e.track.push_back((t, pos));
                        }
                        _ => e.track.push_back((t, pos)),
                    }
                    while e.track.len() > 8 {
                        e.track.pop_front();
                    }
                }
            }
        }
        // Riders relative to their body, both where the server had them this tick.
        for &(id, pos, .., frame) in &list {
            let body = self.entities.get(&frame).filter(|_| frame != 0).map(|b| (b.target, b.pos));
            let Some(e) = self.entities.get_mut(&id) else { continue };
            match body {
                Some((target, shown)) => {
                    if e.rel_target.is_none() {
                        e.rel = e.pos - shown;
                    }
                    e.rel_target = Some(pos - target);
                }
                None => e.rel_target = None,
            }
        }
    }

    /// What the terrain thread finished, a batch at a time (shown together), for a few
    /// milliseconds a frame: taking all that came at once froze frames while flying. A loading
    /// batch may be cut there (a step of the view unloads a slice of the world); one with edits
    /// is taken whole.
    fn take_terrain(&mut self) {
        let start = Instant::now();
        while start.elapsed() < self.budget {
            let batch = match self.terrain_left.take() {
                Some(b) => b,
                None => match self.terrain.rx.try_recv() {
                    Ok(b) => b,
                    Err(_) => break,
                },
            };
            if batch.resets != self.terrain_resets {
                continue;
            }
            let (resets, whole) = (batch.resets, batch.whole);
            self.world.begin_batch(whole);
            let mut items = batch.out.into_iter();
            while let Some(o) = items.next() {
                match o {
                    terrain::Out::Cube { pos, cube, around } => self.world.insert(pos, cube, around),
                    terrain::Out::Blocks(changes) => {
                        for (p, s) in changes {
                            self.world.set_block(p, s);
                        }
                    }
                    terrain::Out::Unload(pos) => self.world.remove(pos),
                    terrain::Out::Range { lo, hi } => self.world.set_range(lo, hi),
                }
                if !whole && start.elapsed() >= self.budget {
                    let out: Vec<terrain::Out> = items.collect();
                    if !out.is_empty() {
                        self.terrain_left = Some(terrain::Batch { resets, whole, out });
                    }
                    break;
                }
            }
            self.world.end_batch();
        }
    }
}
