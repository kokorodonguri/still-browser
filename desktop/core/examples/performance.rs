//! Repeatable microbenchmark, not a timing assertion. Run with:
//! cargo run -p still-core --example performance --release --offline
use std::hint::black_box;
use std::time::Instant;
use still_core::session::{Bounds, Session};

fn main() {
    println!("tabs,iterations,switch_snapshot_ns,title_snapshot_ns,update_bytes");
    for count in [10, 1_000, 10_000] {
        let tabs: Vec<_> = (0..count).map(|i| serde_json::json!({"Url": format!("https://example.org/{i}")})).collect();
        let json = serde_json::json!({"Tabs": tabs}).to_string();
        let mut session = Session::load(Some(Ok(json)), &[]).0;
        session.set_bounds(Bounds { x: 8.0, y: 48.0, width: 1000.0, height: 700.0 });
        black_box(session.snapshot(false));
        let iterations = 2_000;
        let start = Instant::now();
        for _ in 0..iterations {
            session.cycle(false);
            black_box(session.plan_show(false));
            black_box(session.snapshot(false));
        }
        let switch_ns = start.elapsed().as_nanos() / iterations;
        let id = session.active();
        let start = Instant::now();
        for i in 0..iterations {
            session.title_changed(id, if i % 2 == 0 { "Title A" } else { "Title B" });
            black_box(session.snapshot(false));
        }
        let title_ns = start.elapsed().as_nanos() / iterations;
        session.title_changed(id, "Measured title");
        let bytes = serde_json::to_vec(&session.snapshot(false)).unwrap().len();
        println!("{count},{iterations},{switch_ns},{title_ns},{bytes}");
    }
}
