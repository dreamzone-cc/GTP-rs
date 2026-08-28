# GTP-IO-01: I/O Backend Abstraction & Batching

**Status:** Normative Sub-Specification  
**Version:** 1.1  

---

## 1. PacketIo Trait
```rust
pub trait PacketIo {
    fn send_batch(&mut self, packets: &[OutgoingPacket]) -> Result<usize, std::io::Error>;
    fn recv_batch(&mut self, buffers: &mut [IncomingBuffer]) -> Result<usize, std::io::Error>;
    fn local_addr(&self) -> std::net::SocketAddr;
}
```

## 2. Implementations
1. **Portable UDP (`socket2`):** Standard cross-platform socket with non-blocking I/O.
2. **Batched Syscalls:** Support for `sendmmsg` and `recvmmsg` on Linux when available.
3. **Async Runtime Adapter (`tokio`):** Bridges asynchronous event loops with GTP's synchronous tick-based core.
