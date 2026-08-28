use gtp_types::{Result, TransportError};

/// High-level protocol connection lifecycle states.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum ConnectionState {
    /// Initial unestablished state.
    Initial,
    /// Client has sent HANDSHAKE_INIT and awaits response.
    Handshaking,
    /// Stateless token verified / client reachability proven.
    Validated,
    /// Full authenticated session active.
    Established,
    /// Graceful closing draining state.
    Draining,
    /// Terminal closed state.
    Closed,
}

impl ConnectionState {
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Established)
    }

    pub fn is_closed(&self) -> bool {
        matches!(self, Self::Closed)
    }

    pub fn transition_to(&mut self, next: ConnectionState) -> Result<()> {
        match (*self, next) {
            (Self::Initial, Self::Handshaking)
            | (Self::Initial, Self::Validated)
            | (Self::Handshaking, Self::Validated)
            | (Self::Validated, Self::Established)
            | (Self::Established, Self::Draining)
            | (Self::Established, Self::Closed)
            | (Self::Draining, Self::Closed)
            | (Self::Handshaking, Self::Closed)
            | (Self::Validated, Self::Closed) => {
                *self = next;
                Ok(())
            }
            (_current, _invalid) => Err(TransportError::ProtocolViolation("Illegal state transition")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_and_invalid_state_transitions() {
        let mut state = ConnectionState::Initial;
        assert!(state.transition_to(ConnectionState::Handshaking).is_ok());
        assert!(state.transition_to(ConnectionState::Validated).is_ok());
        assert!(state.transition_to(ConnectionState::Established).is_ok());
        assert!(state.is_active());
        assert!(state.transition_to(ConnectionState::Draining).is_ok());
        assert!(state.transition_to(ConnectionState::Closed).is_ok());
        assert!(state.is_closed());

        // Invalid jump from Closed to Established
        let mut closed_state = ConnectionState::Closed;
        assert!(closed_state.transition_to(ConnectionState::Established).is_err());
    }
}
