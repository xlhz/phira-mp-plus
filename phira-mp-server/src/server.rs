use crate::{
    ExceedAction, IdMap, IpAccess, Maintenance, Room, SafeMap, Session, TrafficLimiter, User,
    vacant_entry,
};
use anyhow::Result;
use phira_mp_common::RoomId;
use serde::Deserialize;
use std::{collections::HashMap, fs::File, sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tracing::{info, warn};
use uuid::Uuid;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize)]
pub struct Chart {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct ServerConfig {
    pub monitors: Vec<i32>,
}
impl Default for ServerConfig {
    fn default() -> Self {
        Self { monitors: vec![2] }
    }
}

#[derive(Debug, Deserialize)]
pub struct Record {
    pub id: i32,
    pub player: i32,
    pub score: i32,
    pub perfect: i32,
    pub good: i32,
    pub bad: i32,
    pub miss: i32,
    pub max_combo: i32,
    pub accuracy: f32,
    pub full_combo: bool,
    pub std: f32,
    pub std_score: f32,
}

pub struct ServerState {
    pub config: ServerConfig,
    pub sessions: IdMap<Arc<Session>>,
    pub users: SafeMap<i32, Arc<User>>,

    pub rooms: SafeMap<RoomId, Arc<Room>>,

    pub lost_con_tx: mpsc::Sender<Uuid>,
    pub ip_access: Arc<IpAccess>,
    pub traffic: Arc<TrafficLimiter>,
    pub maintenance: Arc<Maintenance>,

    pub listen_addrs: Vec<std::net::SocketAddr>,
}

pub struct Server {
    state: Arc<ServerState>,
    accept_handles: Vec<JoinHandle<()>>,
    lost_con_handle: JoinHandle<()>,
    traffic_handle: JoinHandle<()>,
}

impl Server {
    pub async fn bind_all(addrs: &[std::net::SocketAddr]) -> Result<Self> {
        let mut listeners = Vec::new();
        for addr in addrs {
            let listener = TcpListener::bind(addr).await?;
            listeners.push(listener);
        }

        let (lost_con_tx, mut lost_con_rx) = mpsc::channel(16);
        let ip_access = Arc::new(IpAccess::new());
        let traffic = Arc::new(TrafficLimiter::new());
        let maintenance = Arc::new(Maintenance::new());

        let config: ServerConfig = File::open("server_config.yml")
            .ok()
            .and_then(|f| serde_yaml::from_reader(f).ok())
            .unwrap_or_default();

        let state = Arc::new(ServerState {
            config,
            sessions: IdMap::default(),
            users: SafeMap::default(),

            rooms: SafeMap::default(),

            lost_con_tx,
            ip_access,
            traffic: Arc::clone(&traffic),
            maintenance,

            listen_addrs: addrs.to_vec(),
        });

        let lost_con_handle = tokio::spawn({
            let state = Arc::clone(&state);
            async move {
                while let Some(id) = lost_con_rx.recv().await {
                    warn!("lost connection with {id}");
                    if let Some(session) = state.sessions.write().await.remove(&id)
                        && session
                            .user
                            .session
                            .read()
                            .await
                            .as_ref()
                            .is_some_and(|it| it.ptr_eq(&Arc::downgrade(&session)))
                    {
                        Arc::clone(&session.user).dangle().await;
                    }
                }
            }
        });

        let traffic_handle = tokio::spawn({
            let state = Arc::clone(&state);
            async move {
                let mut last: HashMap<Uuid, (u64, u64)> = HashMap::new();
                loop {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    let cfg = state.traffic.get_config().await;
                    if !cfg.enabled || cfg.limit_bps == 0 {
                        last.clear();
                        continue;
                    }
                    let sessions = state.sessions.read().await;
                    let mut current: HashMap<Uuid, (u64, u64)> = HashMap::new();
                    let mut per_ip: HashMap<String, u64> = HashMap::new();
                    for s in sessions.values() {
                        let (recv, sent) = (s.recv_bytes(), s.sent_bytes());
                        current.insert(s.id, (recv, sent));
                        let Some(ip) = s.ip else { continue };
                        let (lr, ls) = last.get(&s.id).copied().unwrap_or((recv, sent));
                        let delta = recv.saturating_sub(lr) + sent.saturating_sub(ls);
                        if delta > 0 {
                            *per_ip.entry(ip.to_string()).or_default() += delta;
                        }
                    }
                    drop(sessions);
                    last = current;

                    for (ip, bytes) in per_ip {
                        if bytes <= cfg.limit_bps {
                            continue;
                        }
                        warn!(
                            "traffic limit exceeded: {} = {} bytes/s > {} bytes/s ({})",
                            ip,
                            bytes,
                            cfg.limit_bps,
                            cfg.action.label()
                        );
                        match cfg.action {
                            ExceedAction::Warn => {}
                            ExceedAction::Kick | ExceedAction::Ban => {
                                if cfg.action == ExceedAction::Ban {
                                    state.ip_access.add_to_blacklist(ip.clone()).await;
                                }
                                let ids: Vec<Uuid> = {
                                    let sessions = state.sessions.read().await;
                                    sessions
                                        .values()
                                        .filter(|s| {
                                            s.ip.map(|a| a.to_string()).as_deref()
                                                == Some(ip.as_str())
                                        })
                                        .map(|s| s.id)
                                        .collect()
                                };
                                for id in ids {
                                    let _ = state.lost_con_tx.send(id).await;
                                }
                            }
                        }
                    }
                }
            }
        });

        let mut accept_handles = Vec::new();
        for listener in listeners {
            let state = Arc::clone(&state);
            let handle = tokio::spawn(async move {
                loop {
                    match listener.accept().await {
                        Ok((stream, _addr)) => {
                            let ip = stream.peer_addr().map(|a| a.ip()).ok();
                            let state = Arc::clone(&state);

                            tokio::spawn(async move {
                                if state.maintenance.is_closed() {
                                    warn!(
                                        "server temporarily closed, refusing connection from {}",
                                        ip.as_ref()
                                            .map(|a| a.to_string())
                                            .unwrap_or_else(|| "unknown".to_string())
                                    );
                                    return;
                                }
                                if let Some(ref ip) = ip
                                    && !state.ip_access.check_ip(ip).await
                                {
                                    warn!("connection from {} blocked by IP access control", ip);
                                    return;
                                }

                                // 仅在分配会话 id 时短暂持锁;握手与认证期间不持有 sessions 锁,
                                // 否则单个不发数据的连接即可永久阻塞所有会话操作
                                let id = {
                                    let mut guard = state.sessions.write().await;
                                    *vacant_entry(&mut guard).key()
                                };

                                let session = match tokio::time::timeout(
                                    HANDSHAKE_TIMEOUT,
                                    Session::new(id, stream, ip, Arc::clone(&state)),
                                )
                                .await
                                {
                                    Ok(Ok(session)) => session,
                                    Ok(Err(e)) => {
                                        warn!("failed to create session: {e}");
                                        return;
                                    }
                                    Err(_) => {
                                        warn!("handshake timed out after {HANDSHAKE_TIMEOUT:?}");
                                        return;
                                    }
                                };

                                info!(
                                    "received connections from {} ({}), version: {}",
                                    ip.as_ref()
                                        .map(|a| a.to_string())
                                        .unwrap_or_else(|| "unknown".to_string()),
                                    session.id,
                                    session.version()
                                );
                                state.sessions.write().await.insert(id, session);
                            });
                        }
                        Err(e) => {
                            warn!("accept error: {}", e);
                        }
                    }
                }
            });
            accept_handles.push(handle);
        }

        Ok(Self {
            state,
            accept_handles,
            lost_con_handle,
            traffic_handle,
        })
    }

    pub fn state(&self) -> Arc<ServerState> {
        Arc::clone(&self.state)
    }

    pub async fn accept(&self) -> Result<()> {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        Ok(())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.lost_con_handle.abort();
        self.traffic_handle.abort();
        for handle in &self.accept_handles {
            handle.abort();
        }
    }
}
