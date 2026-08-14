//! Spatiotemporal composition of server resources over the `kernel`.
//!
//! This is the ablation seam for the server-lifecycle slice: every pane,
//! topology node, and broadcast subscriber is a `kernel::Component` with
//! inverses, and changes to a declared read key notify exactly that key's
//! declared observers.
//!
//! ## Temporal axis — `mount_pane`/`SpacetimeContext::unmount`
//!
//! Mounting a pane registers it under a stable kernel slot key as a live
//! topology leaf. The effect returns an [`Inverse`]; unmounting replays it in
//! reverse so the context returns to its pre-mount state with **no residue**
//! (no leaked slot, no dangling live-leaf entry, no dangling readers).
//!
//! ## Spatial axis — `SpacetimeContext::set` with declared `reads`
//!
//! A committed `set` on a pane key notifies only the components that declared
//! that key in their `reads`. The hub's ad-hoc "send to every viewer" is
//! replaced here by declared subscribers: a broadcaster declares the pane keys it
//! reads and reacts only to changes on those keys.
//!
//! Kernel keys are fixed `&'static str` slots, so dynamic pane IDs map to a
//! stable pool of slot keys. A slot is allocated for a live pane and freed on
//! unmount, so the pool never grows and never leaks.
#![cfg_attr(not(test), allow(dead_code))]

/// Stable number of pane slot keys.
pub(crate) const PANE_SLOT_COUNT: usize = 64;

macro_rules! pane_slots {
    ($($idx:expr => $name:ident: $key:literal;)*) => {
        $(
        #[allow(dead_code)]
        pub(crate) const $name: &str = $key;
        )*
        pub(crate) const PANE_KEYS: [&str; PANE_SLOT_COUNT] = [
            $( $name ),*
        ];
    };
}
pane_slots! {
    0  => PANE_KEY_0:  "pane_slot_0";
    1  => PANE_KEY_1:  "pane_slot_1";
    2  => PANE_KEY_2:  "pane_slot_2";
    3  => PANE_KEY_3:  "pane_slot_3";
    4  => PANE_KEY_4:  "pane_slot_4";
    5  => PANE_KEY_5:  "pane_slot_5";
    6  => PANE_KEY_6:  "pane_slot_6";
    7  => PANE_KEY_7:  "pane_slot_7";
    8  => PANE_KEY_8:  "pane_slot_8";
    9  => PANE_KEY_9:  "pane_slot_9";
    10 => PANE_KEY_10: "pane_slot_10";
    11 => PANE_KEY_11: "pane_slot_11";
    12 => PANE_KEY_12: "pane_slot_12";
    13 => PANE_KEY_13: "pane_slot_13";
    14 => PANE_KEY_14: "pane_slot_14";
    15 => PANE_KEY_15: "pane_slot_15";
    16 => PANE_KEY_16: "pane_slot_16";
    17 => PANE_KEY_17: "pane_slot_17";
    18 => PANE_KEY_18: "pane_slot_18";
    19 => PANE_KEY_19: "pane_slot_19";
    20 => PANE_KEY_20: "pane_slot_20";
    21 => PANE_KEY_21: "pane_slot_21";
    22 => PANE_KEY_22: "pane_slot_22";
    23 => PANE_KEY_23: "pane_slot_23";
    24 => PANE_KEY_24: "pane_slot_24";
    25 => PANE_KEY_25: "pane_slot_25";
    26 => PANE_KEY_26: "pane_slot_26";
    27 => PANE_KEY_27: "pane_slot_27";
    28 => PANE_KEY_28: "pane_slot_28";
    29 => PANE_KEY_29: "pane_slot_29";
    30 => PANE_KEY_30: "pane_slot_30";
    31 => PANE_KEY_31: "pane_slot_31";
    32 => PANE_KEY_32: "pane_slot_32";
    33 => PANE_KEY_33: "pane_slot_33";
    34 => PANE_KEY_34: "pane_slot_34";
    35 => PANE_KEY_35: "pane_slot_35";
    36 => PANE_KEY_36: "pane_slot_36";
    37 => PANE_KEY_37: "pane_slot_37";
    38 => PANE_KEY_38: "pane_slot_38";
    39 => PANE_KEY_39: "pane_slot_39";
    40 => PANE_KEY_40: "pane_slot_40";
    41 => PANE_KEY_41: "pane_slot_41";
    42 => PANE_KEY_42: "pane_slot_42";
    43 => PANE_KEY_43: "pane_slot_43";
    44 => PANE_KEY_44: "pane_slot_44";
    45 => PANE_KEY_45: "pane_slot_45";
    46 => PANE_KEY_46: "pane_slot_46";
    47 => PANE_KEY_47: "pane_slot_47";
    48 => PANE_KEY_48: "pane_slot_48";
    49 => PANE_KEY_49: "pane_slot_49";
    50 => PANE_KEY_50: "pane_slot_50";
    51 => PANE_KEY_51: "pane_slot_51";
    52 => PANE_KEY_52: "pane_slot_52";
    53 => PANE_KEY_53: "pane_slot_53";
    54 => PANE_KEY_54: "pane_slot_54";
    55 => PANE_KEY_55: "pane_slot_55";
    56 => PANE_KEY_56: "pane_slot_56";
    57 => PANE_KEY_57: "pane_slot_57";
    58 => PANE_KEY_58: "pane_slot_58";
    59 => PANE_KEY_59: "pane_slot_59";
    60 => PANE_KEY_60: "pane_slot_60";
    61 => PANE_KEY_61: "pane_slot_61";
    62 => PANE_KEY_62: "pane_slot_62";
    63 => PANE_KEY_63: "pane_slot_63";
}

/// A stable kernel key allocated for a live pane. Freed back to the pool on
/// unmount, so `&'static str` keys never leak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PaneSlot(pub(crate) usize);

impl PaneSlot {
    pub(crate) fn key(self) -> &'static str {
        PANE_KEYS[self.0]
    }
}

/// Allocates/frees [`PaneSlot`]s around a collection of live panes.
///
/// The allocator hands out the lowest free slot first (`O(1)` free-list
/// reuse), so the live set stays compact and the `&'static str` pool stays
/// bounded — the temporal counterpart of "no resource leaks". It is not `Sync`,
/// so it must live in the single-threaded hub, never in a worker thread.
#[derive(Default)]
pub(crate) struct SlotAllocator {
    free: Vec<usize>,
    high_water: usize,
}

impl SlotAllocator {
    pub(crate) fn allocate(&mut self) -> Option<PaneSlot> {
        if let Some(index) = self.free.pop() {
            Some(PaneSlot(index))
        } else if self.high_water < PANE_SLOT_COUNT {
            let index = self.high_water;
            self.high_water += 1;
            Some(PaneSlot(index))
        } else {
            None
        }
    }

    pub(crate) fn free(&mut self, slot: PaneSlot) {
        self.free.push(slot.0);
    }
}

/// The kernel key for workspace-level metadata mutations (frame epoch).
pub(crate) const WORKSPACE_KEY: &str = "workspace";
/// The kernel key for the topology node's live-leaf declaration.
pub(crate) const TOPOLOGY_KEY: &str = "session_topology";

/// Mount a pane value under a stable slot key as a live topology leaf.
///
/// `entry` is the pane's identity token (the hub routes it to the real
/// `TerminalPane`). The effect commits `ctx.set(key, entry)` — the kernel's
/// single write path — and returns an inverse that removes the key. Because the
/// inverse only `remove`s what the effect `set`, unmount always restores the
/// pre-mount state with no residue (the hub additionally retires the real PTY and
/// frees the slot in its `retire_pane`). Returns the mount id to pass to
/// [`KContext::unmount`].
pub(crate) fn mount_pane(ctx: &mut KContext, slot: PaneSlot, entry: u64) -> usize {
    let key = slot.key();
    let component = Component::new(Vec::new()).effect(move |c| {
        let had = c.has(key);
        let prev = c.get::<u64>(key).copied();
        c.set(key, entry);
        Box::new(move |c| {
            if had {
                if let Some(prev) = prev {
                    c.set(key, prev);
                } else {
                    c.remove(key);
                }
            } else {
                c.remove(key);
            }
        })
    });
    ctx.mount(component)
}

/// The kernel context, re-exported for hub and tests.
pub(crate) type KContext = kernel::Context;
/// Re-export of the kernel `Component` for hub/tests that build their own.
pub(crate) use kernel::Component;

/// Mount a subscriber component that reads `reads` and reacts to changes on
/// exactly those keys. `on_key` receives the changed key. Returns the mount id.
///
/// This is the spatial axis: a broadcaster declares the panes it observes and
/// hears only about changes to those panes, never about undeclared keys.
pub(crate) fn mount_subscriber(
    ctx: &mut KContext,
    reads: Vec<&'static str>,
    mut on_key: impl FnMut(&mut KContext, &'static str) + Send + 'static,
) -> usize {
    let component = Component::new(reads).on_change(move |ctx, key| on_key(ctx, key));
    ctx.mount(component)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Snapshot the set of live pane slot keys plus workspace/topology keys.
    fn snapshot(ctx: &KContext) -> Vec<&'static str> {
        let mut keys = Vec::new();
        for key in PANE_KEYS {
            if ctx.has(key) {
                keys.push(key);
            }
        }
        if ctx.has(WORKSPACE_KEY) {
            keys.push(WORKSPACE_KEY);
        }
        if ctx.has(TOPOLOGY_KEY) {
            keys.push(TOPOLOGY_KEY);
        }
        keys.sort_unstable();
        keys
    }

    /// Canon temporal check: snapshot, mount a pane, unmount, diff empty.
    #[test]
    fn unmount_leaves_no_residue() {
        let mut ctx = KContext::new();
        let before = snapshot(&ctx);

        let slot = {
            let mut alloc = SlotAllocator::default();
            alloc.allocate().expect("a free slot")
        };
        let mount = mount_pane(&mut ctx, slot, 42);
        assert!(ctx.has(slot.key()));
        assert_eq!(ctx.get::<u64>(slot.key()), Some(&42));

        ctx.unmount(mount);
        assert_eq!(snapshot(&ctx), before);
        assert!(!ctx.has(slot.key()));
        // The value-notify did not leave any residue either.
        assert_eq!(ctx.get::<u64>(slot.key()), None);
    }

    /// A second mount with a reused slot must not collide with the first.
    #[test]
    fn overlapping_mounts_unmount_independently() {
        let mut ctx = KContext::new();
        let mut alloc = SlotAllocator::default();
        let slot = alloc.allocate().expect("slot");
        let a = mount_pane(&mut ctx, slot, 1);
        let b = mount_pane(&mut ctx, slot, 2);
        assert_eq!(ctx.get::<u64>(slot.key()), Some(&2));
        ctx.unmount(b);
        assert_eq!(ctx.get::<u64>(slot.key()), Some(&1));
        ctx.unmount(a);
        assert!(!ctx.has(slot.key()));
    }

    /// Canon spatial check: change each declared key, exactly its readers
    /// react; undeclared keys notify nobody.
    #[test]
    fn spatial_notifies_only_declared_readers() {
        let mut ctx = KContext::new();
        let mut alloc = SlotAllocator::default();
        let a = alloc.allocate().expect("slot a");
        let b = alloc.allocate().expect("slot b");
        // Mount two panes so their keys exist as values.
        ctx.set(a.key(), 1u64);
        ctx.set(b.key(), 2u64);

        let hits_a = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hits_b = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let ta = hits_a.clone();
        mount_subscriber(&mut ctx, vec![a.key()], move |_, key| {
            assert_eq!(key, a.key());
            ta.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });
        let tb = hits_b.clone();
        mount_subscriber(&mut ctx, vec![b.key()], move |_, key| {
            assert_eq!(key, b.key());
            tb.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        });

        ctx.set(b.key(), 22u64);
        assert_eq!(hits_a.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(hits_b.load(std::sync::atomic::Ordering::Relaxed), 1);

        ctx.set(a.key(), 11u64);
        assert_eq!(hits_a.load(std::sync::atomic::Ordering::Relaxed), 1);
        assert_eq!(hits_b.load(std::sync::atomic::Ordering::Relaxed), 1);

        // Undeclared key change notifies nobody.
        let c0 = hits_a.load(std::sync::atomic::Ordering::Relaxed);
        let d0 = hits_b.load(std::sync::atomic::Ordering::Relaxed);
        mount_subscriber(&mut ctx, Vec::new(), |_, _| {
            panic!("undeclared key must not notify a reader");
        });
        ctx.set(WORKSPACE_KEY, 1u64);
        assert_eq!(hits_a.load(std::sync::atomic::Ordering::Relaxed), c0);
        assert_eq!(hits_b.load(std::sync::atomic::Ordering::Relaxed), d0);
    }
}
