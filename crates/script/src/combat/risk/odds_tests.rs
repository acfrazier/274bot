use super::*;
use crate::combat::risk::{combat_roll, consts::DEATH_BOUND, defence_rolls};

// Failing-before contract for every P1-B test below: the approved P1-A head has
// no risk::odds, so these exact imports and assertions do not compile there.
// The retained before probe compiles this file against the merged facts head.
// The old worst-case wolf comparison is 7 * 6 >= 40, not a probability admission.
fn attack(hit: f64, amount: u8) -> Event {
    Event {
        tick: 0,
        kind: EventKind::Attack,
        amount,
        hit: HitChance::ceil_from(hit),
    }
}
fn sample(seed: &mut u64) -> f64 {
    // Calculator c024a4152, fight.rs: deterministic independent draws.
    *seed ^= *seed << 13;
    *seed ^= *seed >> 7;
    *seed ^= *seed << 17;
    (*seed >> 11) as f64 / ((1u64 << 53) as f64)
}

#[test]
fn inclusive_roll_hand_cases() {
    // Moved from calculator c024a4152, fight.rs. A tie misses.
    assert_eq!(hit_chance(3, 1), 5.0 / 8.0);
    assert_eq!(hit_chance(1, 3), 1.0 / 8.0);
    assert_eq!(hit_chance(2, 2), 3.0 / 9.0);
    assert_eq!(hit_chance(0, 0), 0.0);
    for a in 0..=12 {
        for d in 0..=12 {
            let successes = (0..=a)
                .map(|x| (0..=d).filter(|y| x > *y).count())
                .sum::<usize>();
            assert!(
                (hit_chance(a, d) - successes as f64 / f64::from((a + 1) * (d + 1))).abs() < 1e-14
            );
        }
    }
    assert_eq!(hit_chance(0, u32::MAX), 0.0);
    assert!(hit_chance(u32::MAX, 0) < 1.0);
}

#[test]
fn armour_bonus_reduces_incoming_hit_chance_hand_case() {
    // Calculator hand case, expressed through P1-A's checked stat conventions.
    let npc = combat_roll(9, 0).unwrap();
    let bare = defence_rolls(40, 0, [0; 4]).unwrap()[0];
    let armour = defence_rolls(40, 0, [40; 4]).unwrap()[0];
    assert_eq!((npc, bare, armour), (1152, 3072, 4992));
    assert_eq!(hit_chance(npc, armour), 1152.0 / (2.0 * 4993.0));
    assert!(hit_chance(npc, armour) < hit_chance(npc, bare));
}

#[test]
fn hit_chance_matches_inclusive_roll_monte_carlo() {
    // Calculator sampling convention: independent accuracy and damage draws.
    // The fight-clock solver tests belong to P1-E, not this no-action core.
    let mut seed = 0x0192_dab4_bafe_7135u64;
    const TRIALS: u32 = 200_000;
    for (a, d) in [(3, 1), (1, 3), (2, 2), (3776, 12096), (0, 0)] {
        let mut hits = 0;
        for _ in 0..TRIALS {
            let rolled_a = (sample(&mut seed) * (f64::from(a) + 1.0)) as u32;
            let rolled_d = (sample(&mut seed) * (f64::from(d) + 1.0)) as u32;
            hits += u32::from(rolled_a > rolled_d);
        }
        let observed = f64::from(hits) / f64::from(TRIALS);
        assert!((observed - hit_chance(a, d)).abs() < 0.006);
    }
}

#[test]
fn bound_wolf_crossing_at_base_40_is_survivable_without_food() {
    let wolf = combat_roll(50, 0).unwrap();
    let kit = defence_rolls(40, 0, [188, 202, 198, 190]).unwrap();
    let events = [attack(hit_chance(wolf, kit[0]), 6); 7];
    let bound = death_bound(&events, 40).unwrap();
    eprintln!("wolf: {bound:?}");
    assert!((bound.bound / 9.86e-11 - 1.0).abs() < 0.001);
    assert!(bound.admits(DEATH_BOUND));
}

#[test]
fn bound_is_order_independent() {
    let mut events = [
        attack(0.156, 6),
        attack(0.7, 3),
        attack(0.156, 6),
        Event {
            kind: EventKind::Poison,
            amount: 2,
            ..attack(0.0, 0)
        },
        attack(0.4, 8),
        Event {
            kind: EventKind::Certain,
            amount: 1,
            ..attack(0.0, 0)
        },
        attack(0.4, 8),
    ];
    let expected = death_bound(&events, 17).unwrap();
    let mut seed = 0x0192_dab4_bafe_7135;
    for _ in 0..1000 {
        for index in 0..events.len() {
            let other = (sample(&mut seed) * events.len() as f64) as usize;
            events.swap(index, other);
            events[index].tick = (sample(&mut seed) * 65536.0) as u16;
        }
        let actual = death_bound(&events, 17).unwrap();
        assert_eq!(actual.bound.to_bits(), expected.bound.to_bits());
        assert_eq!(actual.visits, expected.visits);
    }
}

// Integer-scaled enumeration is independent of the product convolution.
// Every stored f32 probability is an exact dyadic rational. Small fixtures
// keep the product denominator within u128, including non-dyadic source input.
fn exact_hit(event: Event) -> (u128, u128) {
    let bits = event.hit.get().to_bits();
    if bits == 0 {
        return (0, 1);
    }
    let mantissa = (bits & ((1u64 << 52) - 1)) | (1u64 << 52);
    let exponent = ((bits >> 52) & 0x7ff) as i32 - 1023 - 52;
    let common = mantissa.trailing_zeros().min((-exponent) as u32);
    (
        u128::from(mantissa >> common),
        1u128 << (-exponent - common as i32),
    )
}

fn exact_tail(events: &[Event], hp: u8) -> (u128, u128) {
    let Some((&event, rest)) = events.split_first() else {
        return (u128::from(hp == 0), 1);
    };
    if event.kind != EventKind::Attack {
        return exact_tail(rest, hp.saturating_sub(event.amount));
    }
    let (hit, scale) = exact_hit(event);
    let event_scale = scale * (u128::from(event.amount) + 1);
    let mut numerator = 0;
    let mut denominator = 1;
    for damage in 0..=event.amount {
        let weight = if damage == 0 {
            event_scale - hit * u128::from(event.amount)
        } else {
            hit
        };
        let (tail, tail_scale) = exact_tail(rest, hp.saturating_sub(damage));
        numerator += weight * tail;
        denominator = event_scale * tail_scale;
    }
    (numerator, denominator)
}

fn exhaustive_tail(events: &[Event], hp: u8) -> f64 {
    let (numerator, denominator) = exact_tail(events, hp);
    numerator as f64 / denominator as f64
}

#[test]
fn bound_dominating_kind_never_understates() {
    // Old: only pairs, checked with another rounded recursion.
    // New: every size-1..=3 mixture on the grid, checked with integer weights.
    let kinds: [Event; 20] =
        std::array::from_fn(|index| attack((index / 4) as f64 / 4.0, (index % 4) as u8));
    for len in 1..=3 {
        for mut selection in 0usize..20usize.pow(len as u32) {
            let mut mixture = [kinds[0]; 3];
            for event in mixture.iter_mut().take(len) {
                *event = kinds[selection % kinds.len()];
                selection /= kinds.len();
            }
            let mixture = &mixture[..len];
            let max_hit = mixture
                .iter()
                .map(|event| event.hit.get())
                .fold(0.0, f64::max);
            let max_amount = mixture.iter().map(|event| event.amount).max().unwrap();
            let envelope = [attack(max_hit, max_amount); 3];
            for hp in 1..=10 {
                let mixed = death_bound(mixture, hp).unwrap();
                let dominating = death_bound(&envelope[..len], hp).unwrap();
                let exact = exhaustive_tail(mixture, hp);
                assert!((mixed.bound - exact).abs() <= f64::from(mixed.visits) * f64::EPSILON);
                assert!(dominating.bound + f64::from(dominating.visits) * f64::EPSILON >= exact);
            }
        }
    }
}

#[test]
fn bound_distinct_keys_and_fixed_events_match_exact_reference() {
    // F1/F2: old tests had no exact reference for 3+ distinct keys or fixed
    // companion events. Four keys, a duplicate, and both fixed kinds now do.
    let certain = Event {
        kind: EventKind::Certain,
        ..attack(0.0, 2)
    };
    let poison = Event {
        kind: EventKind::Poison,
        ..attack(0.0, 3)
    };
    let mixture = [
        attack(0.156, 6),
        attack(0.25, 1),
        attack(0.5, 2),
        attack(0.75, 3),
        attack(0.25, 1),
        certain,
        poison,
    ];
    assert!(
        mixture[0].hit.get() > 0.156,
        "fixture exercises upward rounding"
    );
    let envelope = [attack(0.75, 6); 5];
    for hp in 1..=20 {
        let actual = death_bound(&mixture, hp).unwrap();
        let exact = exhaustive_tail(&mixture, hp);
        assert!((actual.bound - exact).abs() <= f64::from(actual.visits) * f64::EPSILON);
        let dominating = death_bound(
            &[
                envelope[0],
                envelope[1],
                envelope[2],
                envelope[3],
                envelope[4],
                certain,
                poison,
            ],
            hp,
        )
        .unwrap();
        assert!(dominating.bound + f64::from(dominating.visits) * f64::EPSILON >= exact);
    }
}

#[test]
fn forced_max_kind_is_a_certain_hit() {
    // A forced maximum is not an accuracy-1 uniform roll. Its full amount lands.
    let certain = Event {
        kind: EventKind::Certain,
        ..attack(0.0, 6)
    };
    assert_eq!(death_bound(&[certain], 6).unwrap().bound, 1.0);
    assert_eq!(death_bound(&[certain], 7).unwrap().bound, 0.0);
    assert!(death_bound(&[attack(1.0, 6)], 6).unwrap().bound < 1.0);
    let poison = Event {
        kind: EventKind::Poison,
        amount: 3,
        ..certain
    };
    assert_eq!(death_bound(&[certain, poison], 9).unwrap().bound, 1.0);
    assert_eq!(death_bound(&[certain, poison], 10).unwrap().bound, 0.0);
}

#[test]
fn hit_chance_is_rounded_up() {
    let mut seed = 0x0192_dab4_bafe_7135;
    let mut nearest_understated = false;
    for _ in 0..1_000_000 {
        let a = (sample(&mut seed) * (f64::from(u32::MAX) + 1.0)) as u32;
        let d = (sample(&mut seed) * (f64::from(u32::MAX) + 1.0)) as u32;
        let h = hit_chance(a, d);
        let stored = HitChance::ceil_from(h).get();
        nearest_understated |= f64::from(h as f32) < h;
        assert!(stored >= h && stored <= 1.0);
        if f64::from(h as f32) < h {
            assert_eq!(stored, f64::from((h as f32).next_up()));
        } else {
            assert_eq!(stored, f64::from(h as f32));
        }
    }
    assert!(
        nearest_understated,
        "nearest rounding would understate this sample"
    );
    assert_eq!(HitChance::ceil_from(0.0).get(), 0.0);
    assert_eq!(HitChance::ceil_from(1.0).get(), 1.0);
}

#[test]
fn bound_numeric_guard_is_visits_times_epsilon() {
    let bound = death_bound(&[attack(0.5, 1)], 1).unwrap();
    assert_eq!(
        bound,
        DeathBound {
            bound: 0.25,
            visits: 2
        }
    );
    let guarded = bound.bound + f64::from(bound.visits) * f64::EPSILON;
    assert!(!bound.admits(bound.bound));
    assert!(!bound.admits(guarded.next_down()));
    assert!(bound.admits(guarded));
    assert!(!bound.admits(f64::NAN));
    let largest = death_bound(&[attack(0.5, 255); MAX_EVENTS], 127).unwrap();
    assert!(largest.visits <= (MAX_EVENTS * 128 * 256) as u32);
    assert!(f64::from(largest.visits) * f64::EPSILON <= 1.5e-8);
    eprintln!("maximum-domain guard: {largest:?}");
}

#[test]
fn bound_overflow_refuses_above_max_events() {
    let event = attack(0.0, 0);
    let mut buffer = EventBuf::new();
    for _ in 0..MAX_EVENTS {
        buffer.push(event).unwrap();
    }
    assert_eq!(death_bound(buffer.events(), 127).unwrap().bound, 0.0);
    assert_eq!(buffer.push(event), Err(UnknownWhy::Overflow));
    assert_eq!(buffer.events().len(), MAX_EVENTS);
    let oversized = [event; MAX_EVENTS + 1];
    assert_eq!(death_bound(&oversized, 40), Err(UnknownWhy::Overflow));
    assert_eq!(
        escape_floor(&oversized, 40, DEATH_BOUND),
        Err(UnknownWhy::Overflow)
    );
    assert_eq!(death_bound(&[], 128), Err(UnknownWhy::Overflow));
    assert_eq!(death_bound(&[], 0), Err(UnknownWhy::MissingFacts));
    assert_eq!(
        escape_floor(&[], 128, DEATH_BOUND),
        Err(UnknownWhy::Overflow)
    );
    assert_eq!(
        escape_floor(&[], 0, DEATH_BOUND),
        Err(UnknownWhy::MissingFacts)
    );
    for invalid in [f64::NAN, f64::INFINITY, -0.1, 1.1] {
        assert_eq!(
            death_bound(&[attack(invalid, 6)], 40),
            Err(UnknownWhy::MissingFacts)
        );
    }
    buffer.clear();
    assert!(buffer.events().is_empty());
    buffer.push(event).unwrap();
    assert_eq!(buffer.events(), &[event]);
}

#[test]
fn odds_is_allocation_free() {
    let mut buffer = None;
    let creation = allocation_counter::measure(|| buffer = Some(EventBuf::new()));
    assert_eq!(creation.count_total, 1);
    assert_eq!(creation.bytes_total, MAX_EVENTS as u64 * 8);
    let mut buffer = buffer.unwrap();
    let measured = allocation_counter::measure(|| {
        for _ in 0..7 {
            buffer.push(attack(0.156, 6)).unwrap();
        }
        std::hint::black_box(hit_chance(3776, 12096));
        std::hint::black_box(death_bound(buffer.events(), 40).unwrap());
        std::hint::black_box(escape_floor(buffer.events(), 40, DEATH_BOUND).unwrap());
        buffer.clear();
    });
    assert_eq!(measured.count_total, 0);
    assert_eq!(measured.bytes_total, 0);
}

#[test]
fn escape_floor_wolf_d9_is_10_at_one_percent() {
    // Nine-tile wolf corridor: three (hit .156, maximum 6) events.
    // Old worst-case floor was 24. New safe floor is 10, not 9 or 11.
    let corridor = [attack(0.156, 6); 3];
    assert!(!death_bound(&corridor, 9).unwrap().admits(DEATH_BOUND));
    assert!(death_bound(&corridor, 10).unwrap().admits(DEATH_BOUND));
    assert_eq!(escape_floor(&corridor, 40, DEATH_BOUND), Ok(10));
    assert_eq!(escape_floor(&corridor, 9, DEATH_BOUND), Ok(10));
}

#[test]
fn escape_floor_is_zero_without_attackers() {
    assert_eq!(escape_floor(&[], 40, DEATH_BOUND), Ok(0));
    assert_eq!(escape_floor(&[attack(0.0, 255)], 40, DEATH_BOUND), Ok(0));
    assert_eq!(escape_floor(&[attack(1.0, 0)], 40, DEATH_BOUND), Ok(0));
}

#[test]
fn escape_floor_matches_independent_death_tails() {
    let events = [
        attack(0.4, 6),
        attack(0.75, 2),
        Event {
            kind: EventKind::Poison,
            ..attack(0.0, 2)
        },
    ];
    for maximum in 1..=40 {
        let first = (1..=maximum).find(|&hp| death_bound(&events, hp).unwrap().admits(DEATH_BOUND));
        let expected = match first {
            Some(1) => 0,
            Some(hp) => hp,
            None => maximum + 1,
        };
        assert_eq!(escape_floor(&events, maximum, DEATH_BOUND), Ok(expected));
    }
}

#[test]
fn bound_matches_no_action_monte_carlo() {
    let events = [
        attack(0.6, 2),
        attack(0.4, 3),
        attack(0.35, 3),
        attack(0.7, 1),
    ];
    let mut seed = 0x0192_dab4_bafe_7135;
    let mut deaths = 0;
    const TRIALS: u32 = 200_000;
    for _ in 0..TRIALS {
        let mut damage = 0;
        for event in events {
            if sample(&mut seed) < event.hit.get() {
                damage += (sample(&mut seed) * (f64::from(event.amount) + 1.0)) as u8;
            }
        }
        deaths += u32::from(damage >= 5);
    }
    let exact = death_bound(&events, 5).unwrap();
    assert!((exact.bound - f64::from(deaths) / f64::from(TRIALS)).abs() < 0.006);
}

#[test]
fn odds_layout_sizes_are_bounded() {
    assert_eq!(std::mem::size_of::<HitChance>(), 4);
    assert_eq!(std::mem::size_of::<Event>(), 8);
    assert_eq!(std::mem::size_of::<DeathBound>(), 16);
    assert_eq!(std::mem::size_of::<EventBuf>(), 16);
    assert_eq!(std::mem::size_of::<[[f64; 128]; 2]>(), 2048);
    eprintln!("P1-B: per-bot retained=0B; event payload={}B, owner={}B per worker; solve rows=2048B stack; DeathBound=16B", MAX_EVENTS * 8, std::mem::size_of::<EventBuf>());
}
