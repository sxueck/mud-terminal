//! Phase 1 agent settings domain model. No API/network integrations yet.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchKind {
    #[default]
    Shell,
    Agent,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutopilotMode {
    #[default]
    StepApproval,
    Smart,
    FullAuto,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSettings {
    pub base_url: String,
    pub model_id: String,
}

impl Default for ModelSettings {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".into(),
            model_id: String::new(),
        }
    }
}

impl ModelSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !(self.base_url.starts_with("http://") || self.base_url.starts_with("https://")) {
            return Err("base_url must start with http:// or https://".into());
        }
        if self.model_id.trim().is_empty() {
            return Err("model_id must not be empty".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_kind_defaults_to_shell() {
        assert_eq!(LaunchKind::default(), LaunchKind::Shell);
    }

    #[test]
    fn autopilot_defaults_to_step_approval() {
        assert_eq!(AutopilotMode::default(), AutopilotMode::StepApproval);
    }

    #[test]
    fn model_settings_defaults_to_openai_url_and_blank_model() {
        let s = ModelSettings::default();
        assert_eq!(s.base_url, "https://api.openai.com/v1");
        assert_eq!(s.model_id, "");
    }

    #[test]
    fn validate_rejects_non_http_base_url() {
        let s = ModelSettings {
            model_id: "gpt-4o".into(),
            base_url: "ftp://x".into(),
        };
        assert_eq!(
            s.validate(),
            Err("base_url must start with http:// or https://".into())
        );
    }

    #[test]
    fn validate_rejects_blank_model_id() {
        let s = ModelSettings {
            base_url: "https://api.openai.com/v1".into(),
            model_id: " ".into(),
        };
        assert_eq!(s.validate(), Err("model_id must not be empty".into()));
    }

    #[test]
    fn validate_accepts_http_and_https() {
        let s = ModelSettings {
            base_url: "http://localhost:1234".into(),
            model_id: "m".into(),
        };
        assert_eq!(s.validate(), Ok(()));
        let s = ModelSettings {
            base_url: "https://api.openai.com/v1".into(),
            model_id: "m".into(),
        };
        assert_eq!(s.validate(), Ok(()));
    }

    #[test]
    fn settings_implement_serde_traits() {
        fn assert_serde<T: serde::Serialize + serde::de::DeserializeOwned>() {}
        assert_serde::<ModelSettings>();
        assert_serde::<LaunchKind>();
        assert_serde::<AutopilotMode>();
    }
}
