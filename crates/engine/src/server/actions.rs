//! What players ask for, one list of what each message takes (`allowed`), then the actions.

use super::*;

impl Server {
    pub(super) fn handle_messages(&mut self, c: &'static Content) {
        for pi in 0..self.players.len() {
            while let Some(msg) = self.players[pi].conn.recv() {
                self.handle(c, pi, msg);
                self.settle(c);
            }
        }
    }

    /// Items held long enough take effect; another item in hand ends it.
    pub(super) fn finish_uses(&mut self, c: &'static Content) {
        for pi in 0..self.players.len() {
            let Some((item, done)) = self.players[pi].using else { continue };
            let held = self.region(pi).and_then(|(r, id)| r.entities.get(id).map(|e| e.selected).and_then(|sel| r.entities.inventories.get(&id).and_then(|i| i.get(sel as usize)).copied())).unwrap_or_default();
            if held.item != item || held.is_empty() {
                self.players[pi].using = None;
                self.players[pi].conn.send(ToClient::Using(None));
            } else if self.tick >= done {
                self.players[pi].using = None;
                self.players[pi].conn.send(ToClient::Using(None));
                self.players[pi].use_finishing = true;
                self.handle(c, pi, ToServer::UseItem { target: None });
                self.players[pi].use_finishing = false;
            }
        }
    }

    /// The region holding player `pi`'s entity, with the entity id.
    pub(super) fn region(&mut self, pi: usize) -> Option<(&mut Dim, u64)> {
        let p = &self.players[pi];
        let id = p.entity;
        self.dims[p.dim as usize].entity_region(id).map(|r| (r, id))
    }

    /// Whether player `pi` may send this at all. One list before the actions, so a new action
    /// can't forget that its sender may be dead, a spectator or not in the game yet.
    pub(super) fn allowed(&self, pi: usize, msg: &ToServer) -> bool {
        use ToServer as M;
        let p = &self.players[pi];
        if matches!(msg, M::Join { .. } | M::Leave) {
            return true;
        }
        let Some(e) = self.dims[p.dim as usize].entity(p.entity).filter(|_| p.joined) else { return false };
        match msg {
            // Sent by a modified client only; they would index out of bounds.
            M::Use { face, .. } | M::UseItem { target: Some((_, face)) } if *face >= 6 => false,
            M::Respawn => !e.alive(),
            M::Break { .. } | M::Use { .. } | M::UseItem { .. } | M::Attack(_) | M::Interact(_) | M::Drop { .. } => e.alive() && e.mode != GameMode::Spectator,
            M::Move { .. } | M::OpenInventory | M::Click { .. } | M::Drag { .. } | M::CreativeTake { .. } | M::Pick { .. } => e.alive(),
            _ => true,
        }
    }

    pub(super) fn handle(&mut self, c: &'static Content, pi: usize, msg: ToServer) {
        if !self.allowed(pi, &msg) {
            log::debug!(target: "net", "refused {} from {}", crate::net::Message::kind(&msg), self.players[pi].name);
            return;
        }
        match msg {
            ToServer::Join { name, key } => self.join(c, pi, name, key),
            ToServer::StartBreak { pos, frame } => {
                self.players[pi].mining = Some((frame, pos, self.tick));
                let Some((r, entity)) = self.region(pi) else { return };
                let held = r.entities.held(entity).item;
                // Said now, or it would mine all that time for nothing.
                if r.with_space(c, frame, |d, _| d.may_break(c, pos, held, entity)) == Some(false) {
                    self.players[pi].mining = None;
                    self.players[pi].conn.send(ToClient::BreakRefused { frame, pos });
                }
            }
            ToServer::StopBreak => self.players[pi].mining = None,
            ToServer::StopUsing => self.players[pi].using = None,
            ToServer::Move { pos, yaw, pitch, on_ground, pose, sprint, flying, frame } => {
                if !pos.is_finite() {
                    return;
                }
                // On a body: where the client has the player on it, on the body as it is here.
                let dim = &self.dims[self.players[pi].dim as usize];
                let on = frame.filter(|f| f.1.is_finite()).and_then(|(b, rel)| dim.entity_region_ref(b).and_then(|r| r.entities.get(b)).map(|e| (b, e.pos + rel)));
                let pos = on.map_or(pos, |o| o.1);
                let p = &mut self.players[pi];
                if let Some(t) = p.teleported {
                    if (pos - t).length_squared() > 16.0 {
                        return;
                    }
                    p.teleported = None;
                    p.last_y = pos.y;
                }
                let dy = pos.y - p.last_y;
                p.last_y = pos.y;
                let Some((r, entity)) = self.region(pi) else { return };
                let state = r.get(pos.floor().as_ivec3());
                let in_liquid = state.is_some_and(|s| c.states[s as usize].liquid.is_some());
                let climbing = state.is_some_and(|s| c.states[s as usize].flags & abi::block_flags::CLIMBABLE != 0);
                let mut fall = r.entities.get(entity).map_or(0.0, |e| e.fall);
                let land = entity::track_fall(&mut fall, dy, on_ground, in_liquid || climbing).and_then(|f| entity::fall_damage(&c.game.entities[c.game.player_kind as usize], f));
                if let Some(e) = r.entities.get_mut(entity) {
                    e.fall = fall;
                    e.sprinting = sprint;
                    e.flying = flying;
                    e.jumped = e.on_ground && !on_ground && dy > 0.0;
                    e.moved = ((pos.x - e.pos.x).powi(2) + (pos.z - e.pos.z).powi(2)).sqrt() as f32;
                    e.pos = pos;
                    e.support = on.map(|o| o.0);
                    e.yaw = yaw;
                    e.pitch = pitch;
                    e.on_ground = on_ground;
                    e.in_liquid = in_liquid;
                    e.pose = pose;
                }
                if let Some(amount) = land {
                    r.damage(c, entity, amount, 0, damage::FALL);
                }
            }

            ToServer::Break { pos, frame } => {
                let mining = self.players[pi].mining.take();
                let now = self.tick;
                let Some((r, entity)) = self.region(pi) else { return };
                let Some(e) = r.entities.get(entity) else { return };
                let (eye, sel, creative, on_ground) = (e.pos + DVec3::Y * e.eye(c), e.selected, e.mode == GameMode::Creative, e.on_ground);
                let head_wet = r.head_in_liquid(c, e);
                let speed = r.attribute(c, entity, extend::MINING_SPEED, false) as f32;
                let reach = r.attribute(c, entity, extend::BLOCK_REACH, false) + BLOCK_SLACK;
                let mut refused = false;
                let held = r.entities.held(entity);
                let worn = r.with_space(c, frame, |d, o| {
                    // How long it should have taken, with room for lag: the client times it, and
                    // a modified one could say anything.
                    let started = mining.filter(|m| m.0 == frame && m.1 == pos).map_or(now, |m| m.2);
                    let need = d.get(pos).map_or(0.0, |s| dim::mining_time(c, s, held.item, head_wet, on_ground, speed)) * crate::world::TPS as f32;
                    let too_soon = !creative && ((now - started) as f32 + 2.0) < need * 0.7;
                    let keeps = creative && c.no_creative_break(held.item);
                    if !within(eye - o, pos, reach) || too_soon || keeps {
                        // The client already shows air.
                        if let Some(state) = d.get(pos) {
                            d.changes.push((pos, state));
                        }
                        return false;
                    }
                    let free = c.states.get(d.get(pos).unwrap_or(0) as usize).is_some_and(|s| s.hardness <= 0.0);
                    let done = d.break_block(c, pos, held, entity, !creative);
                    // The client already shows air: a refused break sends the block back.
                    if !done && let Some(state) = d.get(pos) {
                        d.changes.push((pos, state));
                        refused = true;
                    }
                    done && !free
                });
                if refused {
                    self.players[pi].conn.send(ToClient::BreakRefused { frame, pos });
                }
                let Some((r, _)) = self.region(pi) else { return };
                if worn == Some(true) {
                    wear(c, r, entity, sel, 1);
                }
            }

            ToServer::Use { pos, face, look, sneak, frame } => {
                enum Did {
                    Used,
                    Placed,
                    Nothing,
                    NoBlock,
                }
                let Some((r, entity)) = self.region(pi) else { return };
                let Some(e) = r.entities.get(entity) else { return };
                let (eye, sel, player_box, creative) = (e.pos + DVec3::Y * e.eye(c), e.selected, e.aabb(c), e.mode == GameMode::Creative);
                let held = r.entities.held(entity);
                let reach = r.attribute(c, entity, extend::BLOCK_REACH, false) + BLOCK_SLACK;
                let did = r.with_space(c, frame, |d, o| {
                    if !within(eye - o, pos, reach) {
                        return Did::Nothing;
                    }
                    if !sneak && d.player_use(c, pos, face, entity) {
                        return Did::Used;
                    }
                    let Some(state) = c.items.get(held.item as usize).and_then(|i| i.block).filter(|_| !held.is_empty()) else { return Did::NoBlock };
                    let target = if d.get(pos).is_some_and(|s| c.states[s as usize].flags & abi::block_flags::REPLACEABLE != 0) {
                        pos
                    } else {
                        pos + crate::world::DIRS[face as usize]
                    };
                    let block_box = crate::physics::Aabb { min: target.as_dvec3(), max: target.as_dvec3() + DVec3::ONE };
                    let solid = c.states[state as usize].flags & abi::block_flags::SOLID != 0;
                    if (!solid || !player_box.shifted(-o).intersects(&block_box)) && d.place(c, target, face, state, look, entity, held.item) {
                        // A named stack names the block entity it becomes (a renamed chest).
                        if let (Some(name), Some(&id)) = (crate::stack::name(&held), d.entities.anchors.get(&target)) {
                            d.entities.comps.insert((id, c.game.name_comp), name.into_bytes().into_boxed_slice());
                        }
                        Did::Placed
                    } else {
                        Did::Nothing
                    }
                });
                match did {
                    Some(Did::Used) => self.players[pi].conn.send(ToClient::Swing { used_item: false }),
                    Some(Did::Placed) => {
                        if !creative {
                            r.entities.set_slot(entity, sel, ItemStack { count: held.count - 1, ..held });
                        }
                        self.players[pi].conn.send(ToClient::Swing { used_item: true });
                    }
                    // Items used on a block (buckets) work on the world's blocks only, so far.
                    Some(Did::NoBlock) => self.handle(c, pi, ToServer::UseItem { target: (frame == 0).then_some((pos, face)) }),
                    _ => {}
                }
            }

            ToServer::UseItem { target } => {
                let Some((r, entity)) = self.region(pi) else { return };
                let Some(sel) = r.entities.get(entity).map(|e| e.selected) else { return };
                let held = r.entities.held(entity);
                let Some(def) = c.items.get(held.item as usize) else { return };
                let (pos, face) = target.unwrap_or((IVec3::ZERO, 0));
                let state = target.and_then(|_| r.get(pos)).unwrap_or(0);
                let mut q = abi::Query { entity, item: held.item, pos: pos.to_array(), state, ..abi::Query::DEFAULT };
                if !r.ask(c, abi::question::USE_ITEM, &mut q) {
                    return;
                }
                // An item that takes time starts here, and comes back through this when it is done.
                let ticks = crate::content::use_ticks(c, held.item);
                let finishing = std::mem::take(&mut self.players[pi].use_finishing);
                if ticks > 0 && !finishing {
                    let done = self.tick + ticks as u64;
                    self.players[pi].using = Some((held.item, done));
                    self.players[pi].conn.send(ToClient::Using(Some(ticks)));
                    return;
                }
                let Some((r, entity)) = self.region(pi) else { return };
                let mut used = false;
                if def.on_use.is_none() {
                    if let Some(after) = r.use_container(c, entity, held) {
                        if after != held {
                            r.entities.set_slot(entity, sel, after);
                            used = true;
                        }
                    }
                } else if let Some(cb) = def.on_use {
                    let a = r.call(def.owner, cb, |w| abi::ItemUseArgs {
                        world: w,
                        player: entity,
                        stack: held,
                        pos: pos.to_array(),
                        face: face as u32,
                        has_target: target.is_some() as u32,
                        handled: 0,
                    });
                    if a.handled != 0 && a.stack != held {
                        r.entities.set_slot(entity, sel, a.stack);
                    }
                    used = a.handled != 0;
                }
                if used {
                    r.emit_engine(c, abi::event::ITEM_USED, &abi::ItemUsed { player: entity, stack: held, pos: pos.to_array(), has_target: target.is_some() as u32 });
                }
                if used {
                    self.players[pi].conn.send(ToClient::Swing { used_item: true });
                }
            }
            ToServer::Pick { item } => {
                let Some((r, entity)) = self.region(pi) else { return };
                let Some(e) = r.entities.get(entity) else { return };
                let (sel, creative) = (e.selected, e.mode == GameMode::Creative);
                let Some(inv) = r.entities.inventories.get(&entity) else { return };
                let current = inv.get(sel as usize).copied().unwrap_or_default();
                if let Some(j) = c.game.player_layout.main().map(|j| j as usize).find(|&j| inv.get(j).is_some_and(|s| s.item == item && !s.is_empty())) {
                    let found = inv[j];
                    r.entities.set_slot(entity, j as u32, current);
                    r.entities.set_slot(entity, sel, found);
                } else if creative && c.items.get(item as usize).is_some() {
                    r.entities.set_slot(entity, sel, ItemStack::new(item, 1));
                }
            }
            ToServer::Select(slot) => {
                if slot < c.game.player_layout.hotbar {
                    if let Some((r, entity)) = self.region(pi) {
                        if let Some(e) = r.entities.get_mut(entity) {
                            // Changing what is in hand starts the swing over, as long as it does change.
                            if e.selected != slot {
                                e.swing_ticks = 0;
                            }
                            e.selected = slot;
                        }
                    }
                }
            }

            ToServer::Attack(target) => {
                let Some((r, entity)) = self.region(pi) else { return };
                let (Some(me), Some(t)) = (r.entities.get(entity), r.entities.get(target)) else { return };
                if !in_reach(c, r, me, t) {
                    return;
                }
                r.melee(c, entity, target);
            }

            ToServer::Interact(target) => {
                let Some((r, entity)) = self.region(pi) else { return };
                let Some(kind) = r.entities.get(target).map(|e| e.kind) else { return };
                if !r.entities.get(entity).zip(r.entities.get(target)).is_some_and(|(me, t)| in_reach(c, r, me, t)) {
                    return;
                }
                let held = r.entities.held(entity);
                let mut q = abi::Query { entity: target, other: entity, item: held.item, ..abi::Query::DEFAULT };
                if !r.ask(c, abi::question::INTERACT_ENTITY, &mut q) {
                    return;
                }
                let mut handled = false;
                for &(owner, cb) in &c.game.entities[kind as usize].on_interact {
                    handled |= r.call(owner, cb, |w| abi::EntityEventArgs { world: w, entity: target, other: entity, amount: 0.0, cause: 0, handled: 0 }).handled != 0;
                }
                if handled {
                    self.players[pi].conn.send(ToClient::Swing { used_item: false });
                }
            }
            ToServer::OpenInventory => {
                if let Some(m) = c.game.player_menu {
                    self.open_menu(c, pi, m, 0, 0);
                }
            }
            ToServer::Click { slot, button, shift } => {
                let Server { players, dims, .. } = self;
                let p = &mut players[pi];
                if let (Some(m), Some(r)) = (&mut p.menu, dims[p.dim as usize].entity_region(p.entity)) {
                    menu::click(m, r, c, p.entity, slot, button, shift);
                }
            }
            ToServer::CreativeTake { item, button } => {
                let Server { players, dims, .. } = self;
                let p = &mut players[pi];
                let creative = dims[p.dim as usize].entity(p.entity).is_some_and(|e| e.mode == GameMode::Creative);
                if let (true, Some(m), true) = (creative, &mut p.menu, item != 0 && (item as usize) < c.items.len()) {
                    // Like vanilla: the same item adds one, anything else held is thrown away
                    // first, an empty hand takes a stack (left) or one (right).
                    let max = c.max_stack(item);
                    m.cursor = if m.cursor.is_empty() {
                        ItemStack::new(item, if button == 1 { 1 } else { max })
                    } else if m.cursor.item == item && m.cursor.data == 0 {
                        ItemStack { count: (m.cursor.count + 1).min(max), ..m.cursor }
                    } else {
                        ItemStack::EMPTY
                    };
                }
            }
            ToServer::Drag { slots, button } => {
                let Server { players, dims, .. } = self;
                let p = &mut players[pi];
                if let (Some(m), Some(r)) = (&mut p.menu, dims[p.dim as usize].entity_region(p.entity)) {
                    menu::drag(m, r, c, p.entity, &slots, button);
                }
            }
            ToServer::CloseMenu => {
                let Server { players, dims, .. } = self;
                let p = &mut players[pi];
                if let (Some(m), Some(r)) = (p.menu.take(), dims[p.dim as usize].entity_region(p.entity)) {
                    menu::close(m, r, c, p.entity);
                }
            }

            ToServer::Drop { all } => {
                let Some((r, entity)) = self.region(pi) else { return };
                let Some(e) = r.entities.get(entity) else { return };
                let (sel, from, yaw, pitch) = (e.selected, e.drop_point(c), e.yaw, e.pitch);
                let held = r.entities.held(entity);
                if held.is_empty() {
                    return;
                }
                let n = if all { held.count } else { 1 };
                r.entities.set_slot(entity, sel, ItemStack { count: held.count - n, ..held });
                let id = r.spawn_item(c, from, ItemStack { count: n, ..held });
                if let Some(it) = r.entities.get_mut(id) {
                    let (sy, cy) = (yaw as f64).sin_cos();
                    let (sp, cp) = (pitch as f64).sin_cos();
                    it.vel = DVec3::new(-sy * cp, sp + 0.2, -cy * cp) * 6.0;
                    it.pickup_delay = 40;
                }
            }
            ToServer::Respawn => {
                if let Some((r, id)) = self.region(pi) { if let Some(e) = r.entities.get_mut(id) { e.fall = 0.0; } }
                let spawn = self.dims[c.spawn_dim as usize].spawn_point(c);
                self.change_dim(c, pi, c.spawn_dim, spawn);
                if let Some((r, entity)) = self.region(pi) {
                    r.entities.reset_components(c, entity);
                    // The game mode's modifiers went with the rest.
                    let mode = r.entities.get(entity).map_or(GameMode::Survival, |e| e.mode);
                    mode_modifiers(c, r, entity, mode);
                    // After the components: modifiers are gone with them. Sent as any change.
                    let max = r.max_health(c, entity);
                    if let Some(e) = r.entities.get_mut(entity) {
                        e.health = max;
                        e.fire = 0;
                        e.hurt_cooldown = 0;
                    }
                }
            }
            ToServer::Chat(text) => {
                let name = self.players[pi].name.clone();
                if let Some(cmd) = text.strip_prefix('/') {
                    for line in self.command(c, pi, cmd) {
                        self.players[pi].conn.send(ToClient::Chat(line));
                    }
                } else {
                    log::info!("<{name}> {text}");
                    let line = Msg::new("chat.message", vec![Arg::Text(name), Arg::Text(text)]);
                    for p in &mut self.players {
                        p.conn.send(ToClient::Chat(line.clone()));
                    }
                }
            }
            ToServer::Command { id, line } => {
                let lines = self.command(c, pi, line.trim_start_matches('/'));
                self.players[pi].conn.send(ToClient::CommandReply { id, lines });
            }
            ToServer::Packet { channel, data } => {
                let Some((owner, cb)) = c.game.packets.get(channel as usize).and_then(|d| d.server) else { return };
                let Some((r, player)) = self.region(pi) else { return };
                let slice = abi::Slice::new(&data);
                r.call(owner, cb, |w| abi::PacketArgs { world: w, player, data: slice });
            }
            ToServer::Skin(png) => {
                // Only a skin's size of PNG: everyone else stores and uploads it.
                let size = (png.len() <= 65536 && png.starts_with(b"\x89PNG") && png.len() > 24).then(|| {
                    let at = |i: usize| u32::from_be_bytes(png[i..i + 4].try_into().unwrap());
                    (at(16), at(20))
                });
                if !self.players[pi].joined || !matches!(size, Some((64, 64) | (64, 32))) {
                    return;
                }
                let png = Arc::new(png);
                let entity = self.players[pi].entity;
                self.players[pi].skin = Some(png.clone());
                for p in self.players.iter_mut().filter(|p| p.joined) {
                    p.conn.send(ToClient::Skin { entity, png: png.to_vec() });
                }
            }
            ToServer::View(r) => {
                let c = content();
                let def = crate::config::by_name("core:view_distance").map(|id| &c.game.configs[id as usize]);
                self.players[pi].view_radius = def.map_or(r as f64, |d| d.fit(r as f64)) as i32;
            }
            ToServer::Gizmos(mut names) => {
                names.retain(|n| n.len() <= 64);
                names.truncate(64);
                self.players[pi].gizmos = names;
            }
            ToServer::Leave => {
                if !self.players[pi].joined {
                    return;
                }
                log::info!("{} left", self.players[pi].name);
                let line = Msg::new("multiplayer.left", vec![Arg::Text(self.players[pi].name.clone())]);
                for q in self.players.iter_mut().filter(|q| q.joined) {
                    q.conn.send(ToClient::Chat(line.clone()));
                }
                let Server { players, dims, storage, .. } = self;
                let p = &mut players[pi];
                if let Some(r) = dims[p.dim as usize].entity_region(p.entity) {
                    if let Some(m) = p.menu.take() {
                        menu::close(m, r, c, p.entity);
                    }
                    if let Some(s) = storage {
                        save_player(&s.lock(), p, r);
                    }
                    r.entities.remove(p.entity);
                }
                p.conn.send(ToClient::Disconnect("bye".into()));
                p.joined = false;
            }
        }
    }
}

/// The block is within reach of these eyes.
/// From the eyes to the nearest point of the target's box, like modern's.
pub(super) fn in_reach(c: &Content, d: &Dim, me: &Entity, target: &Entity) -> bool {
    let eye = me.pos + DVec3::Y * me.eye(c);
    let b = target.aabb(c);
    let reach = d.attribute(c, me.id, extend::ENTITY_REACH, false) + ENTITY_SLACK;
    (eye.clamp(b.min, b.max) - eye).length_squared() <= reach * reach
}

pub(super) fn within(eye: DVec3, block: IVec3, reach: f64) -> bool {
    (block.as_dvec3() + DVec3::splat(0.5) - eye).length_squared() < reach * reach
}

/// Damages the held tool; breaks it when used up. Creative players' tools never wear.
pub(super) fn wear(c: &Content, d: &mut Dim, player: u64, slot: u32, amount: u32) {
    if d.entities.get(player).is_some_and(|e| e.mode == GameMode::Creative) {
        return;
    }
    let Some(stack) = d.entities.inventories.get(&player).and_then(|i| i.get(slot as usize)).copied() else { return };
    let Some(def) = c.items.get(stack.item as usize) else { return };
    if def.durability == 0 || stack.is_empty() {
        return;
    }
    let damage = crate::stack::damage(&stack) + amount;
    let next = if damage >= def.durability { ItemStack::EMPTY } else { crate::stack::with_damage(stack, damage) };
    d.entities.set_slot(player, slot, next);
}
