//! `cargo run --example sensors -- 20` prints the fan-loop inputs once a second (no admin needed).
#[path = "../src/sensors.rs"]
#[allow(dead_code)]
mod sensors;

fn main() {
    let secs: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let mut s = sensors::Sensors::new();
    for i in 0..secs {
        let gpus: Vec<String> = s.gpus().iter().map(|(h, t)| format!("{h:#x}={t:.1}")).collect();
        println!("{i:>3}s cpu_zone={:?} gpus=[{}]", s.cpu(), gpus.join(" "));
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}
