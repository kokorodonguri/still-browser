//! Heap measurements for the Rust side of Still (the WebView2 engine is not
//! included: it runs in its own processes). A counting allocator records live
//! bytes, the peak and the number of allocations for each scenario. Run with:
//! cargo run -p still-core --example memory --release --offline
use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use still_core::session::{Bounds, Session};

struct Counting;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let live = LIVE.fetch_add(layout.size(), Relaxed) + layout.size();
        PEAK.fetch_max(live, Relaxed);
        COUNT.fetch_add(1, Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if new_size > layout.size() {
            let live = LIVE.fetch_add(new_size - layout.size(), Relaxed) + new_size - layout.size();
            PEAK.fetch_max(live, Relaxed);
        } else {
            LIVE.fetch_sub(layout.size() - new_size, Relaxed);
        }
        COUNT.fetch_add(1, Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Live bytes before, peak above that baseline, live bytes after, allocations.
struct Measure {
    start: usize,
    count: usize,
}

impl Measure {
    fn begin() -> Self {
        let start = LIVE.load(Relaxed);
        PEAK.store(start, Relaxed);
        Self { start, count: COUNT.load(Relaxed) }
    }
    fn end(self, name: &str, per: usize) {
        let live = LIVE.load(Relaxed);
        let peak = PEAK.load(Relaxed) - self.start;
        let allocations = (COUNT.load(Relaxed) - self.count) / per.max(1);
        let retained = live as i64 - self.start as i64;
        println!("{name},{peak},{retained},{allocations}");
    }
}

const B: Bounds = Bounds { x: 8.0, y: 48.0, width: 1000.0, height: 700.0 };

/// A heavy profile: 100 tabs, 1,000 bookmarks, a full history and download list.
/// Every `bookmarked_every`-th history entry is a visit to a bookmarked page.
fn heavy_state(bookmarked_every: usize) -> String {
    let tabs: Vec<_> = (0..100).map(|i| serde_json::json!({"Url": format!("https://site{i}.example.org/articles/{i}?ref=still")})).collect();
    let bookmarks: Vec<_> = (0..1000).map(|i| format!("https://bookmark{i}.example.org/some/longer/path/{i}")).collect();
    let history: Vec<_> = (0..500)
        .map(|i| {
            let url = if i % bookmarked_every == 0 {
                format!("https://bookmark{i}.example.org/some/longer/path/{i}")
            } else {
                format!("https://visited{i}.example.org/some/longer/path/{i}")
            };
            serde_json::json!({"Url": url, "VisitedAt": "2026-10-13T05:00:00+00:00", "Title": format!("A typical page title number {i} — Example")})
        })
        .collect();
    let downloads: Vec<_> = (0..200)
        .map(|i| serde_json::json!({"Path": format!("C:\\Users\\someone\\Downloads\\file-{i}.zip"), "State": "Completed", "Bytes": 123456, "Total": 123456}))
        .collect();
    serde_json::json!({"Version": 2, "Tabs": tabs, "ActiveIndex": 3, "Bookmarks": bookmarks, "History": history, "Downloads": downloads, "Settings": {"Theme": "Dark"}}).to_string()
}

fn main() {
    println!("scenario,peak_bytes,retained_bytes,allocations_per_op");
    // Page loads with a typical history: one visit in ten is to a bookmarked page.
    {
        let mut session = Session::load(Some(Ok(heavy_state(10))), &[]).0;
        session.set_bounds(B);
        session.plan_show(false);
        session.snapshot(false);
        let id = session.active();
        let (loads, mut bytes) = (200, 0);
        let m = Measure::begin();
        for i in 0..loads {
            let url = format!("https://news.example.org/story/{i}");
            session.load_started(id, &url);
            session.title_changed(id, "Breaking story — Example News");
            session.load_finished(id, &url);
            session.history_changed(id, true, false, &url);
            let update = serde_json::to_string(&session.snapshot(false)).unwrap();
            bytes += update.len();
            black_box(update);
        }
        m.end("page_load_typical_history", loads);
        println!("# UI update bytes per page load (typical history): {}", bytes / loads);
    }

    // The remaining scenarios use a history made only of bookmarked pages, the
    // worst case: every trimmed visit changes a bookmark row's title.
    let json = heavy_state(1);
    println!("# state.json size: {} bytes", json.len());

    // Startup: parse the saved file, build the session, send the first full snapshot.
    let m = Measure::begin();
    let mut session = Session::load(Some(Ok(json.clone())), &[]).0;
    m.end("startup_load", 1);
    let m = Measure::begin();
    let first = serde_json::to_string(&session.snapshot(true)).unwrap();
    println!("# first full snapshot: {} bytes", first.len());
    drop(first);
    m.end("startup_first_snapshot", 1);
    session.set_bounds(B);
    session.plan_show(false);
    session.snapshot(false);

    // One page load on the active tab, then the UI update (serialized like Tauri's emit).
    let id = session.active();
    let mut update_bytes = 0;
    let loads = 200;
    let m = Measure::begin();
    for i in 0..loads {
        let url = format!("https://news.example.org/story/{i}");
        session.load_started(id, &url);
        session.title_changed(id, "Breaking story — Example News");
        session.load_finished(id, &url);
        session.history_changed(id, true, false, &url);
        let update = serde_json::to_string(&session.snapshot(false)).unwrap();
        update_bytes += update.len();
        black_box(update);
    }
    m.end("page_load_all_history_bookmarked", loads);
    println!("# UI update bytes per page load (all history bookmarked): {}", update_bytes / loads);

    // A save: capture under the session lock, encode, write to a temporary file.
    let dir = std::env::temp_dir().join(format!("still-memory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("state.json");
    session.load_finished(id, "https://news.example.org/final");
    let m = Measure::begin();
    let state = session.take_state().unwrap();
    // As the shell's flush does: encode straight into the temporary file.
    still_core::save_atomic_with(&path, |out| serde_json::to_writer(out, &state).map_err(std::io::Error::from)).unwrap();
    drop(state);
    m.end("save", 1);

    // Tab churn: open and close 5,000 tabs ten times. Retained bytes show growth.
    let m = Measure::begin();
    for _ in 0..10 {
        let ids: Vec<_> = (0..5000).map(|i| session.open_tab(&format!("https://churn{i}.example.org/"))).collect();
        session.snapshot(false);
        for id in ids {
            session.close(id);
        }
        session.snapshot(false);
        black_box(session.take_state());
    }
    m.end("tab_churn_50000", 1);

    // Long run: 20,000 more page loads with saves every 50 loads.
    let m = Measure::begin();
    for i in 0..20_000 {
        let url = format!("https://long.example.org/{i}");
        session.load_started(id, &url);
        session.title_changed(id, "Long running page");
        session.load_finished(id, &url);
        black_box(serde_json::to_string(&session.snapshot(false)).unwrap());
        if i % 50 == 0 {
            if let Some(state) = session.take_state() {
                black_box(serde_json::to_string(&state).unwrap());
            }
        }
    }
    m.end("long_run_20000_loads", 1);

    let m = Measure::begin();
    drop(session);
    m.end("drop_session", 1);
    let _ = std::fs::remove_dir_all(dir);
}
