# GTP-TEST-01: Verification, Simulation & Benchmarking Plan

**Status:** Normative Sub-Specification  
**Version:** 1.1  

---

## 1. Deterministic Simulation (`gtp-sim`)
Virtual test harness providing programmatic network impairments:
- Packet Loss: 0% to 50%, burst loss models (Gilbert-Elliott).
- Latency & Jitter: Configurable RTT (5ms - 300ms) with Gaussian jitter.
- Packet Reordering: 0% to 20% reorder probability.
- Duplication & Corruptions.

## 2. Test Suites
1. **Unit Tests:** Codec roundtrips, RTT calculation, CUBIC window curves, priority queues.
2. **Integration Tests:** Handshake sequence, NAT rebinding challenge/response, ordered delivery.
3. **Simulation Tests:** Mixed traffic profile under 5% loss and 50ms RTT verifying that reliable messages deliver exactly once, while stale unreliables are dropped.
