//! App-wide config. Real implementation is `Remaining work` — see
//! `claude/backend.md` §7.1.7. These are dev-time placeholders so the
//! boot sequence in `app.rs` compiles and runs against a local server.

pub fn ws_url() -> String {
    "ws://localhost:8080/ws".to_string()
}

pub fn auth_token() -> String {
    String::new()
}

#[allow(dead_code)]
pub fn api_base() -> String {
    "http://localhost:8080/api".to_string()
}
