use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpAccessConfig {
    pub blacklist: HashSet<String>,
    pub whitelist: HashSet<String>,
    pub whitelist_mode: bool,
    pub enabled: bool,
}

impl Default for IpAccessConfig {
    fn default() -> Self {
        Self {
            blacklist: HashSet::new(),
            whitelist: HashSet::new(),
            whitelist_mode: false,
            enabled: true,
        }
    }
}

impl IpAccessConfig {
    pub fn try_load() -> Option<Self> {
        std::fs::read_to_string("ip_access.yml")
            .ok()
            .and_then(|s| serde_yaml::from_str(&s).ok())
    }

    pub fn load() -> Self {
        Self::try_load().unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let yaml = serde_yaml::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write("ip_access.yml", yaml)
    }
}

pub struct IpAccess {
    config: Arc<RwLock<IpAccessConfig>>,
}

impl IpAccess {
    pub fn new() -> Self {
        Self {
            config: Arc::new(RwLock::new(IpAccessConfig::load())),
        }
    }

    pub async fn check_ip(&self, ip: &IpAddr) -> bool {
        let ip_str = ip.to_string();
        let config = self.config.read().await;

        if !config.enabled {
            return true;
        }

        if config.whitelist_mode {
            config.whitelist.iter().any(|p| matches_pattern(p, &ip_str))
        } else {
            !config.blacklist.iter().any(|p| matches_pattern(p, &ip_str))
        }
    }

    pub async fn add_to_blacklist(&self, ip: String) {
        let mut config = self.config.write().await;
        config.blacklist.insert(ip.clone());
        config.whitelist.remove(&ip);
        let _ = config.save();
    }

    pub async fn remove_from_blacklist(&self, ip: &str) {
        let mut config = self.config.write().await;
        config.blacklist.remove(ip);
        let _ = config.save();
    }

    pub async fn add_to_whitelist(&self, ip: String) {
        let mut config = self.config.write().await;
        config.whitelist.insert(ip.clone());
        config.blacklist.remove(&ip);
        let _ = config.save();
    }

    pub async fn remove_from_whitelist(&self, ip: &str) {
        let mut config = self.config.write().await;
        config.whitelist.remove(ip);
        let _ = config.save();
    }

    pub async fn set_whitelist_mode(&self, enabled: bool) {
        let mut config = self.config.write().await;
        config.whitelist_mode = enabled;
        let _ = config.save();
    }

    pub async fn set_enabled(&self, enabled: bool) {
        let mut config = self.config.write().await;
        config.enabled = enabled;
        let _ = config.save();
    }

    pub async fn get_config(&self) -> IpAccessConfig {
        self.config.read().await.clone()
    }

    pub async fn reload(&self) -> bool {
        match IpAccessConfig::try_load() {
            Some(cfg) => {
                *self.config.write().await = cfg;
                true
            }
            None => false,
        }
    }
}

impl Default for IpAccess {
    fn default() -> Self {
        Self::new()
    }
}

pub fn matches_pattern(pattern: &str, ip: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    if pattern == "*" {
        return true;
    }
    if let Some((net, prefix)) = pattern.split_once('/') {
        if let (Ok(net), Ok(prefix)) = (net.trim().parse::<IpAddr>(), prefix.trim().parse::<u8>())
            && let Ok(addr) = ip.parse::<IpAddr>()
        {
            return cidr_contains(net, prefix, addr);
        }
        return false;
    }
    if pattern.chars().any(|c| matches!(c, '*' | '?' | '[')) {
        return glob_match(pattern, ip);
    }
    pattern.eq_ignore_ascii_case(ip)
}

fn cidr_contains(net: IpAddr, prefix: u8, ip: IpAddr) -> bool {
    match (net, ip) {
        (IpAddr::V4(n), IpAddr::V4(i)) => {
            if prefix > 32 {
                return false;
            }
            let mask = if prefix == 0 {
                0u32
            } else {
                u32::MAX << (32 - prefix)
            };
            (u32::from(n) & mask) == (u32::from(i) & mask)
        }
        (IpAddr::V6(n), IpAddr::V6(i)) => {
            if prefix > 128 {
                return false;
            }
            let mask = if prefix == 0 {
                0u128
            } else {
                u128::MAX << (128 - prefix)
            };
            (u128::from(n) & mask) == (u128::from(i) & mask)
        }
        _ => false,
    }
}

fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<u8> = pattern.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let txt: Vec<u8> = text.bytes().map(|b| b.to_ascii_lowercase()).collect();
    let (mut p, mut t) = (0usize, 0usize);
    let mut star_p: Option<usize> = None;
    let mut star_t = 0usize;

    while t < txt.len() {
        if p < pat.len() {
            match pat[p] {
                b'*' => {
                    star_p = Some(p);
                    star_t = t;
                    p += 1;
                    continue;
                }
                b'?' => {
                    p += 1;
                    t += 1;
                    continue;
                }
                b'[' => {
                    if let Some((hit, next)) = match_class(&pat, p, txt[t]) {
                        if hit {
                            p = next;
                            t += 1;
                            continue;
                        }
                    } else if txt[t] == b'[' {
                        p += 1;
                        t += 1;
                        continue;
                    }
                }
                c => {
                    if c == txt[t] {
                        p += 1;
                        t += 1;
                        continue;
                    }
                }
            }
        }
        if let Some(sp) = star_p {
            star_t += 1;
            t = star_t;
            p = sp + 1;
        } else {
            return false;
        }
    }
    while p < pat.len() && pat[p] == b'*' {
        p += 1;
    }
    p == pat.len()
}

fn match_class(pat: &[u8], start: usize, ch: u8) -> Option<(bool, usize)> {
    let mut i = start + 1;
    let negate = if i < pat.len() && (pat[i] == b'!' || pat[i] == b'^') {
        i += 1;
        true
    } else {
        false
    };
    let mut matched = false;
    let mut first = true;
    while i < pat.len() {
        if pat[i] == b']' && !first {
            return Some((matched != negate, i + 1));
        }
        first = false;
        if i + 2 < pat.len() && pat[i + 1] == b'-' && pat[i + 2] != b']' {
            if pat[i] <= ch && ch <= pat[i + 2] {
                matched = true;
            }
            i += 3;
        } else {
            if pat[i] == ch {
                matched = true;
            }
            i += 1;
        }
    }
    None
}
