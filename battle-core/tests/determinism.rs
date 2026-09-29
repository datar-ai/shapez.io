//! The same seed must give the same battle, whatever the thread count.

use battlecore::scenario;

fn run(seed: u64, scale: u32, threads: usize, steps: u32) -> u64 {
    let mut b = scenario::demo(seed, scale);
    b.threads = threads;
    for _ in 0..steps {
        b.step();
        if b.outcome.is_some() {
            break;
        }
    }
    b.state_hash()
}

#[test]
fn same_seed_same_battle() {
    assert_eq!(run(3, 1, 1, 100_000), run(3, 1, 1, 100_000));
}

#[test]
fn different_seed_different_battle() {
    assert_ne!(run(3, 1, 1, 100_000), run(4, 1, 1, 100_000));
}

#[test]
fn thread_count_does_not_change_result() {
    // Scale 20 is about 1,360 ships, enough to split the ship phase into chunks.
    let one = run(5, 20, 1, 600);
    assert_eq!(one, run(5, 20, 2, 600));
    assert_eq!(one, run(5, 20, 4, 600));
}
