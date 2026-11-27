//! Hybrid logical clocks, and the clock each synced field of a node carries
//! (wiki: Per-Node-Sync §3.1–3.2, §4.1).
//!
//! A clock is (wall milliseconds, counter, actor), compared in that order, so any two writes
//! are ordered the same way on every device. A writer's clock never goes backwards: it
//! follows the wall clock when that moves ahead, counts up while it doesn't (or goes back),
//! and catches up with every clock it sees, so a write sorts after everything its writer had
//! seen. The actor is one per writing process, so two processes never stamp the same clock.

/// A hybrid logical clock value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hlc {
    /// Milliseconds since the Unix epoch (48 bits are used).
    pub wall: u64,
    pub counter: u16,
    /// The writing process; 0 for clocks derived from data written before Flowie had them.
    pub actor: u64,
}

const WALL_MASK: u64 = (1 << 48) - 1;

impl Hlc {
    /// A clock for a time recorded without one (an old `created`/`modified`), from actor 0.
    pub fn at_millis(millis: i64) -> Hlc {
        Hlc { wall: millis.max(0) as u64 & WALL_MASK, counter: 0, actor: 0 }
    }

    /// 16 bytes: wall (6, big-endian), counter (2), actor (8). Byte order is clock order.
    pub fn to_bytes(self) -> [u8; 16] {
        let mut out = [0; 16];
        out[..6].copy_from_slice(&self.wall.to_be_bytes()[2..]);
        out[6..8].copy_from_slice(&self.counter.to_be_bytes());
        out[8..].copy_from_slice(&self.actor.to_be_bytes());
        out
    }

    pub fn from_bytes(b: &[u8]) -> Option<Hlc> {
        let b: &[u8; 16] = b.try_into().ok()?;
        let mut wall = [0; 8];
        wall[2..].copy_from_slice(&b[..6]);
        Some(Hlc {
            wall: u64::from_be_bytes(wall),
            counter: u16::from_be_bytes([b[6], b[7]]),
            actor: u64::from_be_bytes(b[8..].try_into().ok()?),
        })
    }

    /// The wire form: fixed-width hex `wall12-counter4-actor16`, which sorts as text in clock
    /// order and stays exact in JavaScript (an actor doesn't fit a JS number).
    pub fn to_wire(self) -> String {
        format!("{:012x}-{:04x}-{:016x}", self.wall, self.counter, self.actor)
    }

    pub fn from_wire(s: &str) -> Option<Hlc> {
        let mut parts = s.split('-');
        let (wall, counter, actor) = (parts.next()?, parts.next()?, parts.next()?);
        if parts.next().is_some() || wall.len() != 12 || counter.len() != 4 || actor.len() != 16 {
            return None;
        }
        Some(Hlc {
            wall: u64::from_str_radix(wall, 16).ok()?,
            counter: u16::from_str_radix(counter, 16).ok()?,
            actor: u64::from_str_radix(actor, 16).ok()?,
        })
    }
}

impl serde::Serialize for Hlc {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_wire())
    }
}

impl<'de> serde::Deserialize<'de> for Hlc {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Hlc, D::Error> {
        let s = String::deserialize(d)?;
        Hlc::from_wire(&s).ok_or_else(|| serde::de::Error::custom("expected a clock (wall12-counter4-actor16 in hex)"))
    }
}

/// One process's clock.
#[derive(Debug, Clone, Copy, Default)]
pub struct HlcClock {
    last: Hlc,
    actor: u64,
}

impl HlcClock {
    pub fn new(actor: u64) -> Self {
        HlcClock { last: Hlc { actor, ..Hlc::default() }, actor }
    }

    pub fn actor(&self) -> u64 {
        self.actor
    }

    pub fn set_actor(&mut self, actor: u64) {
        self.actor = actor;
        self.last.actor = actor;
    }

    /// A new clock for a local write at wall time `millis`: later than every clock this
    /// one has stamped or seen.
    pub fn tick(&mut self, millis: i64) -> Hlc {
        let wall = millis.max(0) as u64 & WALL_MASK;
        self.last = if wall > self.last.wall {
            Hlc { wall, counter: 0, actor: self.actor }
        } else if self.last.counter == u16::MAX {
            Hlc { wall: self.last.wall + 1, counter: 0, actor: self.actor }
        } else {
            Hlc { wall: self.last.wall, counter: self.last.counter + 1, actor: self.actor }
        };
        self.last
    }

    /// Catch up with a clock seen elsewhere, so the next tick sorts after it.
    pub fn observe(&mut self, seen: Hlc) {
        if (seen.wall, seen.counter) > (self.last.wall, self.last.counter) {
            self.last.wall = seen.wall;
            self.last.counter = seen.counter;
        }
    }
}

/// A node's synced fields, each a last-writer-wins register with its own clock
/// (per-node-sync.md §3.2). Collapsed state and the edit time aren't synced, so they have none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reg {
    /// Parent and rank together, so a move is atomic.
    Position,
    Text,
    Note,
    Completed,
    Kind,
    Mirror,
    Image,
    Star,
    Deleted,
}

impl Reg {
    pub const ALL: [Reg; 9] =
        [Reg::Position, Reg::Text, Reg::Note, Reg::Completed, Reg::Kind, Reg::Mirror, Reg::Image, Reg::Star, Reg::Deleted];

    fn index(self) -> usize {
        self as usize
    }

    /// This register's bit in an outbox mask.
    pub fn bit(self) -> u16 {
        1 << self.index()
    }
}

/// The outbox bit for a record removed for good: the rest of the mask means nothing then.
pub const REMOVED: u16 = 1 << 15;

/// Every register's bit: a record new to the outbox.
pub const ALL_REGS: u16 = (1 << Reg::ALL.len()) - 1;

/// When a node was created, and when each of its synced fields was last written.
///
/// Every node has one, so it is kept small: until a field is written on its own, all of them
/// share one clock (`base`: the creation, or the edit time of a node saved before Flowie kept
/// clocks), and only a node with a field written since holds a clock per field.
#[derive(Debug, Clone, Default)]
pub struct Clocks {
    pub created: Hlc,
    base: Hlc,
    regs: Option<Box<[Hlc; 9]>>,
}

impl PartialEq for Clocks {
    fn eq(&self, other: &Self) -> bool {
        self.created == other.created && Reg::ALL.into_iter().all(|reg| self.get(reg) == other.get(reg))
    }
}

impl Eq for Clocks {}

impl std::hash::Hash for Clocks {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.created.hash(state);
        for reg in Reg::ALL {
            self.get(reg).hash(state);
        }
    }
}

impl Clocks {
    /// Every field written when the node was created.
    pub fn new(created: Hlc) -> Self {
        Clocks { created, base: created, regs: None }
    }

    /// For a node saved before Flowie kept clocks: created at `created`, every field last
    /// written at `modified` (milliseconds), by actor 0.
    pub fn derived(created: i64, modified: i64) -> Self {
        Clocks { created: Hlc::at_millis(created), base: Hlc::at_millis(modified), regs: None }
    }

    pub fn get(&self, reg: Reg) -> Hlc {
        self.regs.as_ref().map_or(self.base, |regs| regs[reg.index()])
    }

    pub fn set(&mut self, reg: Reg, at: Hlc) {
        match &mut self.regs {
            Some(regs) => regs[reg.index()] = at,
            None if at == self.base => {}
            None => {
                let mut regs = Box::new([self.base; 9]);
                regs[reg.index()] = at;
                self.regs = Some(regs);
            }
        }
    }

    /// The latest clock of the node: when it was last written.
    pub fn max(&self) -> Hlc {
        Reg::ALL.into_iter().map(|reg| self.get(reg)).fold(self.created, Hlc::max)
    }

    /// The saved form: `created`, then a (register, clock) pair for each field written after
    /// it. A field nobody touched since creation costs nothing.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.created.to_bytes().to_vec();
        for reg in Reg::ALL {
            let at = self.get(reg);
            if at != self.created {
                out.push(reg.index() as u8);
                out.extend_from_slice(&at.to_bytes());
            }
        }
        out
    }

    /// Reads [`Clocks::to_bytes`]; `None` for anything else (an empty blob included).
    pub fn from_bytes(b: &[u8]) -> Option<Clocks> {
        let mut clocks = Clocks::new(Hlc::from_bytes(b.get(..16)?)?);
        let mut rest = &b[16..];
        while !rest.is_empty() {
            let reg = *Reg::ALL.get(*rest.first()? as usize)?;
            clocks.set(reg, Hlc::from_bytes(rest.get(1..17)?)?);
            rest = &rest[17..];
        }
        Some(clocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clocks_order_by_wall_then_counter_then_actor_in_every_form() {
        let a = Hlc { wall: 5, counter: 9, actor: u64::MAX };
        let b = Hlc { wall: 6, counter: 0, actor: 1 };
        let c = Hlc { wall: 6, counter: 0, actor: 2 };
        assert!(a < b && b < c);
        assert!(a.to_bytes() < b.to_bytes() && b.to_bytes() < c.to_bytes());
        assert!(a.to_wire() < b.to_wire() && b.to_wire() < c.to_wire());
        for h in [a, b, c] {
            assert_eq!(Hlc::from_bytes(&h.to_bytes()), Some(h));
            assert_eq!(Hlc::from_wire(&h.to_wire()), Some(h));
        }
        assert_eq!(c.to_wire(), "000000000006-0000-0000000000000002");
        assert_eq!(Hlc::from_wire("6-0-2"), None);
    }

    #[test]
    fn a_clock_never_goes_back_and_passes_what_it_sees() {
        let mut clock = HlcClock::new(7);
        let first = clock.tick(1000);
        assert_eq!(first, Hlc { wall: 1000, counter: 0, actor: 7 });
        // The wall clock went back: count up instead.
        let second = clock.tick(400);
        assert!(second > first);
        assert_eq!(second.wall, 1000);
        // A clock seen from a device ahead pulls this one along.
        clock.observe(Hlc { wall: 5000, counter: 3, actor: 1 });
        let third = clock.tick(1001);
        assert_eq!(third, Hlc { wall: 5000, counter: 4, actor: 7 });
        // The counter running out moves the wall on.
        let mut clock = HlcClock::new(1);
        clock.observe(Hlc { wall: 10, counter: u16::MAX, actor: 2 });
        assert_eq!(clock.tick(0), Hlc { wall: 11, counter: 0, actor: 1 });
    }

    #[test]
    fn clocks_hold_one_clock_until_a_field_is_written_on_its_own() {
        // Every node has clocks (#18): 56 bytes, not ten clocks inline.
        assert!(std::mem::size_of::<Clocks>() <= 56, "{}", std::mem::size_of::<Clocks>());
        let created = Hlc { wall: 100, counter: 0, actor: 1 };
        let mut clocks = Clocks::new(created);
        clocks.set(Reg::Text, created);
        assert!(clocks.regs.is_none());
        let mut spelled_out = Clocks::new(created);
        spelled_out.regs = Some(Box::new([created; 9]));
        assert_eq!(clocks, spelled_out, "equal by what they say, not how they keep it");

        let later = Hlc { wall: 200, ..created };
        clocks.set(Reg::Note, later);
        assert_eq!((clocks.get(Reg::Note), clocks.get(Reg::Text), clocks.max()), (later, created, later));
        let derived = Clocks::derived(100, 300);
        assert_eq!((derived.created.wall, derived.get(Reg::Position).wall, derived.max().wall), (100, 300, 300));
    }

    #[test]
    fn clocks_save_only_the_fields_written_after_creation() {
        let created = Hlc { wall: 100, counter: 0, actor: 3 };
        let mut clocks = Clocks::new(created);
        assert_eq!(clocks.to_bytes().len(), 16);
        clocks.set(Reg::Text, Hlc { wall: 200, counter: 1, actor: 3 });
        clocks.set(Reg::Position, Hlc { wall: 300, counter: 0, actor: 4 });
        let saved = clocks.to_bytes();
        assert_eq!(saved.len(), 16 + 2 * 17);
        assert_eq!(Clocks::from_bytes(&saved), Some(clocks.clone()));
        assert_eq!(clocks.max(), Hlc { wall: 300, counter: 0, actor: 4 });
        assert_eq!(Clocks::from_bytes(&[]), None);
        assert_eq!(Clocks::from_bytes(&saved[..20]), None);
    }
}
