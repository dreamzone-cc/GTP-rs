# GTP-ARCH-01: Protocol Architecture & Design Principles

**Status:** Normative Sub-Specification  
**Version:** 1.1  
**Target Language:** Rust  

---

## 1. Overview
Game Transport Protocol v1.1 (GTP/1.1) is a specialized transport layer built on top of UDP for competitive, real-time multiplayer games.

## 2. Core Architectural Principles
1. **Semantics-First Transport:** Unlike generic stream protocols (TCP, generic QUIC) or naive unreliable UDP, GTP understands game state lifecycle: validity, freshness, deadlines, and supersession.
2. **Four Message Semantics:**
   - `UNRELIABLE`: Low-latency, drop-on-loss data.
   - `UNRELIABLE_SEQUENCED`: Freshness-guaranteed updates per state key.
   - `RELIABLE_UNORDERED`: Guaranteed delivery without head-of-line blocking.
   - `RELIABLE_ORDERED`: Guaranteed, strictly ordered delivery within scoped groups.
3. **Strict Separation of Identifiers:** Complete separation between Packet Numbers, Message IDs, Fragment IDs, Transmission IDs, State Sequences, and Generation IDs.
4. **Unified Congestion State:** One connection owns one active path, one congestion controller (CUBIC baseline), and one high-resolution pacing engine.
5. **No Starvation & Bounded Buffers:** Multi-tier priority scheduler (P0-P4) with reserved control bandwidth and effective queue calculation.
