//! Run the caption receiver on its own and print every line it takes, to check the browser extension without
//! starting Maru (no model, no history, no config file is read or written).
//!
//! ```text
//! cargo run -p maru-engine --example captions_listen -- 0123456789abcdef0123456789abcdef [port]
//! ```
//!
//! The first argument is the code the extension must send (at least 16 letters or digits); the port defaults to
//! 18096. Stop it with Ctrl+C.

use maru_core::captions::CaptionsConfig;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(code) = args.next() else {
        eprintln!("usage: captions_listen <code> [port]");
        std::process::exit(2);
    };
    let port = args.next().and_then(|p| p.parse().ok()).unwrap_or(CaptionsConfig::default().port);
    let cfg = CaptionsConfig { enabled: true, port, token: Some(code.into()), ..Default::default() };
    if let Some(problem) = cfg.problems().first() {
        eprintln!("{problem}");
        std::process::exit(2);
    }
    let mut server = match maru_engine::captions::listen(&cfg).await {
        Ok(server) => server,
        Err(why) => {
            eprintln!("{why}");
            std::process::exit(1);
        }
    };
    println!("listening on {} (lines are only printed here)", server.addr());
    while let Some(line) = server.next().await {
        let kind = if line.auto_generated { "auto-generated" } else { "uploaded" };
        println!("{:>8} ms  {kind}  {}  [{} · {}]", line.start_ms.map_or("?".into(), |ms| ms.to_string()), line.text, line.video, line.title);
    }
}
