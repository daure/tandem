use std::{
    fs::File,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
};

use super::*;

struct Tui {
    child: Child,
    terminal: Option<File>,
    output: String,
}

impl Tui {
    fn open(fixture: &Fixture) -> Self {
        let (mut master, mut slave) = (-1, -1);
        let size = libc::winsize {
            ws_row: 40,
            ws_col: 140,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    &size,
                )
            },
            0
        );
        let terminal = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        assert_ne!(
            unsafe { libc::fcntl(master, libc::F_SETFL, libc::O_NONBLOCK) },
            -1
        );
        assert_ne!(
            unsafe { libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC) },
            -1
        );
        let mut command = fixture.command(&[]);
        command
            .env("BLOCK_CONFIG", "1")
            .env("TERM", "xterm-256color")
            .env("XDG_CONFIG_HOME", fixture.home.join("config"))
            .stdin(slave.try_clone().unwrap())
            .stdout(slave.try_clone().unwrap())
            .stderr(slave);
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 || libc::ioctl(0, libc::TIOCSCTTY, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        Self {
            child: command.spawn().unwrap(),
            terminal: Some(terminal),
            output: String::new(),
        }
    }

    fn drain(&mut self) {
        let mut bytes = [0; 16384];
        while let Ok(count) = self.terminal.as_mut().unwrap().read(&mut bytes) {
            if count == 0 {
                break;
            }
            self.output
                .push_str(&String::from_utf8_lossy(&bytes[..count]));
        }
    }

    fn wait_text(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            self.drain();
            if self.output.contains(text) {
                return;
            }
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "TUI exited: {}",
                self.output
            );
            assert!(
                Instant::now() < deadline,
                "TUI did not show {text}: {}",
                self.output
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn keys(&mut self, keys: &[u8]) {
        self.output.clear();
        self.terminal.as_mut().unwrap().write_all(keys).unwrap();
    }

    fn close(&mut self, mode: &str) {
        match mode {
            "quit" => self.keys(b"\x11"),
            "pane" => {
                self.terminal.take();
            }
            "kill" => {
                self.child.kill().unwrap();
            }
            _ => unreachable!(),
        }
        wait_until(|| {
            if self.terminal.is_some() {
                self.drain();
            }
            self.child.try_wait().unwrap().is_some()
        });
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn tui_quit_terminal_closure_and_kill_preserve_startup_and_reopened_status() {
    for mode in ["quit", "pane", "kill"] {
        let fixture = Fixture::new();
        rusqlite::Connection::open(fixture.home.join("settings.sqlite3"))
            .unwrap()
            .execute(
                "INSERT INTO app_settings(key,value) VALUES ('integrations.opencode','false')",
                [],
            )
            .unwrap();
        let mut tui = Tui::open(&fixture);
        tui.wait_text("Instances");
        tui.keys(b"A");
        tui.wait_text("website");
        tui.keys(b"i");
        tui.wait_text("New instance");
        tui.keys(b"review\x1b[13;5u");
        wait_until(|| {
            tui.drain();
            fixture.home.join("configured").exists()
        });
        tui.close(mode);

        let mut reopened = Tui::open(&fixture);
        reopened.wait_text("review");
        reopened.wait_text("Creating");
        assert_ne!(
            unsafe { libc::tcgetsid(reopened.terminal.as_ref().unwrap().as_raw_fd()) },
            -1
        );
        fs::write(fixture.home.join("release-config"), "").unwrap();
        let record_path = fixture.home.join("runtime/cli-test/review.startup.json");
        wait_until(|| {
            reopened.drain();
            let record: Value =
                serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
            record["operation"]["state"] == "succeeded"
        });
        reopened.close("quit");
    }
}
