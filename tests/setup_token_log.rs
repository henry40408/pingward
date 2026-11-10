//! The setup token must reach the operator's stderr and nothing that gets shipped:
//! run the real binary and look for the token in everything else it prints.

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN_MARKER: &str = "Setup token (enter it on /setup): ";

#[test]
fn the_setup_token_is_printed_on_stderr_only() {
    let dir = std::env::temp_dir().join(format!("pingward_token_log_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_pingward"))
        .env(
            "DATABASE_URL",
            format!("sqlite://{}?mode=rwc", dir.join("t.sqlite3").display()),
        )
        .env("PINGWARD_BIND", format!("127.0.0.1:{port}"))
        .env("PINGWARD_SECRET", "pingward-test-secret-32-bytes-xx")
        .env("RUST_LOG", "trace")
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out_thread = std::thread::spawn(move || {
        let mut text = String::new();
        BufReader::new(stdout).read_to_string(&mut text).ok();
        text
    });
    let err_thread = std::thread::spawn(move || {
        let mut token = None;
        let mut rest = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some((_, announced)) = line.split_once(TOKEN_MARKER) {
                token = Some(announced.trim().to_owned());
                continue;
            }
            rest.push_str(&line);
            rest.push('\n');
        }
        (token, rest)
    });

    // Up once /healthz answers; by then every startup message has been written.
    let deadline = Instant::now() + Duration::from_secs(30);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "server did not start");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    child.kill().unwrap();
    child.wait().unwrap();

    let stdout_text = out_thread.join().unwrap();
    let (token, stderr_rest) = err_thread.join().unwrap();
    let _ = std::fs::remove_dir_all(&dir);

    let token = token.expect("the token is printed on stderr");
    assert_eq!(token.len(), 11, "{token}");
    assert!(!stdout_text.contains(&token), "token leaked to stdout logs");
    assert!(!stderr_rest.contains(&token), "token leaked to a log line");
}
