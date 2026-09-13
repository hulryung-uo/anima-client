//! Real loopback HTTP requests; no game server, game files or saved credentials.
use super::*;
use std::io::{Read, Write};
use std::net::TcpStream;

fn request(server: &PlayServer, path: &str, headers: &str, body: &str) -> u16 {
    let mut socket = TcpStream::connect(("127.0.0.1", server.port())).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(socket, "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}", server.port(), body.len()).unwrap();
    let mut response = String::new();
    socket.read_to_string(&mut response).unwrap();
    response.split_whitespace().nth(1).unwrap().parse().unwrap()
}

fn prompt(server: &PlayServer, id: &str) {
    *server.scene.lock().unwrap() = serde_json::json!({
        "auth": "characters", "choice_id": id, "slots": [{"index": 0, "name": "Fixture"}]
    })
    .to_string();
}

#[test]
fn http_rejects_stale_session_input_and_character_actions() {
    let server = bind(PlayConfig {
        host: "127.0.0.1".into(),
        port: 1,
        user: String::new(),
        pass: String::new(),
        shard: 0,
        http_port: 0,
        web_dir: None,
        data_dir: std::env::temp_dir().join(crate::connection::fresh_context_id()),
        login_page: true,
        bind_addr: "127.0.0.1".into(),
        read_only: false,
    })
    .unwrap(); // Intentionally never call run(): only its HTTP side is alive.

    assert_eq!(request(&server, "/input", "", "stop"), 409);
    let old = server.input.activate("old");
    assert_eq!(request(&server, "/input", "", "walk:2:1"), 409);
    assert_eq!(
        request(&server, "/input", "X-Anima-Session: wrong\r\n", "stop"),
        409
    );
    assert!(server.rx.try_recv().is_err());
    assert_eq!(
        request(&server, "/input", "X-Anima-Session: old\r\n", "walk:2:1"),
        200
    );
    drop(old);
    let active = server.input.activate("new");
    assert!(server.rx.try_recv().unwrap().for_session("new").is_none());
    assert_eq!(
        request(&server, "/input", "X-Anima-Session: old\r\n", "walk:2:1"),
        409
    );
    assert_eq!(
        request(
            &server,
            "/input",
            "X-Anima-Session: new\r\nOrigin: https://example.test\r\n",
            "stop"
        ),
        403
    );
    assert_eq!(
        request(&server, "/input", "X-Anima-Session: new\r\n", "walk:5:0"),
        200
    );
    assert!(matches!(
        server.rx.try_recv().unwrap().for_session("new"),
        Some(Some(Action::Walk { dir: 5, run: false }))
    ));
    drop(active);
    assert_eq!(
        request(&server, "/input", "X-Anima-Session: new\r\n", "stop"),
        409
    );

    prompt(&server, "first");
    assert_eq!(request(&server, "/character", "", r#"{"slot":0}"#), 400);
    assert_eq!(
        request(
            &server,
            "/character",
            "",
            r#"{"choice_id":"stale","slot":0}"#
        ),
        409
    );
    assert!(server.character_rx.try_recv().is_err());
    assert_eq!(
        request(
            &server,
            "/character",
            "",
            r#"{"choice_id":"first","slot":0}"#
        ),
        200
    );
    assert_eq!(
        request(
            &server,
            "/character",
            "",
            r#"{"choice_id":"first","slot":0}"#
        ),
        409
    );
    // Even a previously accepted request must not reach the next account/prompt.
    prompt(&server, "second");
    assert!(server
        .character_rx
        .try_recv()
        .unwrap()
        .for_prompt("second")
        .is_none());
    for action in [
        r#""delete_slot":0"#,
        r#""cancel":true"#,
        r#""create":{"name":"New Hero","female":false,"profession":"warrior","strength":60,"dexterity":20,"intelligence":10,"city_index":0}"#,
    ] {
        assert_eq!(
            request(
                &server,
                "/character",
                "",
                &format!(r#"{{"choice_id":"first",{action}}}"#)
            ),
            409
        );
        assert!(server.character_rx.try_recv().is_err());
    }
    for (id, action) in [
        ("second", r#""delete_slot":0"#),
        ("third", r#""cancel":true"#),
        (
            "fourth",
            r#""create":{"name":"New Hero","female":false,"profession":"warrior","strength":60,"dexterity":20,"intelligence":10,"city_index":0}"#,
        ),
    ] {
        prompt(&server, id);
        assert_eq!(
            request(
                &server,
                "/character",
                "",
                &format!(r#"{{"choice_id":"{id}",{action}}}"#)
            ),
            200
        );
        let accepted = server
            .character_rx
            .try_recv()
            .unwrap()
            .for_prompt(id)
            .unwrap();
        match id {
            "second" => assert!(matches!(
                accepted,
                CharacterDecision::Choose(CharacterChoice::Delete(0))
            )),
            "third" => assert!(matches!(accepted, CharacterDecision::Cancel)),
            _ => assert!(matches!(
                accepted,
                CharacterDecision::Choose(CharacterChoice::Create(_))
            )),
        }
    }

    let old = LoginControl::default();
    let current = LoginControl::default();
    *server.active_login.lock().unwrap() = Some(current.clone());
    let old_cancel = serde_json::json!({"attempt_id": old.id()}).to_string();
    let current_cancel = serde_json::json!({"attempt_id": current.id()}).to_string();
    assert_eq!(
        request(
            &server,
            "/login/cancel",
            "X-Anima-Launcher: 1\r\n",
            &old_cancel
        ),
        409
    );
    assert!(!current.is_cancelled());
    assert_eq!(
        request(
            &server,
            "/login/cancel",
            "X-Anima-Launcher: 1\r\n",
            &current_cancel
        ),
        200
    );
    assert!(current.is_cancelled());
}

#[test]
fn http_one_time_login_never_saves_or_borrows_another_endpoints_password() {
    use crate::launcher::PasswordVault;
    use std::collections::HashMap;

    #[derive(Default)]
    struct Vault {
        values: Mutex<HashMap<String, String>>,
        reads: AtomicU64,
        writes: AtomicU64,
    }
    impl PasswordVault for Vault {
        fn get(&self, key: &str) -> Result<Option<String>, String> {
            self.reads.fetch_add(1, Ordering::Relaxed);
            Ok(self.values.lock().unwrap().get(key).cloned())
        }
        fn set(&self, key: &str, value: &str) -> Result<(), String> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            self.values.lock().unwrap().insert(key.into(), value.into());
            Ok(())
        }
        fn delete(&self, key: &str) -> Result<(), String> {
            self.writes.fetch_add(1, Ordering::Relaxed);
            self.values.lock().unwrap().remove(key);
            Ok(())
        }
    }
    let directory = std::env::temp_dir().join(crate::connection::fresh_context_id());
    std::fs::create_dir(&directory).unwrap();
    let file = directory.join("launcher.json");
    let vault = Arc::new(Vault::default());
    let launcher = Arc::new(LauncherStore::open(file.clone(), Some(vault.clone())).unwrap());
    launcher.command(&serde_json::json!({"op":"save_server","id":"world","name":"Fixture","host":"127.0.0.1","port":25111,"shard":0,"notes":""})).unwrap();
    launcher.command(&serde_json::json!({"op":"save_account","id":"saved","server_id":"world","label":"Fixture","username":"player","password":"stored-fixture-secret","remember_password":true})).unwrap();
    let before = std::fs::read(&file).unwrap();
    let writes = vault.writes.load(Ordering::Relaxed);
    vault.reads.store(0, Ordering::Relaxed);
    let server = bind_with_launcher(
        PlayConfig {
            host: "127.0.0.1".into(),
            port: 1,
            user: String::new(),
            pass: String::new(),
            shard: 0,
            http_port: 0,
            web_dir: None,
            data_dir: directory.clone(),
            login_page: true,
            bind_addr: "127.0.0.1".into(),
            read_only: false,
        },
        launcher.clone(),
    )
    .unwrap(); // Only HTTP admission is running; no shard is contacted.

    for (id, typed, expected_port, expected_password, reads) in [
        (None, "one-time-fixture", 25112, "one-time-fixture", 0),
        (None, "", 25112, "", 0),
        (
            Some("saved"),
            "temporary-fixture",
            25111,
            "temporary-fixture",
            0,
        ),
        (Some("saved"), "", 25111, "stored-fixture-secret", 1),
    ] {
        *server.scene.lock().unwrap() = serde_json::json!({"auth":"login"}).to_string();
        let body = serde_json::json!({"account_id":id,"host":"127.0.0.1","port":25112,"shard":0,"username":"player","password":typed,"interactive":true}).to_string();
        assert_eq!(
            request(&server, "/login", "X-Anima-Launcher: 1\r\n", &body),
            200
        );
        let (attempt, _) = server.login_rx.try_recv().unwrap();
        assert_eq!(attempt.account_id.as_deref(), id);
        assert_eq!(attempt.port, expected_port);
        assert_eq!(attempt.username, "player");
        assert_eq!(attempt.password, expected_password);
        assert_eq!(vault.reads.load(Ordering::Relaxed), reads);
        assert_eq!(vault.writes.load(Ordering::Relaxed), writes);
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }
    assert_eq!(
        launcher.resolve_login("saved", "").unwrap().password,
        "stored-fixture-secret"
    );
    std::fs::remove_dir_all(directory).unwrap();
}
