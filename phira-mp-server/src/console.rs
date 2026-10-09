use crate::{ExceedAction, ServerState};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    Frame, Terminal,
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    symbols,
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Row, Table, Tabs, Wrap,
    },
};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const AUTO_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuItem {
    Dashboard,
    Sessions,
    Rooms,
    Network,
    IpAccess,
    Traffic,
    Maintenance,
    Logs,
}

impl MenuItem {
    fn title(self) -> &'static str {
        match self {
            MenuItem::Dashboard => "仪表盘",
            MenuItem::Sessions => "会话",
            MenuItem::Rooms => "房间",
            MenuItem::Network => "网络",
            MenuItem::IpAccess => "IP访问控制",
            MenuItem::Traffic => "流量限制",
            MenuItem::Maintenance => "服务器",
            MenuItem::Logs => "日志",
        }
    }

    fn all() -> &'static [MenuItem] {
        &[
            MenuItem::Dashboard,
            MenuItem::Sessions,
            MenuItem::Rooms,
            MenuItem::Network,
            MenuItem::IpAccess,
            MenuItem::Traffic,
            MenuItem::Maintenance,
            MenuItem::Logs,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IpAccessTab {
    Blacklist,
    Whitelist,
    Settings,
}

struct App {
    state: Arc<ServerState>,
    log_buffer: Arc<Mutex<Vec<String>>>,
    active_menu: MenuItem,
    menu_state: ListState,
    ip_tab: IpAccessTab,
    ip_input: String,
    ip_input_mode: bool,
    logs: Vec<String>,
    log_scroll: u16,
    popup_msg: Option<String>,
    popup_tick: u8,
    exit: bool,

    cached_session_count: usize,
    cached_user_count: usize,
    cached_room_count: usize,
    cached_sessions: Vec<(String, String, String, Option<String>)>,
    cached_rooms: Vec<(String, String, String)>,
    cached_ip_whitelist_mode: bool,
    cached_ip_enabled: bool,
    cached_blacklist: Vec<String>,
    cached_whitelist: Vec<String>,
    cached_connected_ips: Vec<String>,
    ip_list_state: ListState,
    ip_entry_state: ListState,
    ip_edit_old: Option<String>,
    auto_refresh: bool,
    last_reload: Instant,

    cached_traffic_enabled: bool,
    cached_traffic_limit: u64,
    cached_traffic_action: ExceedAction,
    traffic_sel: u8,
    traffic_input: String,
    traffic_input_mode: bool,

    cached_closed: bool,

    net_port: u16,
    net_listen_addrs: Vec<String>,
    net_local_ips: Vec<String>,
}

impl App {
    fn new(state: Arc<ServerState>, log_buffer: Arc<Mutex<Vec<String>>>) -> Self {
        let mut menu_state = ListState::default();
        menu_state.select(Some(0));
        let mut ip_list_state = ListState::default();
        ip_list_state.select(Some(0));
        let mut ip_entry_state = ListState::default();
        ip_entry_state.select(Some(0));
        let net_port = state.listen_addrs.first().map(|a| a.port()).unwrap_or(0);
        let net_listen_addrs = state.listen_addrs.iter().map(|a| a.to_string()).collect();
        let net_local_ips = local_ips();
        Self {
            state,
            log_buffer,
            active_menu: MenuItem::Dashboard,
            menu_state,
            ip_tab: IpAccessTab::Blacklist,
            ip_input: String::new(),
            ip_input_mode: false,
            logs: Vec::new(),
            log_scroll: 0,
            popup_msg: None,
            popup_tick: 0,
            exit: false,
            cached_session_count: 0,
            cached_user_count: 0,
            cached_room_count: 0,
            cached_sessions: Vec::new(),
            cached_rooms: Vec::new(),
            cached_ip_whitelist_mode: false,
            cached_ip_enabled: true,
            cached_blacklist: Vec::new(),
            cached_whitelist: Vec::new(),
            cached_connected_ips: Vec::new(),
            ip_list_state,
            ip_entry_state,
            ip_edit_old: None,
            auto_refresh: true,
            last_reload: Instant::now(),

            cached_traffic_enabled: false,
            cached_traffic_limit: 0,
            cached_traffic_action: ExceedAction::default(),
            traffic_sel: 0,
            traffic_input: String::new(),
            traffic_input_mode: false,

            cached_closed: false,

            net_port,
            net_listen_addrs,
            net_local_ips,
        }
    }

    fn show_popup(&mut self, msg: String) {
        self.popup_msg = Some(msg);
        self.popup_tick = 3;
    }

    async fn on_tick(&mut self) {
        if self.popup_tick > 0 {
            self.popup_tick -= 1;
            if self.popup_tick == 0 {
                self.popup_msg = None;
            }
        }

        if let Ok(mut buffer) = self.log_buffer.lock()
            && !buffer.is_empty()
        {
            self.logs.extend(buffer.drain(..));
            if self.logs.len() > 1000 {
                let excess = self.logs.len() - 1000;
                self.logs.drain(..excess);
            }
        }

        let sessions = self.state.sessions.read().await;
        self.cached_session_count = sessions.len();
        self.cached_user_count = sessions.len();
        let mut session_list = Vec::new();
        let mut connected_ips = Vec::new();
        for s in sessions.values() {
            let ip_str = s.ip.map(|a| a.to_string());
            session_list.push((
                s.id.to_string(),
                s.name().to_string(),
                s.version().to_string(),
                ip_str.clone(),
            ));
            if let Some(ref ip) = ip_str
                && !connected_ips.contains(ip)
            {
                connected_ips.push(ip.clone());
            }
        }
        self.cached_sessions = session_list;
        drop(sessions);

        let rooms = self.state.rooms.read().await;
        self.cached_room_count = rooms.len();
        let mut room_list = Vec::new();
        for room in rooms.values() {
            let name = room
                .chart
                .read()
                .await
                .as_ref()
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "未选曲".to_string());
            let host = room
                .host
                .read()
                .await
                .upgrade()
                .map(|u| u.name.clone())
                .unwrap_or_else(|| "-".to_string());
            let count = room.users().await.len() + room.monitors().await.len();
            room_list.push((name, host, count.to_string()));
        }
        drop(rooms);
        self.cached_rooms = room_list;

        if self.auto_refresh && self.last_reload.elapsed() >= AUTO_REFRESH_INTERVAL {
            self.state.ip_access.reload().await;
            self.last_reload = Instant::now();
        }

        let cfg = self.state.ip_access.get_config().await;
        self.cached_ip_whitelist_mode = cfg.whitelist_mode;
        self.cached_ip_enabled = cfg.enabled;
        self.cached_blacklist = cfg.blacklist.into_iter().collect();
        self.cached_whitelist = cfg.whitelist.into_iter().collect();
        self.cached_blacklist.sort();
        self.cached_whitelist.sort();
        let len = self.current_list_len();
        let sel = self.ip_entry_state.selected().unwrap_or(0);
        if len == 0 {
            self.ip_entry_state.select(Some(0));
        } else if sel >= len {
            self.ip_entry_state.select(Some(len - 1));
        }
        self.cached_connected_ips = connected_ips;

        let tcfg = self.state.traffic.get_config().await;
        self.cached_traffic_enabled = tcfg.enabled;
        self.cached_traffic_limit = tcfg.limit_bps;
        self.cached_traffic_action = tcfg.action;

        self.cached_closed = self.state.maintenance.is_closed();
    }

    fn current_list_len(&self) -> usize {
        if self.ip_tab == IpAccessTab::Blacklist {
            self.cached_blacklist.len()
        } else {
            self.cached_whitelist.len()
        }
    }

    fn selected_entry(&self) -> Option<String> {
        let list = if self.ip_tab == IpAccessTab::Blacklist {
            &self.cached_blacklist
        } else {
            &self.cached_whitelist
        };
        self.ip_entry_state
            .selected()
            .and_then(|i| list.get(i).cloned())
    }

    fn on_up(&mut self) {
        if self.ip_input_mode || self.traffic_input_mode {
            return;
        }
        if self.active_menu == MenuItem::Traffic {
            self.traffic_sel = if self.traffic_sel == 0 {
                2
            } else {
                self.traffic_sel - 1
            };
            return;
        }
        if self.active_menu == MenuItem::IpAccess {
            match self.ip_tab {
                IpAccessTab::Settings => {
                    if !self.cached_connected_ips.is_empty() {
                        let i = self.ip_list_state.selected().unwrap_or(0);
                        let new_i = if i == 0 {
                            self.cached_connected_ips.len() - 1
                        } else {
                            i - 1
                        };
                        self.ip_list_state.select(Some(new_i));
                        return;
                    }
                }
                _ => {
                    let len = self.current_list_len();
                    if len > 0 {
                        let i = self.ip_entry_state.selected().unwrap_or(0);
                        let new_i = if i == 0 { len - 1 } else { i - 1 };
                        self.ip_entry_state.select(Some(new_i));
                        return;
                    }
                }
            }
        }
        let all = MenuItem::all();
        let i = self.menu_state.selected().unwrap_or(0);
        let new_i = if i == 0 { all.len() - 1 } else { i - 1 };
        self.menu_state.select(Some(new_i));
        self.active_menu = all[new_i];
    }

    fn on_down(&mut self) {
        if self.ip_input_mode || self.traffic_input_mode {
            return;
        }
        if self.active_menu == MenuItem::Traffic {
            self.traffic_sel = (self.traffic_sel + 1) % 3;
            return;
        }
        if self.active_menu == MenuItem::IpAccess {
            match self.ip_tab {
                IpAccessTab::Settings => {
                    if !self.cached_connected_ips.is_empty() {
                        let i = self.ip_list_state.selected().unwrap_or(0);
                        let new_i = (i + 1) % self.cached_connected_ips.len();
                        self.ip_list_state.select(Some(new_i));
                        return;
                    }
                }
                _ => {
                    let len = self.current_list_len();
                    if len > 0 {
                        let i = self.ip_entry_state.selected().unwrap_or(0);
                        let new_i = (i + 1) % len;
                        self.ip_entry_state.select(Some(new_i));
                        return;
                    }
                }
            }
        }
        let all = MenuItem::all();
        let i = self.menu_state.selected().unwrap_or(0);
        let new_i = (i + 1) % all.len();
        self.menu_state.select(Some(new_i));
        self.active_menu = all[new_i];
    }

    fn on_left(&mut self) {
        if self.active_menu == MenuItem::IpAccess && !self.ip_input_mode {
            self.ip_tab = match self.ip_tab {
                IpAccessTab::Blacklist => IpAccessTab::Settings,
                IpAccessTab::Whitelist => IpAccessTab::Blacklist,
                IpAccessTab::Settings => IpAccessTab::Whitelist,
            };
        }
    }

    fn on_right(&mut self) {
        if self.active_menu == MenuItem::IpAccess && !self.ip_input_mode {
            self.ip_tab = match self.ip_tab {
                IpAccessTab::Blacklist => IpAccessTab::Whitelist,
                IpAccessTab::Whitelist => IpAccessTab::Settings,
                IpAccessTab::Settings => IpAccessTab::Blacklist,
            };
        }
    }

    fn on_enter(&mut self) {
        if self.popup_msg.is_some() {
            self.popup_msg = None;
            self.popup_tick = 0;
            return;
        }
        if self.traffic_input_mode {
            self.traffic_input_mode = false;
            let raw = self.traffic_input.trim().to_string();
            self.traffic_input.clear();
            if let Ok(limit) = raw.parse::<u64>() {
                let state = Arc::clone(&self.state);
                tokio::spawn(async move {
                    state.traffic.set_limit_bps(limit).await;
                });
                self.cached_traffic_limit = limit;
                self.show_popup(format!("已设置限速为 {} 字节/秒", limit));
            } else if !raw.is_empty() {
                self.show_popup("无效的数值".to_string());
            }
            return;
        }
        if self.active_menu == MenuItem::Traffic {
            match self.traffic_sel {
                0 => {
                    let new_enabled = !self.cached_traffic_enabled;
                    let state = Arc::clone(&self.state);
                    tokio::spawn(async move {
                        state.traffic.set_enabled(new_enabled).await;
                    });
                    self.cached_traffic_enabled = new_enabled;
                    self.show_popup(
                        if new_enabled {
                            "已启用流量限制"
                        } else {
                            "已停用流量限制"
                        }
                        .to_string(),
                    );
                }
                1 => {
                    self.traffic_input = self.cached_traffic_limit.to_string();
                    self.traffic_input_mode = true;
                }
                2 => {
                    let new_action = self.cached_traffic_action.next();
                    let state = Arc::clone(&self.state);
                    tokio::spawn(async move {
                        state.traffic.set_action(new_action).await;
                    });
                    self.cached_traffic_action = new_action;
                    self.show_popup(format!("超限动作: {}", new_action.label()));
                }
                _ => {}
            }
            return;
        }
        if self.active_menu == MenuItem::Maintenance {
            let new_closed = !self.cached_closed;
            self.cached_closed = new_closed;
            let state = Arc::clone(&self.state);
            tokio::spawn(async move {
                state.maintenance.set_closed(new_closed);
                if new_closed {
                    let ids: Vec<_> = state.sessions.read().await.values().map(|s| s.id).collect();
                    for id in ids {
                        let _ = state.lost_con_tx.send(id).await;
                    }
                }
            });
            self.show_popup(
                if new_closed {
                    "服务器已临时关闭"
                } else {
                    "服务器已重新开放"
                }
                .to_string(),
            );
            return;
        }
        if self.active_menu == MenuItem::IpAccess {
            if self.ip_tab == IpAccessTab::Settings {
                let state = Arc::clone(&self.state);
                let new_mode = !self.cached_ip_whitelist_mode;
                tokio::spawn(async move {
                    state.ip_access.set_whitelist_mode(new_mode).await;
                });
                self.cached_ip_whitelist_mode = new_mode;
                let msg = if new_mode {
                    "已切换到白名单模式"
                } else {
                    "已切换到黑名单模式"
                };
                self.show_popup(msg.to_string());
                return;
            }
            self.ip_input_mode = !self.ip_input_mode;
            if self.ip_input_mode {
                self.ip_input.clear();
                self.ip_edit_old = None;
                return;
            }
            let raw = self.ip_input.trim().to_string();
            self.ip_input.clear();
            let old = self.ip_edit_old.take();
            let is_edit = old.is_some();
            if raw.is_empty() {
                return;
            }
            let ip_for_msg = raw.clone();
            let state = Arc::clone(&self.state);
            let tab = self.ip_tab;
            let list = if tab == IpAccessTab::Blacklist {
                &mut self.cached_blacklist
            } else {
                &mut self.cached_whitelist
            };
            if let Some(ref o) = old {
                list.retain(|x| x != o);
            }
            if !list.iter().any(|x| x == &raw) {
                list.push(raw.clone());
            }
            list.sort();
            tokio::spawn(async move {
                if let Some(ref old_ip) = old {
                    match tab {
                        IpAccessTab::Blacklist => {
                            state.ip_access.remove_from_blacklist(old_ip).await
                        }
                        IpAccessTab::Whitelist => {
                            state.ip_access.remove_from_whitelist(old_ip).await
                        }
                        _ => {}
                    }
                }
                match tab {
                    IpAccessTab::Blacklist => state.ip_access.add_to_blacklist(raw).await,
                    IpAccessTab::Whitelist => state.ip_access.add_to_whitelist(raw).await,
                    _ => {}
                }
            });
            let msg = match tab {
                IpAccessTab::Blacklist => {
                    if is_edit {
                        format!("已更新黑名单条目为 {}", ip_for_msg)
                    } else {
                        format!("已添加 {} 到黑名单", ip_for_msg)
                    }
                }
                IpAccessTab::Whitelist => {
                    if is_edit {
                        format!("已更新白名单条目为 {}", ip_for_msg)
                    } else {
                        format!("已添加 {} 到白名单", ip_for_msg)
                    }
                }
                _ => String::new(),
            };
            if !msg.is_empty() {
                self.show_popup(msg);
            }
        }
    }

    fn on_tab(&mut self) {
        if self.active_menu == MenuItem::IpAccess && self.ip_tab == IpAccessTab::Settings {
            let state = Arc::clone(&self.state);
            let new_enabled = !self.cached_ip_enabled;
            tokio::spawn(async move {
                state.ip_access.set_enabled(new_enabled).await;
            });
            self.cached_ip_enabled = new_enabled;
            let msg = if new_enabled {
                "已启用 IP 访问控制"
            } else {
                "已停用 IP 访问控制"
            };
            self.show_popup(msg.to_string());
        }
    }

    fn on_char(&mut self, c: char) {
        if self.traffic_input_mode {
            self.traffic_input.push(c);
            return;
        }
        if self.ip_input_mode {
            self.ip_input.push(c);
            return;
        }
        match c {
            'r' | 'R' => {
                let state = Arc::clone(&self.state);
                tokio::spawn(async move {
                    state.ip_access.reload().await;
                });
                self.last_reload = Instant::now();
                self.show_popup("已重新加载 IP 访问配置".to_string());
                return;
            }
            'f' | 'F' => {
                self.auto_refresh = !self.auto_refresh;
                self.last_reload = Instant::now();
                let msg = if self.auto_refresh {
                    "已开启自动刷新"
                } else {
                    "已关闭自动刷新"
                };
                self.show_popup(msg.to_string());
                return;
            }
            _ => {}
        }
        if self.active_menu == MenuItem::IpAccess && self.ip_tab == IpAccessTab::Settings {
            let state = Arc::clone(&self.state);
            let ips = self.cached_connected_ips.clone();
            match c {
                '1' => {
                    for ip in &ips {
                        if !self.cached_blacklist.contains(ip) {
                            self.cached_blacklist.push(ip.clone());
                        }
                    }
                    tokio::spawn(async move {
                        for ip in ips {
                            state.ip_access.add_to_blacklist(ip).await;
                        }
                    });
                    self.show_popup("已将所有当前连接IP加入黑名单".to_string());
                }
                '2' => {
                    for ip in &ips {
                        if !self.cached_whitelist.contains(ip) {
                            self.cached_whitelist.push(ip.clone());
                        }
                    }
                    tokio::spawn(async move {
                        for ip in ips {
                            state.ip_access.add_to_whitelist(ip).await;
                        }
                    });
                    self.show_popup("已将所有当前连接IP加入白名单".to_string());
                }
                'b' | 'B' => {
                    if let Some(i) = self.ip_list_state.selected()
                        && let Some(ip) = self.cached_connected_ips.get(i)
                    {
                        let ip_clone = ip.clone();
                        if !self.cached_blacklist.contains(ip) {
                            self.cached_blacklist.push(ip.clone());
                        }
                        tokio::spawn(async move {
                            state.ip_access.add_to_blacklist(ip_clone).await;
                        });
                        self.show_popup(format!("已将 {} 加入黑名单", ip));
                    }
                }
                'w' | 'W' => {
                    if let Some(i) = self.ip_list_state.selected()
                        && let Some(ip) = self.cached_connected_ips.get(i)
                    {
                        let ip_clone = ip.clone();
                        if !self.cached_whitelist.contains(ip) {
                            self.cached_whitelist.push(ip.clone());
                        }
                        tokio::spawn(async move {
                            state.ip_access.add_to_whitelist(ip_clone).await;
                        });
                        self.show_popup(format!("已将 {} 加入白名单", ip));
                    }
                }
                _ => {}
            }
            return;
        }
        if self.active_menu == MenuItem::IpAccess && self.ip_tab != IpAccessTab::Settings {
            match c {
                'd' | 'D' => {
                    if let Some(ip) = self.selected_entry() {
                        if self.ip_tab == IpAccessTab::Blacklist {
                            self.cached_blacklist.retain(|x| x != &ip);
                        } else {
                            self.cached_whitelist.retain(|x| x != &ip);
                        }
                        let state = Arc::clone(&self.state);
                        let tab = self.ip_tab;
                        let ip_clone = ip.clone();
                        tokio::spawn(async move {
                            match tab {
                                IpAccessTab::Blacklist => {
                                    state.ip_access.remove_from_blacklist(&ip_clone).await
                                }
                                IpAccessTab::Whitelist => {
                                    state.ip_access.remove_from_whitelist(&ip_clone).await
                                }
                                _ => {}
                            }
                        });
                        self.show_popup(format!("已从列表移除 {}", ip));
                    }
                }
                'e' | 'E' => {
                    if let Some(ip) = self.selected_entry() {
                        self.ip_input = ip.clone();
                        self.ip_edit_old = Some(ip);
                        self.ip_input_mode = true;
                    }
                }
                _ => {}
            }
        }
    }

    fn on_backspace(&mut self) {
        if self.traffic_input_mode {
            self.traffic_input.pop();
            return;
        }
        if self.ip_input_mode {
            self.ip_input.pop();
        }
    }

    fn on_esc(&mut self) {
        if self.traffic_input_mode {
            self.traffic_input_mode = false;
            self.traffic_input.clear();
        } else if self.ip_input_mode {
            self.ip_input_mode = false;
            self.ip_input.clear();
            self.ip_edit_old = None;
        } else {
            self.exit = true;
        }
    }

    fn on_page_up(&mut self) {
        if self.active_menu == MenuItem::Logs && self.log_scroll > 0 {
            self.log_scroll = self.log_scroll.saturating_sub(5);
        }
    }

    fn on_page_down(&mut self) {
        if self.active_menu == MenuItem::Logs {
            self.log_scroll = self.log_scroll.saturating_add(5);
        }
    }
}

pub fn run_console(state: Arc<ServerState>, log_buffer: Arc<Mutex<Vec<String>>>) {
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            if let Err(e) = run_tui(state, log_buffer).await {
                eprintln!("TUI error: {}", e);
            }
        });
    });
}

async fn run_tui(state: Arc<ServerState>, log_buffer: Arc<Mutex<Vec<String>>>) -> io::Result<()> {
    let mut terminal = setup_terminal()?;
    let mut app = App::new(state, log_buffer);

    let mut last_tick = tokio::time::Instant::now();
    let tick_rate = Duration::from_millis(250);

    loop {
        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout).unwrap_or(false)
            && let Ok(Event::Key(key)) = event::read()
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Up => app.on_up(),
                KeyCode::Down => app.on_down(),
                KeyCode::Left => app.on_left(),
                KeyCode::Right => app.on_right(),
                KeyCode::Enter => app.on_enter(),
                KeyCode::Esc => app.on_esc(),
                KeyCode::Char(c) => app.on_char(c),
                KeyCode::Backspace => app.on_backspace(),
                KeyCode::PageUp => app.on_page_up(),
                KeyCode::PageDown => app.on_page_down(),
                KeyCode::Tab => app.on_tab(),
                _ => {}
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.on_tick().await;
            last_tick = tokio::time::Instant::now();
        }

        terminal.draw(|f| draw(f, &mut app))?;

        if app.exit {
            break;
        }
    }

    restore_terminal()?;
    std::process::exit(0);
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    #[cfg(windows)]
    {
        unsafe extern "system" {
            fn GetConsoleWindow() -> isize;
        }
        unsafe extern "system" {
            fn ShowWindow(hwnd: isize, nCmdShow: i32) -> i32;
        }
        const SW_MAXIMIZE: i32 = 3;
        unsafe {
            let hwnd = GetConsoleWindow();
            if hwnd != 0 {
                ShowWindow(hwnd, SW_MAXIMIZE);
            }
        }
    }

    let mut stdout = io::stdout();
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
}

fn restore_terminal() -> io::Result<()> {
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::event::DisableMouseCapture,
        crossterm::terminal::LeaveAlternateScreen
    )?;
    crossterm::terminal::disable_raw_mode()?;
    Ok(())
}

fn draw(f: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(16), Constraint::Min(0)])
        .split(f.area());

    draw_menu_sidebar(f, app, chunks[0]);

    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(6)])
        .split(chunks[1]);

    match app.active_menu {
        MenuItem::Dashboard => draw_dashboard(f, app, right_chunks[0]),
        MenuItem::Sessions => draw_sessions(f, app, right_chunks[0]),
        MenuItem::Rooms => draw_rooms(f, app, right_chunks[0]),
        MenuItem::Network => draw_network(f, app, right_chunks[0]),
        MenuItem::IpAccess => draw_ip_access(f, app, right_chunks[0]),
        MenuItem::Traffic => draw_traffic(f, app, right_chunks[0]),
        MenuItem::Maintenance => draw_maintenance(f, app, right_chunks[0]),
        MenuItem::Logs => draw_logs(f, app, right_chunks[0]),
    }

    draw_log_footer(f, app, right_chunks[1]);

    if let Some(ref msg) = app.popup_msg {
        draw_popup(f, msg);
    }
}

fn draw_menu_sidebar(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(" 菜单 ")
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let items: Vec<ListItem> = MenuItem::all()
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let style = if app.menu_state.selected() == Some(i) {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(item.title()).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default())
        .highlight_style(Style::default().add_modifier(Modifier::BOLD));
    f.render_widget(list, inner);
}

fn draw_dashboard(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(" 仪表盘 ")
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(inner);

    let mode_str = if app.cached_ip_whitelist_mode {
        "白名单模式"
    } else {
        "黑名单模式"
    };
    let enabled_str = if app.cached_ip_enabled {
        "已启用"
    } else {
        "已停用"
    };
    let items = vec![
        (
            "在线会话",
            app.cached_session_count.to_string(),
            Color::Blue,
        ),
        ("在线用户", app.cached_user_count.to_string(), Color::Blue),
        ("活跃房间", app.cached_room_count.to_string(), Color::Blue),
        (
            "IP控制",
            format!("{} ({})", enabled_str, mode_str),
            Color::Blue,
        ),
    ];

    for (i, (label, value, color)) in items.into_iter().enumerate() {
        let row = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(12), Constraint::Min(0)])
            .split(chunks[i]);

        let label_para = Paragraph::new(label)
            .style(Style::default().fg(Color::Gray))
            .alignment(Alignment::Right);
        f.render_widget(label_para, row[0]);

        let value_para = Paragraph::new(value)
            .style(Style::default().fg(color).add_modifier(Modifier::BOLD))
            .alignment(Alignment::Left);
        f.render_widget(value_para, row[1]);
    }
}

fn draw_sessions(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(" 会话列表 ")
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED);
    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.cached_sessions.is_empty() {
        let para = Paragraph::new("暂无活跃会话").style(Style::default().fg(Color::DarkGray));
        f.render_widget(para, inner);
        return;
    }

    let rows: Vec<Row> = app
        .cached_sessions
        .iter()
        .map(|(id, name, ver, ip)| {
            let ip_str = ip.as_deref().unwrap_or("-");
            Row::new(vec![
                id.chars().take(8).collect::<String>(),
                name.clone(),
                ver.clone(),
                ip_str.to_string(),
            ])
        })
        .collect();

    let header = Row::new(vec!["会话ID", "用户名", "版本", "IP"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .fg(Color::Blue);
    let table = Table::new(
        rows,
        [
            Constraint::Length(10),
            Constraint::Percentage(30),
            Constraint::Length(6),
            Constraint::Percentage(40),
        ],
    )
    .header(header)
    .block(Block::default());
    f.render_widget(table, inner);
}

fn draw_rooms(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(" 房间列表 ")
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let room_rows: Vec<Row> = if app.cached_rooms.is_empty() {
        vec![Row::new(vec!["暂无房间"]).style(Style::default().fg(Color::DarkGray))]
    } else {
        app.cached_rooms
            .iter()
            .map(|(name, owner, count)| Row::new(vec![name.clone(), owner.clone(), count.clone()]))
            .collect()
    };

    let header = Row::new(vec!["房间名", "房主", "人数"])
        .style(Style::default().add_modifier(Modifier::BOLD))
        .fg(Color::Blue);
    let table = Table::new(
        room_rows,
        [
            Constraint::Percentage(50),
            Constraint::Percentage(30),
            Constraint::Percentage(20),
        ],
    )
    .header(header)
    .block(Block::default());
    f.render_widget(table, inner);
}

fn draw_network(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(" 网络信息 ")
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut text = vec![
        Line::from(""),
        Line::from(vec![
            Span::raw("监听端口: "),
            Span::styled(
                app.net_port.to_string(),
                Style::default()
                    .fg(Color::Blue)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from("监听地址:"),
    ];

    for addr in &app.net_listen_addrs {
        text.push(Line::from(format!("    {}", addr)));
    }

    text.push(Line::from(""));
    text.push(Line::from("本机IP:"));
    if app.net_local_ips.is_empty() {
        text.push(Line::from("    未检测到").fg(Color::DarkGray));
    } else {
        for ip in &app.net_local_ips {
            text.push(Line::from(format!("    {}", ip)));
        }
    }

    text.push(Line::from(""));
    text.push(Line::from("玩家连接地址:"));
    if app.net_local_ips.is_empty() {
        text.push(Line::from("    未检测到").fg(Color::DarkGray));
    } else {
        for ip in &app.net_local_ips {
            text.push(Line::from(format!("    {}:{}", ip, app.net_port)).fg(Color::Green));
        }
    }

    let para = Paragraph::new(text).wrap(Wrap { trim: true });
    f.render_widget(para, inner);
}

fn local_ips() -> Vec<String> {
    let mut ips = Vec::new();
    if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0")
        && sock.connect("8.8.8.8:80").is_ok()
        && let Ok(addr) = sock.local_addr()
    {
        ips.push(addr.ip().to_string());
    }
    ips
}

fn draw_ip_access(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(" IP访问控制 ")
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let tab_titles: Vec<Line> = vec![
        Line::from(Span::styled(
            " 黑名单 ",
            if app.ip_tab == IpAccessTab::Blacklist {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Red)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            },
        )),
        Line::from(Span::styled(
            " 白名单 ",
            if app.ip_tab == IpAccessTab::Whitelist {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Green)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            },
        )),
        Line::from(Span::styled(
            " 设置 ",
            if app.ip_tab == IpAccessTab::Settings {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::White)
            },
        )),
    ];

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(0)])
        .split(inner);

    let tabs = Tabs::new(tab_titles)
        .block(Block::default().borders(Borders::BOTTOM))
        .select(match app.ip_tab {
            IpAccessTab::Blacklist => 0,
            IpAccessTab::Whitelist => 1,
            IpAccessTab::Settings => 2,
        });
    f.render_widget(tabs, chunks[0]);

    let content_area = chunks[1];

    match app.ip_tab {
        IpAccessTab::Blacklist | IpAccessTab::Whitelist => {
            let ip_list = if app.ip_tab == IpAccessTab::Blacklist {
                &app.cached_blacklist
            } else {
                &app.cached_whitelist
            };
            let items: Vec<ListItem> = if ip_list.is_empty() {
                vec![ListItem::new("列表为空").style(Style::default().fg(Color::DarkGray))]
            } else {
                ip_list
                    .iter()
                    .enumerate()
                    .map(|(i, ip)| {
                        let style = if app.ip_entry_state.selected() == Some(i) {
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::White)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        ListItem::new(ip.as_str()).style(style)
                    })
                    .collect()
            };

            let list_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(1)])
                .split(content_area);

            let list = List::new(items).block(Block::default());
            f.render_widget(list, list_chunks[0]);

            let hint = Paragraph::new("上下键选择  Enter添加  E编辑  D删除")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);
            f.render_widget(hint, list_chunks[1]);

            if app.ip_input_mode {
                let title = if app.ip_edit_old.is_some() {
                    " 编辑IP地址 (Enter保存, Esc取消) "
                } else {
                    " 输入IP地址 (Enter确认, Esc取消) "
                };
                let input_block = Block::default()
                    .title(title)
                    .borders(Borders::ALL)
                    .border_set(symbols::border::ROUNDED)
                    .style(Style::default().fg(Color::Yellow));
                let input_para = Paragraph::new(app.ip_input.as_str()).block(input_block);
                let input_area = centered_rect(60, 20, f.area());
                f.render_widget(Clear, input_area);
                f.render_widget(input_para, input_area);
            }
        }
        IpAccessTab::Settings => {
            let mode = if app.cached_ip_whitelist_mode {
                ("白名单模式", Color::Green)
            } else {
                ("黑名单模式", Color::Red)
            };
            let enabled = if app.cached_ip_enabled {
                ("已启用", Color::Green)
            } else {
                ("已停用", Color::Red)
            };

            let settings_text = vec![
                Line::from(""),
                Line::from(vec![
                    Span::raw("状态: "),
                    Span::styled(
                        enabled.0,
                        Style::default().fg(enabled.1).add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::raw("当前模式: "),
                    Span::styled(
                        mode.0,
                        Style::default().fg(mode.1).add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(vec![
                    Span::raw("自动刷新: "),
                    Span::styled(
                        if app.auto_refresh {
                            "已开启"
                        } else {
                            "已关闭"
                        },
                        Style::default()
                            .fg(if app.auto_refresh {
                                Color::Green
                            } else {
                                Color::DarkGray
                            })
                            .add_modifier(Modifier::BOLD),
                    ),
                ]),
                Line::from(""),
                Line::from("按 R 手动刷新配置文件，按 F 切换自动刷新"),
                Line::from("按 Tab 启用/停用 IP 访问控制"),
                Line::from("按 Enter 切换黑/白名单模式"),
                Line::from("按 ← → 切换标签页，在黑名单/白名单页面按 Enter 添加IP"),
                Line::from("在黑名单/白名单页面按上下键选择，E 编辑，D 删除"),
                Line::from("IP 支持精确、通配符(* ?)、字符类([a-f])与 CIDR(10.0.0.0/8)"),
                Line::from("按 1 批量加入黑名单，按 2 批量加入白名单"),
                Line::from("按 B 将选中IP加入黑名单，按 W 加入白名单"),
                Line::from(""),
                Line::from("当前连接IP (上下键选择):"),
            ];

            let settings_para = Paragraph::new(settings_text).wrap(Wrap { trim: true });
            f.render_widget(settings_para, content_area);

            if !app.cached_connected_ips.is_empty() {
                let ip_items: Vec<ListItem> = app
                    .cached_connected_ips
                    .iter()
                    .enumerate()
                    .map(|(i, ip)| {
                        let style = if app.ip_list_state.selected() == Some(i) {
                            Style::default()
                                .fg(Color::Black)
                                .bg(Color::White)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(Color::White)
                        };
                        ListItem::new(format!("  {}", ip)).style(style)
                    })
                    .collect();
                let ip_list = List::new(ip_items).block(Block::default());
                let list_height = app.cached_connected_ips.len().min(5) as u16 + 2;
                let list_area = Rect {
                    x: content_area.x,
                    y: content_area.y + content_area.height.saturating_sub(list_height),
                    width: content_area.width,
                    height: list_height,
                };
                f.render_widget(ip_list, list_area);
            }
        }
    }
}

fn draw_traffic(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(" 流量限制 ")
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let (enabled_str, enabled_color) = if app.cached_traffic_enabled {
        ("已启用", Color::Green)
    } else {
        ("已停用", Color::Red)
    };
    let action_color = match app.cached_traffic_action {
        ExceedAction::Warn => Color::Yellow,
        ExceedAction::Kick => Color::Red,
        ExceedAction::Ban => Color::Magenta,
    };
    let limit_str = if app.cached_traffic_limit == 0 {
        "不限速".to_string()
    } else {
        format!(
            "{} 字节/秒 ({:.2} KB/s)",
            app.cached_traffic_limit,
            app.cached_traffic_limit as f64 / 1024.0
        )
    };

    let rows: Vec<(&str, String, Color)> = vec![
        ("启用状态", enabled_str.to_string(), enabled_color),
        ("限速(按IP)", limit_str, Color::Cyan),
        (
            "超限动作",
            app.cached_traffic_action.label().to_string(),
            action_color,
        ),
    ];

    let mut text = vec![Line::from("")];
    for (i, (label, value, color)) in rows.iter().enumerate() {
        let selected = app.traffic_sel as usize == i;
        let row_style = if selected {
            Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::White)
        };
        text.push(Line::from(vec![
            Span::styled(if selected { "> " } else { "  " }, row_style),
            Span::styled(format!("{:<12}", label), row_style),
            Span::styled(
                value.clone(),
                Style::default().fg(*color).add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    text.push(Line::from(""));
    text.push(Line::from("上下键选择，Enter 启用/修改，Esc 取消编辑"));
    text.push(Line::from("限速按同一 IP 所有连接合计的收发字节/秒计算"));
    text.push(Line::from("限速值 0 表示不限制"));

    let para = Paragraph::new(text).wrap(Wrap { trim: true });
    f.render_widget(para, inner);

    if app.traffic_input_mode {
        let input_block = Block::default()
            .title(" 输入限速值(字节/秒) (Enter确认, Esc取消) ")
            .borders(Borders::ALL)
            .border_set(symbols::border::ROUNDED)
            .style(Style::default().fg(Color::Yellow));
        let input_para = Paragraph::new(app.traffic_input.as_str()).block(input_block);
        let input_area = centered_rect(60, 20, f.area());
        f.render_widget(Clear, input_area);
        f.render_widget(input_para, input_area);
    }
}

fn draw_maintenance(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(" 服务器 ")
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let (status, color) = if app.cached_closed {
        ("已临时关闭", Color::Red)
    } else {
        ("运行中", Color::Green)
    };

    let text = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled("状态: ", Style::default().fg(Color::White)),
            Span::styled(
                status,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from("Enter: 切换 临时关闭 / 重新开放"),
        Line::from("关闭后拒绝新连接，并断开所有现有会话"),
        Line::from("该状态不会持久化，重启后恢复为运行中"),
    ];

    let para = Paragraph::new(text).wrap(Wrap { trim: true });
    f.render_widget(para, inner);
}

fn draw_logs(f: &mut Frame, app: &mut App, area: Rect) {
    let block = Block::default()
        .title(" 日志 ")
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let text = if app.logs.is_empty() {
        Text::from("暂无日志显示")
    } else {
        let lines: Vec<Line> = app.logs.iter().map(|l| Line::from(l.as_str())).collect();
        Text::from(lines)
    };

    let para = Paragraph::new(text)
        .wrap(Wrap { trim: true })
        .scroll((app.log_scroll, 0));
    f.render_widget(para, inner);
}

fn draw_log_footer(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" 最新日志 ")
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let recent_logs: Vec<Line> = app
        .logs
        .iter()
        .rev()
        .take(3)
        .map(|l| Line::from(l.as_str()))
        .collect();

    let text = if recent_logs.is_empty() {
        Text::from(vec![Line::from("暂无日志".fg(Color::DarkGray))])
    } else {
        Text::from(recent_logs)
    };

    let para = Paragraph::new(text).wrap(Wrap { trim: true });
    f.render_widget(para, inner);
}

fn draw_popup(f: &mut Frame, msg: &str) {
    let area = centered_rect(50, 20, f.area());
    let block = Block::default()
        .title(" 提示 ")
        .borders(Borders::ALL)
        .border_set(symbols::border::ROUNDED)
        .style(Style::default().fg(Color::Yellow).bg(Color::Black));
    let para = Paragraph::new(msg)
        .block(block)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    f.render_widget(Clear, area);
    f.render_widget(para, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
