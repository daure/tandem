use super::*;
use std::{
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) fn install(home: &Path, bin: &Path) {
    for (name, contents) in [("zellij", ZELLIJ), ("opencode", OPENCODE)] {
        fs::write(bin.join(name), contents).unwrap();
        fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::write(
        home.join("panes.json"),
        json!([
            {"id":100,"is_plugin":false,"exited":false,"tab_id":9,"tab_name":"review"}
        ])
        .to_string(),
    )
    .unwrap();
}

#[test]
fn blank_sessions_clear_inherited_prompts() {
    let fixture = Fixture::new();
    let output = wait_output(
        fixture
            .command(&["new-instance", "review", "-t", "website", "-o"])
            .env("TANDEM_INITIAL_PROMPT", "inherited prompt must not run")
            .spawn()
            .unwrap(),
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.home.join("opened-parameters")).unwrap(),
        ""
    );
    let calls = fs::read_to_string(fixture.home.join("zellij-calls")).unwrap();
    assert!(calls.contains("new-tab --name review"), "{calls}");
    assert!(calls.contains("focus-pane-id terminal_100"), "{calls}");
}

#[test]
fn opencode_requires_zellij_and_enabled_integration_before_provisioning() {
    for disabled in [false, true] {
        let fixture = Fixture::new();
        if disabled {
            rusqlite::Connection::open(fixture.home.join("settings.sqlite3")).unwrap()
                .execute("INSERT INTO app_settings(key, value) VALUES ('integrations.opencode', 'false')", []).unwrap();
        }
        let mut command = fixture.command(&["new-instance", "review", "-t", "website", "-o"]);
        if !disabled {
            command.env_remove("ZELLIJ_SESSION_NAME");
        }
        let output = wait_output(command.spawn().unwrap());
        assert!(!output.status.success());
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(
            error.contains(if disabled {
                "integration is disabled"
            } else {
                "inside Zellij"
            }),
            "{error}"
        );
        assert!(!fixture.home.join("workspaces/review").exists());
        assert!(!fixture.home.join("opened").exists());
    }
}

#[test]
fn a_fresh_cli_reuses_the_shared_server_and_stacks_in_the_observed_instance_tab() {
    let fixture = Fixture::new();
    let server = Server::start();
    let workspace = fixture.home.join("workspaces/review");
    let presence = fixture.home.join("state/tandem/opencode");
    fs::create_dir_all(&presence).unwrap();
    fs::write(
        presence.join("client.json"),
        json!({
            "pid": std::process::id(),
            "observed_at": SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis(),
            "id": "", "title": "OpenCode", "activity": "idle",
            "directory": workspace, "server": server.url,
            "zellij_session": "main", "pane_id": 7
        })
        .to_string(),
    )
    .unwrap();
    fs::write(
        fixture.home.join("panes.json"),
        json!([
            {"id":7,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"review"},
            {"id":100,"is_plugin":false,"exited":false,"tab_id":4,"tab_name":"review"}
        ])
        .to_string(),
    )
    .unwrap();
    let prompt = "Explain 'this'; $(touch injected)\nnext line";
    let output = fixture.run(&["new-instance", "review", "-t", "website", "-o", prompt]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = fs::read_to_string(fixture.home.join("zellij-calls")).unwrap();
    assert!(
        calls
            .lines()
            .any(|line| line.contains("new-pane --stacked") && line.contains("--tab-id 4")),
        "{calls}"
    );
    assert!(calls.contains("focus-pane-id terminal_100"), "{calls}");
    assert!(!calls.contains("new-tab"), "{calls}");
    let args = fs::read_to_string(fixture.home.join("opencode-args")).unwrap();
    assert_eq!(
        args.split_terminator('\0').collect::<Vec<_>>(),
        ["attach", &server.url, "--dir", workspace.to_str().unwrap()]
    );
    assert_eq!(
        fs::read_to_string(fixture.home.join("opened-parameters")).unwrap(),
        prompt
    );
    assert!(!workspace.join("injected").exists());
}

struct Server {
    url: String,
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let thread = thread::spawn(move || {
            while !stopping.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(5));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = String::new();
                BufReader::new(stream.try_clone().unwrap())
                    .read_line(&mut request)
                    .unwrap();
                let body = if request.contains("/experimental/session") {
                    "[]"
                } else {
                    "{}"
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            url,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

const ZELLIJ: &str = r#"#!/bin/sh
printf '%s\n' "$*" >> "$TANDEM_HOME/zellij-calls"
case "$*" in
  list-sessions*) printf 'main\n';;
  *list-panes*) cat "$TANDEM_HOME/panes.json";;
  *new-tab*|*new-pane*)
    if [ "$FAIL_OPENCODE" = 1 ]; then printf 'fixture OpenCode launch failed\n' >&2; exit 23; fi
    kind=tab
    for arg in "$@"; do if [ "$arg" = new-pane ]; then kind=pane; fi; done
    while [ "$#" -gt 0 ]; do
      case "$1" in
        --cwd) shift; directory=$1;;
        --) shift; break;;
      esac
      shift
    done
    cd "$directory" || exit 24
    if [ "$BLOCK_OPEN" = 1 ]; then
      touch "$TANDEM_HOME/opening"
      count=0
      while [ ! -f "$TANDEM_HOME/release-open" ]; do
        count=$((count + 1))
        if [ "$count" -gt 200 ]; then exit 26; fi
        sleep 0.05
      done
    fi
    "$@" || exit 25
    if [ "$kind" = tab ]; then printf '9\n'; else printf 'terminal_100\n'; fi
    ;;
esac
"#;

const OPENCODE: &str = r#"#!/bin/sh
if [ "$1" = --version ]; then printf '1.18.29\n'; exit 0; fi
if [ "$EXPECT_PREPARED_GUIDANCE" = 1 ]; then
  test ! -e app || exit 27
  test -f AGENTS.md || exit 28
  test -f seed.txt || exit 30
  grep -q 'Template workflow' AGENTS.md || exit 31
  grep -q './app/AGENTS.md' AGENTS.md || exit 32
  test ! -e "$TANDEM_HOME/started" || exit 33
fi
if [ "$EDIT_GUIDANCE" = 1 ]; then printf 'Client guidance\n' > AGENTS.md; fi
if [ -f app/file.txt ]; then
  git -C app branch --show-current > "$TANDEM_HOME/branch"
fi
printf '%s' "$TANDEM_INITIAL_PROMPT" > "$TANDEM_HOME/opened-parameters"
printf '%s\0' "$@" > "$TANDEM_HOME/opencode-args"
printf '%s\n' "$PWD" >> "$TANDEM_HOME/opened"
"#;
