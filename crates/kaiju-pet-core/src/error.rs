use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PetError {
    #[error("cannot reach daemon")]
    Transport,
    #[error("authentication required")]
    AuthRequired,
    #[error("agent no longer exists")]
    AgentGone,
    #[error("agent is not running")]
    AgentTerminal,
    #[error("daemon error ({0})")]
    Server(u16),
    #[error("malformed daemon response")]
    Decode,
    #[error("unexpected response ({0})")]
    Unknown(u16),
}

impl PetError {
    pub fn from_status(code: u16) -> PetError {
        match code {
            401 => PetError::AuthRequired,
            404 => PetError::AgentGone,
            409 => PetError::AgentTerminal,
            500..=599 => PetError::Server(code),
            _ => PetError::Unknown(code),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_http_statuses_to_intent() {
        assert!(matches!(PetError::from_status(401), PetError::AuthRequired));
        assert!(matches!(PetError::from_status(404), PetError::AgentGone));
        assert!(matches!(
            PetError::from_status(409),
            PetError::AgentTerminal
        ));
        assert!(matches!(PetError::from_status(500), PetError::Server(500)));
        assert!(matches!(PetError::from_status(503), PetError::Server(503)));
        assert!(matches!(PetError::from_status(418), PetError::Unknown(418)));
    }
}
