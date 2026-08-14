//! Open exactly one plugin file by path and say what it is. The blunt diagnostic behind the
//! blacklist: when a scan hangs or dies, this is how the culprit is confirmed in isolation —
//! `cargo run --release --example openone -- "C:\...\Thing.dll"`. Expect never to return if the
//! plugin hangs in LoadLibrary; that non-answer is the answer.

fn main() {
    let path = std::env::args().nth(1).expect("usage: openone <path to .dll or .vst3>");
    println!("opening {path} …");
    match volt_vsthost::Module::open(std::path::Path::new(&path)) {
        Ok(m) => {
            for c in m.classes() {
                println!("  [{:?}] {} — {} ({})", c.format, c.name, c.vendor, c.subcategories);
            }
            println!("opened cleanly.");
        }
        Err(e) => println!("  failed: {e}"),
    }
    volt_vsthost::hard_exit(0);
}
