//! One-click demo launcher. Starts a private PostgreSQL (bundled next to this program), loads the
//! sample company on first run, serves the ERP on localhost and opens the browser.
//! Layout next to the program (or ../share/weft for AppImage): pg/  web/  demo.sql
use std::{path::{Path, PathBuf}, process::{Command, Stdio}, sync::Arc, time::Duration};
use weft_erp::{build_app, config::Config, state::AppState, MIGRATOR};

fn resources() -> PathBuf {
    if let Ok(p) = std::env::var("WEFT_HOME") { return PathBuf::from(p); }
    let exe = std::env::current_exe().expect("cannot locate program");
    let dir = exe.parent().unwrap().to_path_buf();
    for cand in [dir.clone(), dir.join("../share/weft")] {
        if cand.join("pg").is_dir() && cand.join("web").is_dir() { return cand; }
    }
    die(&format!("Demo files not found next to {}", dir.display()));
}

fn die(msg: &str) -> ! {
    eprintln!("\nERROR: {msg}\n");
    if cfg!(windows) { eprintln!("Press Enter to close."); let _ = std::io::stdin().read_line(&mut String::new()); }
    std::process::exit(1);
}

fn exe(base: &Path, name: &str) -> PathBuf {
    base.join("pg/bin").join(if cfg!(windows) { format!("{name}.exe") } else { name.to_string() })
}

fn run(cmd: &mut Command, what: &str) {
    let out = cmd.stdin(Stdio::null()).output().unwrap_or_else(|e| die(&format!("{what}: {e}")));
    if !out.status.success() {
        die(&format!("{what} failed:\n{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr)));
    }
}

fn free_port(preferred: u16) -> u16 {
    if std::net::TcpListener::bind(("127.0.0.1", preferred)).is_ok() { return preferred; }
    std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port()
}

#[tokio::main]
async fn main() {
    let res = resources();
    let home = dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("WEFT-ERP-Demo");
    let data = home.join("pgdata");
    if std::env::args().any(|a| a == "--reset") { let _ = std::fs::remove_dir_all(&home); println!("Demo data cleared."); }
    std::fs::create_dir_all(&home).unwrap();
    println!("WEFT ERP — client demo (sample data only)\n");

    let pg_ctl = exe(&res, "pg_ctl");
    // A previous run closed abruptly may have left the database running.
    if data.join("PG_VERSION").exists() {
        let _ = Command::new(&pg_ctl).args(["-D", data.to_str().unwrap(), "-m", "immediate", "stop"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    }
    let fresh = !data.join("PG_VERSION").exists();
    if fresh {
        println!("First start: preparing the demo database (about a minute)...");
        run(Command::new(exe(&res, "initdb")).args(["-D", data.to_str().unwrap(), "-U", "postgres", "--auth=trust", "-E", "UTF8", "--locale=C"]), "initdb");
    }
    let pg_port = free_port(54329);
    let mut opts = format!("-p {pg_port} -c listen_addresses=127.0.0.1 -c fsync=off -c synchronous_commit=off");
    // Distribution builds of PostgreSQL default to a system socket folder we may not write to.
    if cfg!(unix) { opts.push_str(" -c unix_socket_directories=/tmp"); }
    run(Command::new(&pg_ctl).args(["-D", data.to_str().unwrap(), "-o", &opts, "-l", home.join("postgres.log").to_str().unwrap(), "-w", "-t", "60", "start"]), "starting PostgreSQL");
    let port_s = pg_port.to_string();
    if fresh {
        run(Command::new(exe(&res, "createdb")).args(["-h", "127.0.0.1", "-p", &port_s, "-U", "postgres", "erp"]), "createdb");
        run(Command::new(exe(&res, "psql")).args(["-h", "127.0.0.1", "-p", &port_s, "-U", "postgres", "-d", "erp", "-v", "ON_ERROR_STOP=1", "-q", "-f", res.join("demo.sql").to_str().unwrap()]), "loading sample data");
    }

    let app_port = free_port(8080);
    let secret = { let b: [u8; 32] = rand::random(); hex::encode(b) };
    let cfg = Config {
        database_url: format!("postgres://postgres@127.0.0.1:{pg_port}/erp"),
        bind_addr: format!("127.0.0.1:{app_port}"),
        jwt_secret: secret,
        access_ttl_minutes: 240,
        refresh_ttl_days: 1,
        cors_origins: vec![],
        run_jobs: true,
        static_dir: Some(res.join("web").to_string_lossy().into_owned()),
    };
    let db = sqlx::postgres::PgPoolOptions::new().max_connections(10).connect(&cfg.database_url).await.unwrap_or_else(|e| die(&format!("cannot connect to the demo database: {e}")));
    MIGRATOR.run(&db).await.unwrap_or_else(|e| die(&format!("database migration failed: {e}")));
    let state = AppState { db, cfg: Arc::new(cfg.clone()) };
    weft_erp::modules::jobs::spawn(state.clone());
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind_addr).await.unwrap_or_else(|e| die(&format!("cannot start the web server: {e}")));

    let url = format!("http://localhost:{app_port}");
    println!("\n  WEFT ERP demo is running:  {url}");
    println!("  Sign in:  owner@meghna.example  /  Demo@12345");
    println!("  (other roles: sales@ store@ hr@ meghna.example, same password)");
    println!("\n  Keep this window open during the demo. Close it, or press Ctrl+C, to stop.\n");
    let u = url.clone();
    if std::env::var_os("WEFT_NO_BROWSER").is_none() {
        tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(600)).await; let _ = webbrowser::open(&u); });
    }

    axum::serve(listener, app).with_graceful_shutdown(shutdown()).await.ok();
    println!("Stopping...");
    let _ = Command::new(&pg_ctl).args(["-D", data.to_str().unwrap(), "-m", "fast", "-w", "stop"]).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

async fn shutdown() {
    #[cfg(windows)]
    {
        use tokio::signal::windows;
        let mut c = windows::ctrl_c().unwrap();
        let mut close = windows::ctrl_close().unwrap();
        let mut brk = windows::ctrl_break().unwrap();
        tokio::select! { _ = c.recv() => {}, _ = close.recv() => {}, _ = brk.recv() => {} }
    }
    #[cfg(not(windows))]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut t = signal(SignalKind::terminate()).unwrap();
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = t.recv() => {} }
    }
}
