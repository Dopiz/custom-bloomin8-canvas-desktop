//! Standalone MockDevice process for manual verification and E2E use.
//!
//! Not part of the shipped app — build/run it explicitly:
//!
//! ```sh
//! cargo run --features mock-device --bin mockdevice -- --port 18080
//! ```
//!
//! Point the Device page's LAN IP field at the printed base URL's
//! `host:port` (e.g. `127.0.0.1:18080`) to drive the real UI against a fake
//! device without hardware.

use bloomin8_desktop_lib::device::MockDevice;

const DEFAULT_PORT: u16 = 18080;

#[tokio::main]
async fn main() {
    let port = parse_port_arg().unwrap_or(DEFAULT_PORT);
    let addr = format!("127.0.0.1:{port}");

    let mock = MockDevice::start_at(&addr).await;
    println!("MockDevice listening at {}", mock.base_url());
    println!("Point the Device page's LAN IP field at: {}", mock.base_url());
    println!("(Ctrl-C to stop)");

    tokio::signal::ctrl_c()
        .await
        .expect("failed to listen for ctrl-c");
    println!("shutting down mockdevice");
}

/// Parses `--port <N>` from argv; falls back to `DEFAULT_PORT` if absent or
/// unparsable.
fn parse_port_arg() -> Option<u16> {
    let args: Vec<String> = std::env::args().collect();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--port" {
            return iter.next().and_then(|v| v.parse().ok());
        }
    }
    None
}
