# GTP-SCHED-01: Game-Aware Scheduling, Freshness & Deadlines

**Status:** Normative Sub-Specification  
**Version:** 1.1  

---

## 1. Freshness & Deadline Semantics
1. **Deadline Calculation:**
   $$\text{remaining\_lifetime} = \text{deadline} - \text{now}$$
   If $\text{now} \ge \text{deadline}$, message is dropped immediately without sending.
2. **Predictive Admission:**
   If $\text{now} + \frac{\text{RTT}}{2} + \text{queue\_delay} > \text{deadline}$, reject or drop early if semantics permit.

## 2. Generation-Aware Supersession & Coalescing
1. **State Key Definition:** $\text{StateKey} = (\text{entity\_id: u32}, \text{state\_type: u16})$.
2. **Supersession Rule:** When a message with higher `GenerationId` or higher `StateSequence` is enqueued for a given `StateKey`, all older pending messages for that `StateKey` are removed.
3. **Coalescing:** Multiple pending updates for the same state key are coalesced into the newest payload.

## 3. Multi-Tier Priority Scheduler
- **P0 (Control):** ACKs, Handshake, Path Challenge/Response, PING, CLOSE (Reserved Bandwidth: 15%).
- **P1 (Player Input):** Critical client movement/actions (Weight: 35%).
- **P2 (World State):** Transform updates, entity snapshots (Weight: 30%).
- **P3 (Reliable Gameplay):** Damage events, inventory items (Weight: 15%).
- **P4 (Bulk / Cosmetic):** Effects, chat, non-essential data (Weight: 5%).

Deficit Round Robin (DRR) ensures weighted service without starvation while strictly obeying `send_budget`.
