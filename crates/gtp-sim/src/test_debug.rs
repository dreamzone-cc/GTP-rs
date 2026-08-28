use super::*;
use gtp_types::{Duration, OrderedGroupId, PriorityTier};

#[test]
fn test_debug_simulation_single_step() {
    let mut runner = SimulationRunner::new(12345, NetworkProfile::lan());

    let msg_id = runner
        .client
        .send_reliable_ordered(
            OrderedGroupId(1),
            b"test_msg_1".to_vec(),
            PriorityTier::P3ReliableGameplay,
            None,
            runner.current_time,
        )
        .unwrap();
    println!("Enqueued msg_id: {:?}", msg_id);

    let (c, s) = runner.run_for(Duration::from_millis(50), Duration::from_millis(1));
    println!("After 50ms: client msgs={}, server msgs={}", c.len(), s.len());
    assert_eq!(s.len(), 1);
}
