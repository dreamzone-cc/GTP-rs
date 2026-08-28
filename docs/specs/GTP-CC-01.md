# GTP-CC-01: Congestion Control, Pacing & ECN

**Status:** Normative Sub-Specification  
**Version:** 1.1  

---

## 1. Congestion Controller Trait
The protocol core interacts with congestion control via a generic interface:
```rust
pub trait CongestionController {
    fn on_packet_sent(&mut self, packet_number: u64, bytes: usize, send_time: MonotonicTime);
    fn on_ack(&mut self, ack_event: &AckEvent);
    fn on_loss(&mut self, loss_event: &LossEvent);
    fn on_ecn(&mut self, ecn_ce_count: u64);
    fn on_rtt(&mut self, rtt_sample: Duration);
    fn on_timeout(&mut self);
    fn cwnd(&self) -> u64;
    fn pacing_rate(&self) -> u64; // bytes per second
    fn inflight(&self) -> u64;
}
```

## 2. CUBIC Baseline Algorithm
- **Initial Window:** $10 \times \text{SMSS}$ (typically 12,000 to 14,720 bytes).
- **Window Growth:**
  $$W_{\text{cubic}}(t) = C (t - K)^3 + W_{\text{max}}$$
  where $C = 0.4$, $\beta = 0.7$, and $K = \sqrt[3]{\frac{W_{\text{max}} (1 - \beta)}{C}}$.
- **Fast Recovery:** On packet loss or ECN CE mark, set $W_{\text{max}} = \text{cwnd}$, $\text{ssthresh} = \max(\text{cwnd} \times \beta, 2 \times \text{SMSS})$, $\text{cwnd} = \text{ssthresh}$.

## 3. High-Precision Pacing Engine
- Mandatory pacing to eliminate micro-bursts and bufferbloat.
- **Inter-packet send interval:**
  $$\Delta t = \frac{\text{packet\_size}}{\text{pacing\_rate}}$$
- Send budget calculation:
  $$\text{send\_budget} = \min(\text{cwnd} - \text{inflight}, \text{pacing\_tokens})$$
- Burst allowance cap: $\text{burst\_cap} = \min(10 \times \text{SMSS}, \text{pacing\_rate} \times 1\text{ms})$.
