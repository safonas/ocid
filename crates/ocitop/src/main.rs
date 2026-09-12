//! `ocitop` — interactive terminal dashboard for a running `ocid` daemon.
//!
//! Polls the control API for state (status, releases, peers) and streams live
//! `DaemonEvent`s over SSE from `/_ocid/events`. Policy actions (seed, follow,
//! pin) edit `policy.toml` and ask the daemon to reload, exactly like `ocictl`.

use std::{
    collections::VecDeque,
    io,
    net::SocketAddr,
    path::PathBuf,
    time::{Duration, Instant},
};

use anyhow::Result;
use clap::Parser;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ocid_core::{
    api::{
        DaemonEvent, GcReport, GcReq, MetricsSnapshot, OkResp, PeerInfo, ReleaseInfo, RmReq,
        RmResp, Status, SyncReq, SyncResp,
    },
    client::Client,
    config::{Config, Mode, Policy},
    identity::did_key,
    paths::Paths,
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Clear, Gauge, List, ListItem, Paragraph, Row, Sparkline, Table,
        TableState, Tabs,
    },
    Frame, Terminal,
};

const TICK: Duration = Duration::from_millis(150);
const POLL: Duration = Duration::from_secs(2);
const NOTICE: Duration = Duration::from_secs(3);
const LOG_CAP: usize = 200;
/// Points kept per sparkline (~4 min at the 2 s poll interval).
const HIST: usize = 120;

#[derive(Debug, Parser)]
#[command(
    name = "ocitop",
    version,
    about = "Interactive terminal dashboard for the ocid daemon"
)]
struct Cli {
    /// Node home directory.
    #[arg(long, env = "OCID_HOME")]
    home: Option<PathBuf>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Images = 0,
    Peers = 1,
    Events = 2,
    Metrics = 3,
}

impl Tab {
    fn next(self) -> Self {
        match self {
            Tab::Images => Tab::Peers,
            Tab::Peers => Tab::Events,
            Tab::Events => Tab::Metrics,
            Tab::Metrics => Tab::Images,
        }
    }
}

#[derive(Clone)]
enum Modal {
    None,
    ConfirmGc {
        force: bool,
    },
    ConfirmDelete(String),
    /// Seed `<publisher>/<name>`; the user types the retention mode.
    SeedPrompt {
        target: String,
        input: String,
    },
    Notice(String, Instant),
}

struct LogEntry {
    time: String,
    kind: &'static str,
    msg: String,
    color: Color,
}

struct App {
    client: Client,
    listen: SocketAddr,
    paths: Paths,
    policy: Policy,
    status: Option<Status>,
    releases: Vec<ReleaseInfo>,
    peers: Vec<PeerInfo>,
    metrics: Option<MetricsSnapshot>,
    /// Per-poll deltas for the sparklines (one point per `poll()`).
    hist_http: Vec<u64>,
    hist_bytes: Vec<u64>,
    hist_repl: Vec<u64>,
    hist_gossip: Vec<u64>,
    images: TableState,
    peer_rows: TableState,
    tab: Tab,
    modal: Modal,
    log: VecDeque<LogEntry>,
    last_poll: Instant,
    quit: bool,
}

impl App {
    async fn new(paths: Paths) -> Result<Self> {
        let cfg = Config::load(&paths).unwrap_or_default();
        let mut app = Self {
            client: Client::new(cfg.listen),
            listen: cfg.listen,
            policy: Policy::load(&paths).unwrap_or_default(),
            paths,
            status: None,
            releases: Vec::new(),
            peers: Vec::new(),
            metrics: None,
            hist_http: Vec::new(),
            hist_bytes: Vec::new(),
            hist_repl: Vec::new(),
            hist_gossip: Vec::new(),
            images: TableState::default(),
            peer_rows: TableState::default(),
            tab: Tab::Images,
            modal: Modal::None,
            log: VecDeque::with_capacity(LOG_CAP),
            last_poll: Instant::now(),
            quit: false,
        };
        app.poll().await;
        Ok(app)
    }

    // --- state -------------------------------------------------------------

    async fn poll(&mut self) {
        if let Ok(s) = self.client.get::<Status>("/_ocid/status").await {
            self.status = Some(s);
        }
        if let Ok(r) = self.client.get::<Vec<ReleaseInfo>>("/_ocid/releases").await {
            self.releases = r;
        }
        if let Ok(p) = self.client.get::<Vec<PeerInfo>>("/_ocid/peers").await {
            self.peers = p;
        }
        if let Ok(p) = Policy::load(&self.paths) {
            self.policy = p;
        }
        if let Ok(m) = self.client.get::<MetricsSnapshot>("/_ocid/metrics").await {
            self.push_metrics(m);
        }
        clamp(&mut self.images, self.releases.len());
        clamp(&mut self.peer_rows, self.peers.len());
        self.last_poll = Instant::now();
    }

    /// Record a fresh snapshot, keeping per-poll deltas for the sparklines.
    fn push_metrics(&mut self, m: MetricsSnapshot) {
        if let Some(prev) = &self.metrics {
            push_delta(
                &mut self.hist_http,
                m.http_requests_total,
                prev.http_requests_total,
            );
            push_delta(
                &mut self.hist_bytes,
                m.http_bytes_served,
                prev.http_bytes_served,
            );
            push_delta(
                &mut self.hist_repl,
                m.releases_replicated.saturating_add(m.blobs_fetched),
                prev.releases_replicated.saturating_add(prev.blobs_fetched),
            );
            push_delta(
                &mut self.hist_gossip,
                m.announcements_received
                    .saturating_add(m.announcements_sent),
                prev.announcements_received
                    .saturating_add(prev.announcements_sent),
            );
        }
        self.metrics = Some(m);
    }

    fn selected(&self) -> Option<&ReleaseInfo> {
        self.images.selected().and_then(|i| self.releases.get(i))
    }

    fn move_selection(&mut self, delta: isize) {
        let (state, len) = match self.tab {
            Tab::Images => (&mut self.images, self.releases.len()),
            Tab::Peers => (&mut self.peer_rows, self.peers.len()),
            Tab::Events | Tab::Metrics => return,
        };
        if len == 0 {
            return;
        }
        let cur = state.selected().unwrap_or(0) as isize;
        state.select(Some((cur + delta).rem_euclid(len as isize) as usize));
    }

    fn log(&mut self, kind: &'static str, msg: String, color: Color) {
        if self.log.len() >= LOG_CAP {
            self.log.pop_back();
        }
        self.log.push_front(LogEntry {
            time: chrono::Local::now().format("%H:%M:%S").to_string(),
            kind,
            msg,
            color,
        });
    }

    /// Log + transient popup: for the outcome of user actions.
    fn report(&mut self, kind: &'static str, msg: String, color: Color) {
        self.modal = Modal::Notice(msg.clone(), Instant::now());
        self.log(kind, msg, color);
    }

    fn on_event(&mut self, ev: DaemonEvent) {
        match ev {
            DaemonEvent::Gossip {
                publisher,
                name,
                tag,
                outbound,
            } => self.log(
                "GOSSIP",
                format!(
                    "{} {}/{name}:{tag}",
                    if outbound { "→" } else { "←" },
                    short(&publisher)
                ),
                if outbound { Color::Blue } else { Color::Green },
            ),
            DaemonEvent::ReleaseSaved {
                publisher,
                name,
                tag,
                blobs,
            } => self.log(
                "SAVED",
                format!("{}/{name}:{tag} ({blobs} blobs)", short(&publisher)),
                Color::LightGreen,
            ),
            DaemonEvent::Pruned {
                publisher,
                name,
                tag,
                reason,
            } => self.log(
                "PRUNE",
                format!("{}/{name}:{tag} ({reason})", short(&publisher)),
                Color::Yellow,
            ),
            DaemonEvent::PeerChange { id, connected } => self.log(
                "PEER",
                format!(
                    "{} {}",
                    short(&id),
                    if connected {
                        "connected"
                    } else {
                        "disconnected"
                    }
                ),
                if connected {
                    Color::Cyan
                } else {
                    Color::DarkGray
                },
            ),
            DaemonEvent::HttpRequest {
                method,
                path,
                status,
            } => self.log(
                "HTTP",
                format!("{method} {path} → {status}"),
                if status < 400 {
                    Color::Gray
                } else {
                    Color::Red
                },
            ),
        }
    }

    // --- actions -----------------------------------------------------------

    async fn reload_policy(&mut self) {
        let _ = self.policy.save(&self.paths);
        let _: Result<OkResp> = self
            .client
            .post("/_ocid/policy/reload", &serde_json::json!({}))
            .await;
        self.poll().await;
    }

    async fn sync(&mut self) {
        let req = SyncReq { peer: None };
        match self.client.post::<SyncResp>("/_ocid/sync", &req).await {
            Ok(r) => self.report(
                "SYNC",
                format!("synced with {} peer(s)", r.synced),
                Color::Green,
            ),
            Err(e) => self.report("ERROR", format!("sync failed: {e}"), Color::Red),
        }
        self.poll().await;
    }

    async fn gc(&mut self, force: bool) {
        let req = GcReq {
            dry_run: false,
            force,
        };
        match self.client.post::<GcReport>("/_ocid/gc", &req).await {
            Ok(r) => self.report(
                "GC",
                format!(
                    "removed {} release(s), {} blob(s), {} freed",
                    r.releases_removed.len(),
                    r.blobs_removed,
                    human_bytes(r.bytes_freed)
                ),
                Color::Green,
            ),
            Err(e) => self.report("ERROR", format!("gc failed: {e}"), Color::Red),
        }
        self.poll().await;
    }

    async fn delete(&mut self, reference: String) {
        let req = RmReq {
            reference,
            all_tags: false,
        };
        match self.client.post::<RmResp>("/_ocid/rm", &req).await {
            Ok(r) => {
                let note = if r.still_wanted {
                    " (policy still wants it; will be re-fetched)"
                } else {
                    ""
                };
                self.report(
                    "DELETE",
                    format!("removed {}{note}", r.removed.join(", ")),
                    Color::Red,
                );
            }
            Err(e) => self.report("ERROR", format!("delete failed: {e}"), Color::Red),
        }
        self.poll().await;
    }

    /// Selected release unless it is our own (own releases are always kept;
    /// policy rules for them make no sense).
    fn selected_foreign(&mut self) -> Option<ReleaseInfo> {
        let r = self.selected()?.clone();
        if r.mine {
            self.report(
                "POLICY",
                "own release: always kept, no policy needed".into(),
                Color::Yellow,
            );
            return None;
        }
        Some(r)
    }

    async fn toggle_pin(&mut self) {
        let Some(r) = self.selected_foreign() else {
            return;
        };
        let s = &r.summary;
        let reference = format!("{}/{}:{}", s.publisher, s.name, s.tag);
        if self.policy.is_pinned(&s.publisher, &s.name, &s.tag) {
            let _ = self.policy.remove_pin(&reference);
            self.report("PIN", format!("unpinned {reference}"), Color::Yellow);
        } else {
            let _ = self.policy.add_pin(&reference);
            self.report("PIN", format!("pinned {reference}"), Color::Magenta);
        }
        self.reload_policy().await;
    }

    async fn follow(&mut self) {
        let Some(r) = self.selected_foreign() else {
            return;
        };
        let p = r.summary.publisher;
        let msg = if self.policy.add_follow(&p, Mode::Latest) {
            format!("following {} (latest)", short(&p))
        } else {
            format!("already following {}", short(&p))
        };
        self.report("FOLLOW", msg, Color::Cyan);
        self.reload_policy().await;
    }

    async fn seed(&mut self, target: String, input: String) {
        let mode: Mode = match input.trim().parse() {
            Ok(m) => m,
            Err(e) => {
                self.report("ERROR", format!("bad mode {input:?}: {e}"), Color::Red);
                return;
            }
        };
        match self.policy.add_seed(&target, mode) {
            Ok(_) => self.report("SEED", format!("seeding {target} ({mode})"), Color::Cyan),
            Err(e) => self.report("ERROR", format!("seed failed: {e}"), Color::Red),
        }
        self.reload_policy().await;
    }

    // --- input -------------------------------------------------------------

    async fn on_key(&mut self, key: KeyEvent) {
        // Modal keys first.
        match self.modal.clone() {
            Modal::ConfirmGc { force } => {
                self.modal = Modal::None;
                if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    self.gc(force).await;
                }
                return;
            }
            Modal::ConfirmDelete(reference) => {
                self.modal = Modal::None;
                if matches!(key.code, KeyCode::Char('y' | 'Y')) {
                    self.delete(reference).await;
                }
                return;
            }
            Modal::SeedPrompt { target, mut input } => {
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('d'))
                {
                    self.modal = Modal::None;
                    return;
                }
                match key.code {
                    KeyCode::Esc => self.modal = Modal::None,
                    KeyCode::Enter => {
                        self.modal = Modal::None;
                        self.seed(target, input).await;
                    }
                    KeyCode::Backspace => {
                        input.pop();
                        self.modal = Modal::SeedPrompt { target, input };
                    }
                    KeyCode::Char(c) => {
                        input.push(c);
                        self.modal = Modal::SeedPrompt { target, input };
                    }
                    _ => {}
                }
                return;
            }
            Modal::Notice(..) => {
                self.modal = Modal::None;
                return;
            }
            Modal::None => {}
        }

        match key.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Tab => self.tab = self.tab.next(),
            KeyCode::Char('1') => self.tab = Tab::Images,
            KeyCode::Char('2') => self.tab = Tab::Peers,
            KeyCode::Char('3') => self.tab = Tab::Events,
            KeyCode::Char('4') => self.tab = Tab::Metrics,
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Char('r') => self.poll().await,
            KeyCode::Char('y') => self.sync().await,
            KeyCode::Char('g') => self.modal = Modal::ConfirmGc { force: false },
            KeyCode::Char('G') => self.modal = Modal::ConfirmGc { force: true },
            KeyCode::Char('p') => self.toggle_pin().await,
            KeyCode::Char('f') => self.follow().await,
            KeyCode::Char('s') => {
                if let Some(r) = self.selected_foreign() {
                    self.modal = Modal::SeedPrompt {
                        target: format!("{}/{}", r.summary.publisher, r.summary.name),
                        input: "latest".into(),
                    };
                }
            }
            KeyCode::Char('d') => {
                if let Some(r) = self.selected() {
                    let s = &r.summary;
                    self.modal =
                        Modal::ConfirmDelete(format!("{}/{}:{}", s.publisher, s.name, s.tag));
                }
            }
            _ => {}
        }
    }
}

/// Push a saturating per-poll delta, keeping at most `HIST` points.
fn push_delta(hist: &mut Vec<u64>, cur: u64, prev: u64) {
    hist.push(cur.saturating_sub(prev));
    if hist.len() > HIST {
        hist.drain(..hist.len() - HIST);
    }
}

fn clamp(state: &mut TableState, len: usize) {
    match (state.selected(), len) {
        (_, 0) => state.select(None),
        (None, _) => state.select(Some(0)),
        (Some(i), n) if i >= n => state.select(Some(n - 1)),
        _ => {}
    }
}

fn short(id: &impl ToString) -> String {
    let s = id.to_string();
    format!("{}…", &s[..s.len().min(10)])
}

fn human_bytes(b: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

// ---------------------------------------------------------------------------
// main loop
// ---------------------------------------------------------------------------

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, crossterm::cursor::Show);
}

fn setup_panic_hook() {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        prev(info);
    }));
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let paths = Paths::resolve(cli.home)?;
    let mut app = App::new(paths).await?;

    // Live events arrive over SSE in the background; the UI loop drains them.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<DaemonEvent>(256);
    tokio::spawn(listen_sse(app.client.clone(), tx));

    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen)?;
    setup_panic_hook();
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stdout()))?;

    let res = run(&mut terminal, &mut app, &mut rx).await;

    restore_terminal();
    res
}

/// Consume the daemon's `/_ocid/events` SSE stream, forwarding decoded
/// events; reconnects after a short pause if the daemon goes away.
async fn listen_sse(client: Client, tx: tokio::sync::mpsc::Sender<DaemonEvent>) {
    loop {
        if let Ok(mut resp) = client.events().await {
            // Buffer raw bytes: a chunk may split an SSE frame or a UTF-8 char.
            let mut buf: Vec<u8> = Vec::new();
            while let Ok(Some(chunk)) = resp.chunk().await {
                buf.extend_from_slice(&chunk);
                while let Some(pos) = buf.windows(2).position(|w| w == b"\n\n") {
                    let frame: Vec<u8> = buf.drain(..pos + 2).collect();
                    let Ok(text) = std::str::from_utf8(&frame) else {
                        continue;
                    };
                    for line in text.lines() {
                        let line = line.trim_start();
                        let Some(rest) = line.strip_prefix("data:") else {
                            continue;
                        };
                        let json = rest.strip_prefix(' ').unwrap_or(rest);
                        if let Ok(ev) = serde_json::from_str::<DaemonEvent>(json) {
                            if tx.send(ev).await.is_err() {
                                return; // UI gone
                            }
                        }
                    }
                }
            }
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

async fn run<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    rx: &mut tokio::sync::mpsc::Receiver<DaemonEvent>,
) -> Result<()> {
    while !app.quit {
        while let Ok(ev) = rx.try_recv() {
            app.on_event(ev);
        }
        if app.last_poll.elapsed() >= POLL {
            app.poll().await;
        }
        if let Modal::Notice(_, since) = app.modal {
            if since.elapsed() > NOTICE {
                app.modal = Modal::None;
            }
        }
        terminal.draw(|f| ui(f, app))?;
        if event::poll(TICK)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == event::KeyEventKind::Press {
                    app.on_key(key).await;
                }
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

fn panel(title: impl Into<String>, color: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
        .title(Span::styled(
            format!(" {} ", title.into()),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ))
}

fn ui(f: &mut Frame, app: &mut App) {
    // The Events and Metrics tabs use the full body, so the bottom ticker
    // collapses there.
    let ticker = match app.tab {
        Tab::Images | Tab::Peers => 8,
        Tab::Events | Tab::Metrics => 0,
    };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(ticker),
            Constraint::Length(1),
        ])
        .split(f.area());

    render_header(f, app, rows[0]);
    render_tabs(f, app, rows[1]);
    match app.tab {
        Tab::Images => render_images(f, app, rows[2]),
        Tab::Peers => render_peers(f, app, rows[2]),
        Tab::Events => render_events(f, app, rows[2], usize::MAX, "Live Daemon Events (SSE)"),
        Tab::Metrics => render_metrics(f, app, rows[2]),
    }
    if ticker > 0 {
        render_events(f, app, rows[3], 6, "Activity");
    }
    render_footer(f, rows[4]);
    render_modal(f, app);
}

fn render_header(f: &mut Frame, app: &App, area: Rect) {
    let kv = |k: &'static str, v: String, c: Color| {
        vec![
            Span::styled(format!(" {k} "), Style::default().fg(Color::DarkGray)),
            Span::styled(v, Style::default().fg(c).add_modifier(Modifier::BOLD)),
        ]
    };
    let mut spans = vec![
        Span::styled(
            " ocid ",
            Style::default()
                .bg(Color::Cyan)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(" {} ", app.listen),
            Style::default().fg(Color::Yellow),
        ),
    ];
    match &app.status {
        Some(s) => {
            spans.extend(kv("node", short(&s.id), Color::LightGreen));
            spans.extend(kv(
                "did",
                format!("{}…", &s.did[..s.did.len().min(16)]),
                Color::Gray,
            ));
            spans.extend(kv("peers", s.neighbors.len().to_string(), Color::Green));
            spans.extend(kv("releases", s.releases.to_string(), Color::Magenta));
            spans.extend(kv(
                "policy",
                format!("{}f/{}s/{}p", s.follows.len(), s.seeds.len(), s.pins.len()),
                Color::Cyan,
            ));
            spans.extend(kv("up", format!("{}m", s.uptime_secs / 60), Color::White));
            spans.extend(kv("v", s.version.clone(), Color::DarkGray));
        }
        None => spans.push(Span::styled(
            "  daemon not reachable — start `ocid` (retrying)",
            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        )),
    }
    let color = if app.status.is_some() {
        Color::Cyan
    } else {
        Color::Red
    };
    f.render_widget(
        Paragraph::new(Line::from(spans)).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(color)),
        ),
        area,
    );
}

fn render_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles = [
        ("1", "Images & Policy"),
        ("2", "Peers & Swarm"),
        ("3", "Events"),
        ("4", "Metrics"),
    ]
    .map(|(n, t)| {
        Line::from(vec![
            Span::styled(format!(" {n} "), Style::default().fg(Color::DarkGray)),
            Span::raw(t).bold(),
        ])
    });
    f.render_widget(
        Tabs::new(titles)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .select(app.tab as usize)
            .style(Style::default().fg(Color::Gray))
            .highlight_style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
        area,
    );
}

const ROW_HL: Style = Style::new().bg(Color::DarkGray).fg(Color::White);

fn render_images(f: &mut Frame, app: &mut App, area: Rect) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    let rows: Vec<Row> = app
        .releases
        .iter()
        .map(|r| {
            let s = &r.summary;
            let publisher = if r.mine {
                Span::styled("(me)", Style::default().fg(Color::Green))
            } else {
                Span::styled(short(&s.publisher), Style::default().fg(Color::Blue))
            };
            let rule = if r.mine {
                "own".to_string()
            } else if app.policy.is_pinned(&s.publisher, &s.name, &s.tag) {
                "pin".to_string()
            } else {
                app.policy.window(&s.publisher, &s.name).describe()
            };
            let rule_color = match rule.as_str() {
                "own" => Color::Green,
                "pin" => Color::Magenta,
                "-" => Color::DarkGray,
                _ => Color::Cyan,
            };
            let (done, done_color) = if r.complete {
                ("✓", Color::Green)
            } else {
                ("…", Color::Red)
            };
            Row::new(vec![
                publisher,
                Span::raw(s.name.clone()).bold(),
                Span::styled(s.tag.clone(), Style::default().fg(Color::LightYellow)),
                Span::styled(rule, Style::default().fg(rule_color)),
                Span::raw(human_bytes(r.size)),
                Span::styled(done, Style::default().fg(done_color)),
                Span::styled(
                    format!("{}…", &s.manifest_digest.hex()[..10]),
                    Style::default().fg(Color::DarkGray),
                ),
            ])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Min(14),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(10),
            Constraint::Length(4),
            Constraint::Length(12),
        ],
    )
    .header(
        Row::new([
            "PUBLISHER",
            "IMAGE",
            "TAG",
            "POLICY",
            "SIZE",
            "OK",
            "DIGEST",
        ])
        .style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(panel(
        format!("Images ({})", app.releases.len()),
        Color::LightBlue,
    ))
    .row_highlight_style(ROW_HL.add_modifier(Modifier::BOLD))
    .highlight_symbol("▶ ");
    f.render_stateful_widget(table, cols[0], &mut app.images);

    render_inspector(f, app, cols[1]);
}

fn render_inspector(f: &mut Frame, app: &App, area: Rect) {
    let label = |k: &'static str| Span::styled(k, Style::default().fg(Color::Cyan));
    let lines: Vec<Line> = match app.selected() {
        None => vec![Line::from(Span::styled(
            "nothing selected — push an image or follow a publisher",
            Style::default().fg(Color::DarkGray),
        ))],
        Some(r) => {
            let s = &r.summary;
            let reference = format!("{}/{}:{}", s.publisher, s.name, s.tag);
            let window = app.policy.window(&s.publisher, &s.name);
            let followed = app.policy.follow_mode(&s.publisher);
            let pinned = app.policy.is_pinned(&s.publisher, &s.name, &s.tag);
            vec![
                Line::from(vec![
                    Span::raw(s.name.clone()).bold(),
                    Span::raw(":"),
                    Span::styled(s.tag.clone(), Style::default().fg(Color::LightYellow)),
                    if r.mine {
                        Span::styled("  (own release)", Style::default().fg(Color::Green))
                    } else {
                        Span::raw("")
                    },
                ]),
                Line::from(""),
                Line::from(vec![
                    label("publisher  "),
                    Span::raw(s.publisher.to_string()),
                ]),
                Line::from(vec![
                    label("did        "),
                    Span::styled(did_key(&s.publisher), Style::default().fg(Color::DarkGray)),
                ]),
                Line::from(vec![
                    label("manifest   "),
                    Span::raw(s.manifest_digest.to_string()),
                ]),
                Line::from(vec![
                    label("size       "),
                    Span::raw(format!("{} in {} blob(s)", human_bytes(r.size), r.blobs)),
                ]),
                Line::from(vec![
                    label("state      "),
                    if r.complete {
                        Span::styled("complete, verified", Style::default().fg(Color::LightGreen))
                    } else {
                        Span::styled("incomplete (fetching)", Style::default().fg(Color::Red))
                    },
                ]),
                Line::from(""),
                Line::from(vec![
                    label("window     "),
                    Span::raw(window.describe()),
                    Span::styled(
                        match followed {
                            Some(m) => format!("   follow: {m}"),
                            None => String::new(),
                        },
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        if pinned { "   pinned" } else { "" },
                        Style::default().fg(Color::Magenta),
                    ),
                ]),
                Line::from(""),
                Line::from(label("pull")),
                Line::from(Span::styled(
                    format!(
                        " podman pull --tls-verify=false {}/{reference} ",
                        app.listen
                    ),
                    Style::default().fg(Color::White).bg(Color::DarkGray),
                )),
            ]
        }
    };
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("Inspector", Color::Magenta))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

fn render_peers(f: &mut Frame, app: &mut App, area: Rect) {
    let rows: Vec<Row> = app
        .peers
        .iter()
        .map(|p| {
            let (nb, nb_color) = if p.neighbor {
                ("✓ neighbor", Color::Green)
            } else if p.known {
                ("known", Color::Gray)
            } else {
                ("seen", Color::DarkGray)
            };
            Row::new(vec![
                Span::styled(p.id.to_string(), Style::default().fg(Color::Cyan)),
                Span::styled(did_key(&p.id), Style::default().fg(Color::DarkGray)),
                Span::styled(nb, Style::default().fg(nb_color)),
                Span::raw(
                    p.last_seen_secs
                        .map(|s| format!("{s}s ago"))
                        .unwrap_or_else(|| "-".into()),
                ),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(64),
            Constraint::Length(56),
            Constraint::Length(12),
            Constraint::Min(10),
        ],
    )
    .header(
        Row::new(["PEER (iroh endpoint id)", "DID", "STATE", "LAST SEEN"]).style(
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(panel(format!("Peers ({})", app.peers.len()), Color::Green))
    .row_highlight_style(ROW_HL)
    .highlight_symbol("▶ ");
    f.render_stateful_widget(table, area, &mut app.peer_rows);
}

/// btop-style metrics: sparklines of per-poll deltas on top, totals and
/// gauges below. Rates are derived client-side from `/_ocid/metrics`
/// snapshots, so no Prometheus text parsing is needed.
fn render_metrics(f: &mut Frame, app: &App, area: Rect) {
    let Some(m) = &app.metrics else {
        f.render_widget(
            Paragraph::new("waiting for /_ocid/metrics…").block(panel("Metrics", Color::Green)),
            area,
        );
        return;
    };
    let graphs = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(9), Constraint::Min(8)])
                .split(area)[0],
        );
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(8)])
        .split(area)[1];
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(body);

    spark(
        f,
        graphs[0],
        "HTTP req/poll",
        &app.hist_http,
        m.http_requests_total,
        Color::Cyan,
    );
    spark(
        f,
        graphs[1],
        "Bytes served/poll",
        &app.hist_bytes,
        m.http_bytes_served,
        Color::LightBlue,
    );
    spark(
        f,
        graphs[2],
        "Replicated+blobs/poll",
        &app.hist_repl,
        m.releases_replicated.saturating_add(m.blobs_fetched),
        Color::Green,
    );
    spark(
        f,
        graphs[3],
        "Announcements/poll",
        &app.hist_gossip,
        m.announcements_received
            .saturating_add(m.announcements_sent),
        Color::Magenta,
    );

    let rows = [
        ("http requests", m.http_requests_total.to_string()),
        ("bytes served", human_bytes(m.http_bytes_served)),
        ("bytes received", human_bytes(m.http_bytes_received)),
        ("published", m.releases_published.to_string()),
        ("replicated", m.releases_replicated.to_string()),
        ("repl. failed", m.releases_failed.to_string()),
        ("blobs fetched", m.blobs_fetched.to_string()),
        ("blobs bytes", human_bytes(m.blobs_fetched_bytes)),
        (
            "announce rx/tx",
            format!("{}/{}", m.announcements_received, m.announcements_sent),
        ),
        ("sync served", m.sync_requests_total.to_string()),
        ("mdns found", m.mdns_discovered.to_string()),
        (
            "gc runs/rel/blobs",
            format!(
                "{}/{}/{}",
                m.gc_runs, m.gc_releases_removed, m.gc_blobs_removed
            ),
        ),
        ("gc bytes freed", human_bytes(m.gc_bytes_freed)),
    ]
    .map(|(k, v)| {
        Row::new(vec![
            Span::styled(k, Style::default().fg(Color::DarkGray)),
            Span::raw(v),
        ])
    });
    f.render_widget(
        Table::new(rows, [Constraint::Length(18), Constraint::Min(10)])
            .header(
                Row::new(["COUNTER", "TOTAL"]).style(
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            )
            .block(panel("Totals", Color::Green)),
        cols[0],
    );

    let gauges = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Length(3),
            Constraint::Min(3),
        ])
        .split(cols[1]);
    gauge(f, gauges[0], "Neighbors", m.neighbors.max(0) as u64, 16);
    gauge(f, gauges[1], "Known peers", m.peers_known.max(0) as u64, 32);
    gauge(
        f,
        gauges[2],
        "Releases",
        m.releases.max(0) as u64,
        m.releases.max(0).max(10) as u64,
    );
    gauge(
        f,
        gauges[3],
        "Gossip topics",
        m.gossip_topics.max(0) as u64,
        m.gossip_topics.max(0).max(4) as u64,
    );
    f.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled("uptime  ", Style::default().fg(Color::Cyan)),
                Span::raw(format!("{}m", m.uptime_seconds / 60)),
            ]),
            Line::from(vec![
                Span::styled("policy  ", Style::default().fg(Color::Cyan)),
                Span::raw(format!("{}f/{}s", m.policy_follows, m.policy_seeds)),
            ]),
            Line::from(Span::styled(
                "source: GET /_ocid/metrics (JSON); /metrics stays OpenMetrics for Prometheus",
                Style::default().fg(Color::DarkGray),
            )),
        ])
        .block(panel("State", Color::Yellow)),
        gauges[4],
    );
}

/// One sparkline panel: history graph plus the running total.
fn spark(f: &mut Frame, area: Rect, title: &str, hist: &[u64], total: u64, color: Color) {
    f.render_widget(panel(format!("{title} ({total})"), color), area);
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(2), Constraint::Length(1)])
        .split(inner);
    f.render_widget(
        Sparkline::default()
            .data(hist)
            .style(Style::default().fg(color)),
        split[0],
    );
    let last = hist.last().copied().unwrap_or(0);
    f.render_widget(Paragraph::new(format!("last/poll {last}")), split[1]);
}

fn gauge(f: &mut Frame, area: Rect, label: &str, value: u64, max: u64) {
    let max = max.max(1);
    f.render_widget(
        Gauge::default()
            .block(Block::default().title(format!(" {label} ")))
            .gauge_style(Style::default().fg(Color::LightGreen))
            .ratio((value.min(max) as f64) / (max as f64))
            .label(format!("{value}/{max}")),
        area,
    );
}

fn render_events(f: &mut Frame, app: &App, area: Rect, take: usize, title: &str) {
    let items: Vec<ListItem> = app
        .log
        .iter()
        .take(take)
        .map(|e| {
            ListItem::new(Line::from(vec![
                Span::styled(&e.time, Style::default().fg(Color::DarkGray)),
                Span::raw(" "),
                Span::styled(
                    format!("{:<6}", e.kind),
                    Style::default().fg(e.color).add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                Span::raw(&e.msg),
            ]))
        })
        .collect();
    let items = if items.is_empty() {
        vec![ListItem::new(Span::styled(
            "waiting for events…",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        items
    };
    f.render_widget(List::new(items).block(panel(title, Color::Yellow)), area);
}

fn render_footer(f: &mut Frame, area: Rect) {
    let key = |k: &'static str, bg: Color, fg: Color| {
        Span::styled(
            format!(" {k} "),
            Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD),
        )
    };
    let hint = |t: &'static str| Span::raw(format!(" {t}  "));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            key("1-4/Tab", Color::DarkGray, Color::White),
            hint("tabs"),
            key("j/k", Color::DarkGray, Color::White),
            hint("select"),
            key("s", Color::Cyan, Color::Black),
            hint("seed"),
            key("f", Color::Blue, Color::White),
            hint("follow"),
            key("p", Color::Magenta, Color::White),
            hint("pin"),
            key("y", Color::LightGreen, Color::Black),
            hint("sync"),
            key("g/G", Color::Yellow, Color::Black),
            hint("gc/force"),
            key("d", Color::Red, Color::White),
            hint("delete"),
            key("r", Color::DarkGray, Color::White),
            hint("refresh"),
            key("q", Color::DarkGray, Color::White),
            hint("quit"),
        ])),
        area,
    );
}

fn render_modal(f: &mut Frame, app: &App) {
    let (title, color, body): (&str, Color, Vec<Line>) = match &app.modal {
        Modal::None => return,
        Modal::Notice(text, _) => ("Notice", Color::LightCyan, vec![Line::from(text.as_str())]),
        Modal::ConfirmGc { force } => (
            if *force {
                "Force garbage collection"
            } else {
                "Garbage collection"
            },
            Color::Yellow,
            vec![
                Line::from(if *force {
                    "Remove ALL unmanaged releases now, ignoring the grace period."
                } else {
                    "Remove unmanaged releases older than the grace period and orphaned blobs."
                }),
                Line::from("Own, pinned and in-window releases are never touched."),
                Line::from(""),
                confirm_line(Color::Yellow, Color::Black, "Y", "run"),
            ],
        ),
        Modal::ConfirmDelete(reference) => (
            "Delete release",
            Color::Red,
            vec![
                Line::from(reference.as_str()).bold(),
                Line::from(
                    "Removes the local record and unpins its blobs; not propagated to peers.",
                ),
                Line::from(""),
                confirm_line(Color::Red, Color::White, "Y", "delete"),
            ],
        ),
        Modal::SeedPrompt { target, input } => (
            "Seed image",
            Color::Cyan,
            vec![
                Line::from(target.as_str()).bold(),
                Line::from("retention mode: latest | last:N | full"),
                Line::from(""),
                Line::from(vec![
                    Span::styled("> ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        input.as_str(),
                        Style::default()
                            .fg(Color::Yellow)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("█"),
                ]),
                Line::from(""),
                confirm_line(Color::Cyan, Color::Black, "Enter", "save"),
            ],
        ),
    };
    let height = (body.len() as u16 + 4).min(f.area().height);
    let area = centered(70, height, f.area());
    f.render_widget(Clear, area);
    f.render_widget(
        Paragraph::new(body).alignment(Alignment::Center).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Double)
                .border_style(Style::default().fg(color))
                .title(Span::styled(
                    format!(" {title} "),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                )),
        ),
        area,
    );
}

fn confirm_line(bg: Color, fg: Color, key: &'static str, verb: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" [{key}] {verb} "),
            Style::default().bg(bg).fg(fg).add_modifier(Modifier::BOLD),
        ),
        Span::raw("   "),
        Span::styled(
            " [Esc/any] cancel ",
            Style::default().bg(Color::DarkGray).fg(Color::White),
        ),
    ])
}

/// Popup of `percent_x` width and fixed `height`, centred in `r`.
fn centered(percent_x: u16, height: u16, r: Rect) -> Rect {
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(r)[1];
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(v)[1]
}
