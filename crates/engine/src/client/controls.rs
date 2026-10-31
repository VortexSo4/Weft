//! What the movement keys ask for: sneak and sprint held or toggled, double taps.

use std::time::{Duration, Instant};

use crate::input::Input;

/// Like vanilla's 7 ticks.
const DOUBLE_TAP: Duration = Duration::from_millis(350);

/// This frame's keys and the settings that give them their meaning.
#[derive(Clone, Copy, Default)]
pub(super) struct Keys {
    /// Keys that switch something only count while the game has the mouse.
    pub captured: bool,
    pub toggle_sneak: bool,
    pub toggle_sprint: bool,
    pub sneak: bool,
    pub sneak_pressed: bool,
    pub sprint: bool,
    pub drop_pressed: bool,
    pub forward: bool,
    pub forward_pressed: bool,
    pub back: bool,
    pub jump_pressed: bool,
    pub fly_pressed: bool,
}

impl Keys {
    pub fn of(input: &Input) -> Keys {
        Keys {
            captured: input.captured,
            toggle_sneak: input.toggle_sneak,
            toggle_sprint: input.toggle_sprint,
            sneak: input.held("core:sneak"),
            sneak_pressed: input.pressed("core:sneak"),
            sprint: input.held("core:sprint"),
            drop_pressed: input.pressed("core:drop"),
            forward: input.held("core:forward"),
            forward_pressed: input.pressed("core:forward"),
            back: input.held("core:back"),
            jump_pressed: input.pressed("core:jump"),
            fly_pressed: input.pressed("core:fly"),
        }
    }
}

/// What the keys come to this frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Wanted {
    pub sneak: bool,
    pub sprint: bool,
    pub flying: bool,
}

#[derive(Default)]
pub(super) struct Controls {
    last_jump: Option<Instant>,
    last_forward: Option<Instant>,
    /// Sprinting from a double tap of forward; it lasts while forward is held.
    double_tap_sprint: bool,
    sneak_toggle: bool,
    sprint_toggle: bool,
    /// The sprint key down with toggle sprint on, and whether it was used for dropping a stack.
    sprint_key: Option<bool>,
}

impl Controls {
    pub fn read(&mut self, k: &Keys, flying: bool, creative: bool, now: Instant) -> Wanted {
        // Sneak and sprint can be switched on and off instead of held. Sprinting still needs
        // forward movement, and flying keeps the sneak key as "go down".
        if k.captured {
            if k.toggle_sneak && !flying && k.sneak_pressed {
                self.sneak_toggle = !self.sneak_toggle;
            }
            if !k.toggle_sneak {
                self.sneak_toggle = false;
            }
            // Switches when let go: the same key held with drop throws a whole stack instead.
            if !k.toggle_sprint {
                (self.sprint_toggle, self.sprint_key) = (false, None);
            } else if k.sprint {
                *self.sprint_key.get_or_insert(false) |= k.drop_pressed;
            } else if self.sprint_key.take() == Some(false) {
                self.sprint_toggle = !self.sprint_toggle;
            }
            // Sneaking ends a sprint, toggled or not.
            if k.sneak_pressed {
                self.sprint_toggle = false;
            }
        }
        let sneak = if k.toggle_sneak && !flying { self.sneak_toggle } else { k.sneak };
        let mut flying = flying;
        if creative && k.captured {
            if k.fly_pressed {
                flying = !flying;
            }
            if k.jump_pressed {
                match self.last_jump.take() {
                    Some(t) if now.duration_since(t) < DOUBLE_TAP => flying = !flying,
                    _ => self.last_jump = Some(now),
                }
            }
        }
        // Double tap forward to sprint, like vanilla; the sprint key still works on its own.
        // Sprinting stops as soon as the player is no longer walking forward.
        if k.captured {
            if k.forward_pressed {
                match self.last_forward.take() {
                    Some(t) if now.duration_since(t) < DOUBLE_TAP => self.double_tap_sprint = true,
                    _ => self.last_forward = Some(now),
                }
            }
            if !k.forward || k.back || sneak {
                self.double_tap_sprint = false;
            }
        }
        let sprint_key = if k.toggle_sprint { self.sprint_toggle } else { k.sprint };
        Wanted { sneak, sprint: sprint_key || self.double_tap_sprint, flying }
    }

    /// The server ended the sprint (no food, say); the key has to be pressed again.
    pub fn stop_sprint(&mut self) {
        self.double_tap_sprint = false;
        self.sprint_toggle = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEYS: Keys = Keys {
        captured: true,
        toggle_sneak: false,
        toggle_sprint: false,
        sneak: false,
        sneak_pressed: false,
        sprint: false,
        drop_pressed: false,
        forward: false,
        forward_pressed: false,
        back: false,
        jump_pressed: false,
        fly_pressed: false,
    };

    fn at(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn double_tap_forward_sprints_while_held() {
        let (mut c, t) = (Controls::default(), Instant::now());
        let tap = Keys { forward: true, forward_pressed: true, ..KEYS };
        let hold = Keys { forward: true, ..KEYS };
        assert!(!c.read(&tap, false, false, at(t, 0)).sprint);
        c.read(&KEYS, false, false, at(t, 100));
        assert!(c.read(&tap, false, false, at(t, 200)).sprint);
        assert!(c.read(&hold, false, false, at(t, 300)).sprint);
        assert!(!c.read(&KEYS, false, false, at(t, 400)).sprint, "letting go ends it");
    }

    #[test]
    fn slow_taps_do_not_sprint() {
        let (mut c, t) = (Controls::default(), Instant::now());
        let tap = Keys { forward: true, forward_pressed: true, ..KEYS };
        c.read(&tap, false, false, at(t, 0));
        assert!(!c.read(&tap, false, false, at(t, 500)).sprint);
    }

    #[test]
    fn sneaking_or_going_back_ends_a_double_tap_sprint() {
        for stop in [Keys { forward: true, sneak: true, ..KEYS }, Keys { forward: true, back: true, ..KEYS }] {
            let (mut c, t) = (Controls::default(), Instant::now());
            let tap = Keys { forward: true, forward_pressed: true, ..KEYS };
            c.read(&tap, false, false, at(t, 0));
            assert!(c.read(&tap, false, false, at(t, 100)).sprint);
            assert!(!c.read(&stop, false, false, at(t, 200)).sprint);
        }
    }

    #[test]
    fn toggle_sprint_switches_on_release_unless_the_key_dropped_a_stack() {
        let (mut c, t) = (Controls::default(), Instant::now());
        let on = Keys { toggle_sprint: true, ..KEYS };
        let down = Keys { sprint: true, ..on };
        assert!(!c.read(&down, false, false, t).sprint, "not yet while it is down");
        assert!(c.read(&on, false, false, t).sprint, "on when let go");
        c.read(&Keys { drop_pressed: true, ..down }, false, false, t);
        assert!(c.read(&on, false, false, t).sprint, "dropping a stack switches nothing");
        c.read(&Keys { sneak_pressed: true, sneak: true, ..on }, false, false, t);
        assert!(!c.read(&on, false, false, t).sprint, "sneaking ends it");
    }

    #[test]
    fn toggle_sneak_flips_on_press_but_not_in_flight() {
        let (mut c, t) = (Controls::default(), Instant::now());
        let on = Keys { toggle_sneak: true, ..KEYS };
        let press = Keys { sneak: true, sneak_pressed: true, ..on };
        assert!(c.read(&press, false, false, t).sneak);
        assert!(c.read(&on, false, false, t).sneak, "stays after letting go");
        assert!(c.read(&press, true, false, t).sneak, "in flight the key is held");
        assert!(!c.read(&on, true, false, t).sneak);
    }

    #[test]
    fn double_tap_jump_flies_only_in_creative() {
        let (mut c, t) = (Controls::default(), Instant::now());
        let jump = Keys { jump_pressed: true, ..KEYS };
        assert!(!c.read(&jump, false, true, at(t, 0)).flying);
        assert!(c.read(&jump, false, true, at(t, 100)).flying);
        let (mut c, t) = (Controls::default(), Instant::now());
        c.read(&jump, false, false, at(t, 0));
        assert!(!c.read(&jump, false, false, at(t, 100)).flying);
    }

    #[test]
    fn nothing_switches_without_the_mouse() {
        let (mut c, t) = (Controls::default(), Instant::now());
        let tap = Keys { captured: false, forward: true, forward_pressed: true, ..KEYS };
        c.read(&tap, false, false, at(t, 0));
        assert!(!c.read(&tap, false, false, at(t, 100)).sprint);
    }
}
