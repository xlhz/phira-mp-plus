use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ExceedAction {
    Warn,
    #[default]
    Kick,
    Ban,
}

impl ExceedAction {
    pub fn label(self) -> &'static str {
        match self {
            ExceedAction::Warn => "仅告警",
            ExceedAction::Kick => "断开连接",
            ExceedAction::Ban => "断开并封禁",
        }
    }

    pub fn next(self) -> Self {
        match self {
            ExceedAction::Warn => ExceedAction::Kick,
            ExceedAction::Kick => ExceedAction::Ban,
            ExceedAction::Ban => ExceedAction::Warn,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrafficConfig {
    pub enabled: bool,
    pub limit_bps: u64,
    pub action: ExceedAction,
}

impl Default for TrafficConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            limit_bps: 512 * 1024,
            action: ExceedAction::Kick,
        }
    }
}

impl TrafficConfig {
    pub fn load() -> Self {
        std::fs::read_to_string("traffic.yml")
            .ok()
            .and_then(|s| serde_yaml::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let yaml = serde_yaml::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write("traffic.yml", yaml)
    }
}

pub struct TrafficLimiter {
    config: Arc<RwLock<TrafficConfig>>,
}

impl TrafficLimiter {
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(TrafficConfig::load())),
        }
    }

    pub async fn get_config(&self) -> TrafficConfig {
        self.config.read().await.clone()
    }

    pub async fn set_enabled(&self, enabled: bool) {
        let mut config = self.config.write().await;
        config.enabled = enabled;
        let _ = config.save();
    }

    pub async fn set_limit_bps(&self, limit_bps: u64) {
        let mut config = self.config.write().await;
        config.limit_bps = limit_bps;
        let _ = config.save();
    }

    pub async fn set_action(&self, action: ExceedAction) {
        let mut config = self.config.write().await;
        config.action = action;
        let _ = config.save();
    }
}

impl Default for TrafficLimiter {
    fn default() -> Self {
        Self::new()
    }
}
