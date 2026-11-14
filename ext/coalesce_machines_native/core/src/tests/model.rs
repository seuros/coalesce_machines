//! Randomized runs against an independent naive model.

use super::*;

/// Independent naive model: growable collections, linear search, no slots.
struct Model {
    flights: Vec<(u64, u64, Vec<u64>)>,
    ready: VecDeque<Notification>,
    issued: u64,
    scope: u16,
    flight_cap: usize,
    waiter_cap: usize,
}

impl Model {
    /// The id of a sequence number, spelled out here rather than borrowed
    /// from the crate under test: scope in the top 16 bits, sequence below.
    fn id(&self, sequence: u64) -> u64 {
        (u64::from(self.scope) << 48) | sequence
    }

    fn held(&self) -> usize {
        self.flights.iter().map(|f| f.2.len()).sum::<usize>() + self.ready.len()
    }

    fn join(&mut self, key: u64) -> Result<Joined, Refusal> {
        let cap = self.waiter_cap;
        let held = self.held();
        if let Some(flight) = self.flights.iter_mut().find(|f| f.1 == key) {
            if held >= cap {
                return Err(Refusal::WaitersFull);
            }
            let id = (u64::from(self.scope) << 48) | (self.issued + 1);
            flight.2.push(id);
            self.issued += 1;
            return Ok(Joined::Follower {
                waiter: WaiterId(id),
                flight: FlightId(flight.0),
            });
        }
        if self.flights.len() >= self.flight_cap {
            return Err(Refusal::FlightsFull);
        }
        self.issued += 1;
        let id = self.id(self.issued);
        self.flights.push((id, key, Vec::new()));
        Ok(Joined::Leader(FlightId(id)))
    }

    fn settle(&mut self, id: u64, outcome: Outcome) -> Result<usize, Refusal> {
        let at = self
            .flights
            .iter()
            .position(|f| f.0 == id)
            .ok_or(Refusal::Stale)?;
        let (flight, key, waiters) = self.flights.remove(at);
        for waiter in &waiters {
            self.ready.push_back(Notification {
                waiter: WaiterId(*waiter),
                flight: FlightId(flight),
                key,
                outcome,
            });
        }
        Ok(waiters.len())
    }

    fn cancel(&mut self, id: u64) -> Result<(), Refusal> {
        for flight in &mut self.flights {
            if let Some(at) = flight.2.iter().position(|w| *w == id) {
                flight.2.remove(at);
                return Ok(());
            }
        }
        let at = self
            .ready
            .iter()
            .position(|n| n.waiter.0 == id)
            .ok_or(Refusal::Stale)?;
        self.ready.remove(at);
        Ok(())
    }
}

pub(super) use matryona_machines::testing::SplitMix64 as Draws;

fn agree<const F: usize, const W: usize>(c: &Coalescer<F, W>, model: &Model, keys: u64) {
    assert_eq!(c.in_flight(), model.flights.len());
    assert_eq!(c.pending(), model.ready.len());
    assert_eq!(c.waiting(), model.held() - model.ready.len());
    for key in 0..keys {
        let expected = model.flights.iter().find(|f| f.1 == key);
        assert_eq!(c.flight(key), expected.map(|f| FlightId(f.0)));
    }
    for id in (0..=model.issued + 1).map(|sequence| model.id(sequence)) {
        let expected = model.flights.iter().find(|f| f.0 == id);
        assert_eq!(c.followers(FlightId(id)), expected.map(|f| f.2.len()));
    }
}

fn run_against_model<const F: usize, const W: usize>(
    scope: u16,
    seed: u64,
    keys: u64,
    steps: usize,
) {
    let mut draws = Draws::new(seed);
    let mut c = Coalescer::<F, W>::with_scope(scope);
    assert_eq!(c.scope(), scope);
    let mut model = Model {
        flights: Vec::new(),
        ready: VecDeque::new(),
        issued: 0,
        scope,
        flight_cap: F,
        waiter_cap: W,
    };
    let mut refusals = 0_usize;
    let mut foreign_ids = 0_usize;
    for _ in 0..steps {
        let before = c.clone();
        // Any id ever issued, plus one that never was: live, stale and wrong-role
        // alike. One in five carries another scope, where the same sequence
        // number may well be live here.
        let sequence = draws.below(model.issued + 2);
        let foreign = draws.below(5) == 0;
        let id = if foreign {
            let other = scope.wrapping_add(1 + u16::try_from(draws.below(3)).unwrap_or(0));
            (u64::from(other) << 48) | sequence
        } else {
            model.id(sequence)
        };
        let refused = match draws.below(11) {
            10 => {
                // A reserved id comes from the same sequence and is nobody's
                // flight or waiter: the model only counts it.
                model.issued += 1;
                assert_eq!(c.reserve(), Ok(model.id(model.issued)));
                false
            }
            0..=3 => {
                let key = draws.below(keys);
                let (got, want) = (c.join(key), model.join(key));
                assert_eq!(got, want);
                got.is_err()
            }
            4 => {
                let got = c.complete(FlightId(id));
                assert_eq!(got, model.settle(id, Outcome::Completed));
                got.is_err()
            }
            5 => {
                let got = c.abandon(FlightId(id));
                assert_eq!(got, model.settle(id, Outcome::Abandoned));
                got.is_err()
            }
            6 | 7 => {
                let got = c.cancel(WaiterId(id));
                assert_eq!(got, model.cancel(id));
                got.is_err()
            }
            8 => {
                assert_eq!(c.poll(), model.ready.pop_front());
                false
            }
            9 => {
                let mut out = [Notification::default(); 3];
                let room = usize::try_from(draws.below(4)).unwrap_or(0);
                let written = c.drain(&mut out[..room]);
                assert_eq!(written, room.min(model.ready.len()));
                for (slot, entry) in out.iter().enumerate() {
                    let want = (slot < written).then(|| model.ready.pop_front()).flatten();
                    assert_eq!(*entry, want.unwrap_or_default());
                }
                false
            }
            other => panic!("draw {other} is outside the eleven operations"),
        };
        if foreign {
            foreign_ids += 1;
            // Whatever the step did, the foreign id itself reaches nothing.
            names_nothing(&before, id);
        }
        if refused {
            refusals += 1;
            assert_eq!(c, before, "a refused call mutated state");
        }
        agree(&c, &model, keys);
    }
    assert!(refusals > 0, "the run never exercised a refusal");
    assert!(foreign_ids > 0, "the run never fed a foreign id");
    assert!(model.issued > 0);
}

#[test]
fn randomized_operations_match_the_naive_model() {
    for seed in 0..24 {
        run_against_model::<3, 5>(scope_for(seed), seed, 5, 1_500);
    }
    // Tight tables: capacity refusals dominate.
    for seed in 100..112 {
        run_against_model::<1, 1>(0, seed, 3, 800);
        run_against_model::<2, 0>(513, seed, 3, 400);
    }
    // Roomy tables: long chains, many keys.
    for seed in 200..206 {
        run_against_model::<8, 32>(u16::MAX - 1, seed, 12, 1_500);
    }
}
