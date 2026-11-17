//! Tiny animation helpers: time-based tweens keyed by node id. The view asks
//! for another frame while any tween is running.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use flowie_core::NodeId;

pub const ROW_IN: Duration = Duration::from_millis(190);
pub const ROW_OUT: Duration = Duration::from_millis(150);
pub const ZOOM: Duration = Duration::from_millis(230);
pub const FLAG: Duration = Duration::from_millis(200);
/// Highlight on rows an agent just changed.
pub const FLASH: Duration = Duration::from_millis(1600);

/// Ease-out cubic.
pub fn ease_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Ease-in-out cubic.
pub fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Tween {
    start: Instant,
    dur: Duration,
}

impl Tween {
    pub fn new(dur: Duration) -> Self {
        Tween { start: Instant::now(), dur }
    }

    /// Linear progress 0..=1.
    pub fn t(&self) -> f32 {
        (self.start.elapsed().as_secs_f32() / self.dur.as_secs_f32()).min(1.0)
    }

    pub fn done(&self) -> bool {
        self.start.elapsed() >= self.dur
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Row appearing (height + opacity grow).
    Enter,
    /// Row disappearing (shrink + fade); the row is dropped when done.
    Leave,
    /// Chevron rotation / collapse state change.
    Fold,
    /// Completed state change (strike + fade).
    Complete,
    /// Background highlight fading out (rows changed by an agent).
    Flash,
}

#[derive(Default)]
pub struct Anims {
    tweens: HashMap<(NodeId, Channel), Tween>,
}

impl Anims {
    pub fn start(&mut self, id: NodeId, ch: Channel, dur: Duration) {
        self.tweens.insert((id, ch), Tween::new(dur));
    }

    /// Linear progress, or None if no tween is running on this channel.
    pub fn t(&self, id: NodeId, ch: Channel) -> Option<f32> {
        self.tweens.get(&(id, ch)).map(Tween::t)
    }

    pub fn is_leaving_done(&self, id: NodeId) -> bool {
        self.tweens.get(&(id, Channel::Leave)).is_none_or(Tween::done)
    }

    pub fn any_running(&self) -> bool {
        self.tweens.values().any(|t| !t.done())
    }

    /// Drop finished tweens, except `Leave` ones (the view removes those rows first).
    pub fn gc(&mut self) {
        self.tweens.retain(|(_, ch), t| *ch == Channel::Leave || !t.done());
    }

    pub fn clear_leave(&mut self, id: NodeId) {
        self.tweens.remove(&(id, Channel::Leave));
    }

    pub fn clear(&mut self) {
        self.tweens.clear();
    }
}
