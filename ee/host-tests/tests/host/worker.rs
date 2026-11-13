//! The proof host under `wrangler dev --local`: built by wrangler's build command, with its D1
//! migrations applied and every setting passed as `--var`, against the fake GitHub. One runs
//! for all the tests and is stopped after the last.

use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The audience CI asks its OIDC token for: the host's origin in production.
pub const AUDIENCE: &str = "https://proof.test";
/// The issuer of Actions OIDC tokens.
pub const ISSUER: &str = "https://token.actions.githubusercontent.com";
pub const INSTALL_URL: &str = "https://github.com/apps/fearlezz/installations/new";

pub struct Worker {
    pub url: String,
    /// wrangler, the leader of a process group that holds workerd too.
    child: Mutex<Child>,
    state: tempfile::TempDir,
}

impl Worker {
    pub fn start(github: &str, app_key_pem: &str) -> Worker {
        let host = Path::new(env!("CARGO_MANIFEST_DIR")).join("../host");
        let state = tempfile::tempdir().unwrap();
        let persist = state.path().to_str().unwrap().to_owned();
        let migrated = wrangler(&host)
            .args([
                "d1",
                "migrations",
                "apply",
                "DB",
                "--local",
                "--persist-to",
                &persist,
            ])
            .output()
            .unwrap();
        assert!(
            migrated.status.success(),
            "applying the D1 migrations failed:\n{}{}",
            String::from_utf8_lossy(&migrated.stdout),
            String::from_utf8_lossy(&migrated.stderr)
        );

        let port = free_port();
        let vars = [
            ("AUDIENCE", AUDIENCE.to_owned()),
            ("OIDC_ISSUER", ISSUER.to_owned()),
            ("OIDC_JWKS", format!("{github}/.well-known/jwks")),
            ("GITHUB_API", github.to_owned()),
            ("GITHUB_WEB", github.to_owned()),
            ("GITHUB_CLIENT_ID", crate::github::CLIENT_ID.to_owned()),
            ("INSTALL_URL", INSTALL_URL.to_owned()),
            (
                "GITHUB_CLIENT_SECRET",
                crate::github::CLIENT_SECRET.to_owned(),
            ),
            ("GITHUB_APP_KEY", app_key_pem.to_owned()),
            (
                "SESSION_KEY",
                "a session key only the tests know".to_owned(),
            ),
        ];
        let mut cmd = wrangler(&host);
        cmd.args([
            "dev",
            "--local",
            "--ip",
            "127.0.0.1",
            "--persist-to",
            &persist,
        ])
        .args(["--port", &port.to_string()])
        .args(["--inspector-port", &free_port().to_string()]);
        for (name, value) in vars {
            cmd.arg("--var").arg(format!("{name}:{value}"));
        }
        let log = std::fs::File::create(state.path().join("wrangler.log")).unwrap();
        let child = cmd
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap();
        let worker = Worker {
            url: format!("http://127.0.0.1:{port}"),
            child: Mutex::new(child),
            state,
        };
        worker.wait_ready();
        worker
    }

    /// Waits for the first answer; the first run builds the worker, which takes a while.
    fn wait_ready(&self) {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(600) {
            if crate::agent().get(&format!("{}/", self.url)).call().is_ok() {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        self.stop();
        panic!(
            "wrangler dev did not answer within 10 minutes:\n{}",
            self.log()
        );
    }

    pub fn log(&self) -> String {
        std::fs::read_to_string(self.state.path().join("wrangler.log")).unwrap_or_default()
    }

    /// Stops wrangler and the workerd it runs, which share its process group.
    pub fn stop(&self) {
        let mut child = self.child.lock().unwrap();
        let _ = Command::new("kill")
            .args(["-TERM", &format!("-{}", child.id())])
            .status();
        let _ = child.wait();
    }
}

fn wrangler(host: &Path) -> Command {
    let mut cmd = Command::new("wrangler");
    cmd.current_dir(host)
        .env("WRANGLER_SEND_METRICS", "false")
        .env("CI", "true")
        .stdin(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }
    cmd
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
