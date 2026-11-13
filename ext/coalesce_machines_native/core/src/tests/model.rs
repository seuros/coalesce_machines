//! Randomized runs against an independent naive model.

use super::*;

/// Independent naive model: growable collections, linear search, no slots.
struct Model {
    flights: Vec<(u64, u64, Vec<u64>)>,
    ready: VecDeque<Notification>,
    issued: u64,
    flight_cap: usize,
    waiter_cap: usize,
}

impl Model {
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
            self.issued += 1;
            flight.2.push(self.issued);
            return Ok(Joined::Follower {
                waiter: WaiterId(self.issued),
                flight: FlightId(flight.0),
            });
        }
        if self.flights.len() >= self.flight_cap {
            return Err(Refusal::FlightsFull);
        }
        self.issued += 1;
        self.flights.push((self.issued, key, Vec::new()));
        Ok(Joined::Leader(FlightId(self.issued)))
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

struct Draws(u64);

impl Draws {
    fn below(&mut self, bound: u64) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % bound
    }
}

fn agree<const F: usize, const W: usize>(c: &Coalescer<F, W>, model: &Model, keys: u64) {
    assert_eq!(c.in_flight(), model.flights.len());
    assert_eq!(c.pending(), model.ready.len());
    assert_eq!(c.waiting(), model.held() - model.ready.len());
    for key in 0..keys {
        let expected = model.flights.iter().find(|f| f.1 == key);
        assert_eq!(c.flight(key), expected.map(|f| FlightId(f.0)));
    }
    for id in 0..=model.issued + 1 {
        let expected = model.flights.iter().find(|f| f.0 == id);
        assert_eq!(c.followers(FlightId(id)), expected.map(|f| f.2.len()));
    }
}

fn run_against_model<const F: usize, const W: usize>(seed: u64, keys: u64, steps: usize) {
    let mut draws = Draws(seed);
    let mut c = Coalescer::<F, W>::new();
    let mut model = Model {
        flights: Vec::new(),
        ready: VecDeque::new(),
        issued: 0,
        flight_cap: F,
        waiter_cap: W,
    };
    let mut refusals = 0_usize;
    for _ in 0..steps {
        let before = c.clone();
        // Any id ever issued, plus one that never was: live, stale and wrong-role alike.
        let id = draws.below(model.issued + 2);
        let refused = match draws.below(10) {
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
            _ => {
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
        };
        if refused {
            refusals += 1;
            assert_eq!(c, before, "a refused call mutated state");
        }
        agree(&c, &model, keys);
    }
    assert!(refusals > 0, "the run never exercised a refusal");
    assert!(model.issued > 0);
}

#[test]
fn randomized_operations_match_the_naive_model() {
    for seed in 0..24 {
        run_against_model::<3, 5>(seed, 5, 1_500);
    }
    // Tight tables: capacity refusals dominate.
    for seed in 100..112 {
        run_against_model::<1, 1>(seed, 3, 800);
        run_against_model::<2, 0>(seed, 3, 400);
    }
    // Roomy tables: long chains, many keys.
    for seed in 200..206 {
        run_against_model::<8, 32>(seed, 12, 1_500);
    }
}
