//! Single descriptor authority for compiled Browse, preparation and Start.
use crate::native::CompiledCard;
use std::borrow::Cow;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock, RwLock};

/// A picker id: the exact string the picker shows and Start keys on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompiledId(pub &'static str);

/// Why an additional compiled-card slice was not installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardInstallError {
    DuplicateId(CompiledId),
}

impl std::fmt::Display for CardInstallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DuplicateId(id) => write!(f, "duplicate compiled card id: {}", id.0),
        }
    }
}

impl std::error::Error for CardInstallError {}

static CARDS: &[CompiledCard] = &[
    crate::gatherer::CARD,
    crate::grinch::CARD,
    crate::quester::CARD,
];

struct Registry {
    installed: Vec<&'static [CompiledCard]>,
    merged: &'static [CompiledCard],
    tail: Option<&'static Publication>,
}

// The static root owns every published table through this append-only chain.
// Old slice references remain valid without leaking or using raw pointers.
struct Publication {
    cards: Cow<'static, [CompiledCard]>,
    next: OnceLock<Box<Publication>>,
}

struct CardRegistry {
    registry: Mutex<Registry>,
    merged: RwLock<&'static [CompiledCard]>,
    // Nonempty append-only installs make the card count a unique generation.
    generation: AtomicU64,
    root: Publication,
}

impl CardRegistry {
    const fn new() -> Self {
        Self {
            registry: Mutex::new(Registry {
                installed: Vec::new(),
                merged: CARDS,
                tail: None,
            }),
            merged: RwLock::new(CARDS),
            generation: AtomicU64::new(CARDS.len() as u64),
            root: Publication {
                cards: Cow::Borrowed(CARDS),
                next: OnceLock::new(),
            },
        }
    }

    fn install(&'static self, cards: &'static [CompiledCard]) -> Result<(), CardInstallError> {
        if cards.is_empty() {
            return Ok(());
        }
        let mut registry = self.registry.lock().expect("compiled card registry lock");
        if registry
            .installed
            .iter()
            .any(|installed| std::ptr::eq(*installed, cards))
        {
            return Ok(());
        }
        for (index, card) in cards.iter().enumerate() {
            if registry
                .merged
                .iter()
                .chain(cards[..index].iter())
                .any(|existing| existing.id == card.id)
            {
                return Err(CardInstallError::DuplicateId(card.id));
            }
        }
        let mut merged = Vec::with_capacity(registry.merged.len() + cards.len());
        merged.extend_from_slice(registry.merged);
        merged.extend_from_slice(cards);
        let generation = merged.len() as u64;
        let previous = registry.tail.unwrap_or(&self.root);
        assert!(
            previous
                .next
                .set(Box::new(Publication {
                    cards: Cow::Owned(merged),
                    next: OnceLock::new(),
                }))
                .is_ok(),
            "compiled card publication tail is empty"
        );
        let publication: &'static Publication = previous
            .next
            .get()
            .expect("compiled card publication was installed");
        let merged = publication.cards.as_ref();
        registry.tail = Some(publication);
        registry.installed.push(cards);
        registry.merged = merged;
        *self.merged.write().expect("compiled card publication lock") = merged;
        self.generation.store(generation, Ordering::Release);
        Ok(())
    }

    fn cards(&self) -> &'static [CompiledCard] {
        *self.merged.read().expect("compiled card publication lock")
    }

    fn frame_cards(&self, snapshot: &mut CardRegistrySnapshot) -> &'static [CompiledCard] {
        let generation = self.generation.load(Ordering::Acquire);
        if snapshot.cards.len() as u64 != generation {
            snapshot.cards = self.cards();
        }
        snapshot.cards
    }
}

static REGISTRY: CardRegistry = CardRegistry::new();

/// A slot's immutable card view. Unchanged frames need one atomic load.
///
/// Each nonempty publication appends cards, so its total length is its
/// generation. The slice carries that value without a separate slot field.
pub(crate) struct CardRegistrySnapshot {
    cards: &'static [CompiledCard],
}

impl Default for CardRegistrySnapshot {
    fn default() -> Self {
        Self { cards: CARDS }
    }
}

/// Install one canonical static slice before its cards are needed.
///
/// Reinstalling the same slice does not publish or allocate. A duplicate ID
/// rejects the complete slice. Successful publications remain valid forever.
/// Empty slices have no effect.
pub fn install_cards(cards: &'static [CompiledCard]) -> Result<(), CardInstallError> {
    REGISTRY.install(cards)
}

pub(crate) fn frame_cards(snapshot: &mut CardRegistrySnapshot) -> &'static [CompiledCard] {
    REGISTRY.frame_cards(snapshot)
}

/// Whale names: real 274 scripts out of scope this campaign. Recognized so
/// the panel can explain why a name is not listed.
const WHALE_IDS: &[&str] = &[
    "GatheringBot",
    "Woodcutter",
    "AIOQuester",
    "AIOTeleport",
    "ClueSolver",
    "AutoFighter",
    "GreenDragon",
    "FireGiant",
    "MossGiant",
    "ChaosDruidKiller",
    "RockCrab",
    "HillGiant",
    "ChickenKiller",
    "CowKiller",
    "BrimhavenAgility",
    "WildyAgility",
    "DuelArena",
    "NatureCrafter",
    "RuneCrafter",
    "MuleCrafter",
    "FlaxRunner",
    "ShopRunner",
];

/// Static metadata exists independently of any account or active run.
pub fn compiled_cards() -> &'static [CompiledCard] {
    REGISTRY.cards()
}

pub fn compiled_card(id: CompiledId) -> Option<&'static CompiledCard> {
    compiled_cards().iter().find(|card| card.id == id)
}

pub fn compiled_id(name: &str) -> Option<CompiledId> {
    compiled_cards()
        .iter()
        .find(|card| card.id.0 == name)
        .map(|card| card.id)
}

/// True when `name` is a known whale script (listed in the 274 client but
/// out of scope for this host).
pub fn is_whale(name: &str) -> bool {
    WHALE_IDS.contains(&name)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDED_CARD: CompiledCard = CompiledCard {
        id: CompiledId("registry-test-card"),
        ..crate::gatherer::CARD
    };
    static ADDED: [CompiledCard; 1] = [ADDED_CARD];

    #[test]
    fn install_after_read_is_visible_and_same_slice_is_idempotent() {
        static REGISTRY: CardRegistry = CardRegistry::new();
        let before = REGISTRY.cards();
        assert_eq!(before.len(), CARDS.len());
        REGISTRY.install(&ADDED).unwrap();
        let after = REGISTRY.cards();
        assert_eq!(after.len(), before.len() + 1);
        assert_eq!(after.last().unwrap().id, ADDED_CARD.id);
        assert_eq!(before.len(), CARDS.len(), "the old view remains valid");
        REGISTRY.install(&ADDED).unwrap();
        REGISTRY.install(&[]).unwrap();
        assert!(std::ptr::eq(after, REGISTRY.cards()));
        assert_eq!(
            REGISTRY.generation.load(Ordering::Relaxed),
            after.len() as u64
        );
    }

    #[test]
    fn duplicate_id_rejects_the_complete_slice_without_publication() {
        static REGISTRY: CardRegistry = CardRegistry::new();
        static EXISTING_DUPLICATE: [CompiledCard; 2] = [ADDED_CARD, crate::gatherer::CARD];
        static INTERNAL_DUPLICATE: [CompiledCard; 2] = [ADDED_CARD, ADDED_CARD];
        let before = REGISTRY.cards();
        assert_eq!(
            REGISTRY.install(&EXISTING_DUPLICATE),
            Err(CardInstallError::DuplicateId(crate::gatherer::CARD.id))
        );
        assert_eq!(
            REGISTRY.install(&INTERNAL_DUPLICATE),
            Err(CardInstallError::DuplicateId(ADDED_CARD.id))
        );
        assert!(std::ptr::eq(before, REGISTRY.cards()));
        assert_eq!(
            REGISTRY.generation.load(Ordering::Relaxed),
            before.len() as u64
        );
        assert_eq!(
            CardInstallError::DuplicateId(crate::gatherer::CARD.id).to_string(),
            "duplicate compiled card id: Gatherer"
        );
    }

    #[test]
    fn next_frame_observes_a_new_install() {
        static REGISTRY: CardRegistry = CardRegistry::new();
        assert_eq!(
            std::mem::size_of::<CardRegistrySnapshot>(),
            std::mem::size_of::<&'static [CompiledCard]>()
        );
        let mut slot_view = CardRegistrySnapshot::default();
        let first_frame = REGISTRY.frame_cards(&mut slot_view);
        assert_eq!(first_frame.len(), CARDS.len());
        REGISTRY.install(&ADDED).unwrap();
        let next_frame = REGISTRY.frame_cards(&mut slot_view);
        assert_eq!(next_frame.last().unwrap().id, ADDED_CARD.id);
        assert_eq!(next_frame.len(), first_frame.len() + 1);
        assert!(std::ptr::eq(
            next_frame,
            REGISTRY.frame_cards(&mut slot_view)
        ));
    }

    #[test]
    fn concurrent_same_slice_install_publishes_once() {
        static REGISTRY: CardRegistry = CardRegistry::new();
        std::thread::scope(|scope| {
            let first = scope.spawn(|| REGISTRY.install(&ADDED));
            let second = scope.spawn(|| REGISTRY.install(&ADDED));
            assert_eq!(first.join().unwrap(), Ok(()));
            assert_eq!(second.join().unwrap(), Ok(()));
        });
        assert_eq!(REGISTRY.cards().len(), CARDS.len() + 1);
        assert_eq!(
            REGISTRY.generation.load(Ordering::Relaxed),
            (CARDS.len() + 1) as u64
        );
    }

    #[test]
    fn compiled_card_ids_order_and_native_kinds_are_stable() {
        use crate::native::request::NativeKind;
        #[cfg(feature = "load")]
        install_cards(&crate::sherlock::CARDS).unwrap();
        let actual: Vec<_> = compiled_cards()
            .iter()
            .enumerate()
            .map(|(index, card)| (index, card.id.0, card.native_kind))
            .collect();
        let expected = [
            (0, "Gatherer", NativeKind::Gather),
            (1, "Grinch", NativeKind::Gather),
            (2, "Quester", NativeKind::Quester),
            #[cfg(feature = "load")]
            (3, "Sherlock", NativeKind::Clue),
        ];
        println!("compiled card order and native kinds: {actual:?}");
        assert_eq!(actual, expected);
    }

    #[test]
    fn whale_gate_keeps_listed_whales_out_and_real_cards_in() {
        for whale in ["Woodcutter", "RockCrab", "NatureCrafter", "ShopRunner"] {
            assert!(is_whale(whale), "{whale} must stay whale-gated");
        }
        for listed in [
            crate::gatherer::CARD.id.0,
            crate::quester::CARD.id.0,
            "BoneBurier",
            "",
        ] {
            assert!(!is_whale(listed), "{listed} must not be whale-gated");
        }
    }
}
