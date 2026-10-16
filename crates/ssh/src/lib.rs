//! Phase 1 SSH host config domain model. No networking or credentials.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub name: String,
    pub address: String,
    pub user: String,
    pub port: u16,
}

impl Host {
    /// Validates required, non-blank fields and a nonzero port.
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("name", &self.name),
            ("address", &self.address),
            ("user", &self.user),
        ] {
            if value.trim().is_empty() {
                return Err(format!("{field} must not be blank"));
            }
        }
        if self.port == 0 {
            return Err("port must be nonzero".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host {
            name: "box".into(),
            address: "10.0.0.1".into(),
            user: "u".into(),
            port: 22,
        }
    }

    #[test]
    fn valid_host_passes() {
        assert_eq!(host().validate(), Ok(()));
    }

    #[test]
    fn rejects_blank_name_address_user() {
        let mut h = host();
        h.name = "   ".into();
        assert!(h.validate().is_err());
        let mut h = host();
        h.address = String::new();
        assert!(h.validate().is_err());
        let mut h = host();
        h.user = " ".into();
        assert!(h.validate().is_err());
    }

    #[test]
    fn rejects_zero_port() {
        let mut h = host();
        h.port = 0;
        assert_eq!(h.validate(), Err("port must be nonzero".into()));
    }

    #[test]
    fn host_implements_serde_traits() {
        fn assert_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}
        assert_serde::<Host>();
    }
}
