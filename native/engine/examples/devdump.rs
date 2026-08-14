//! Print every device the engine's enumeration sees, WITH the fork's log warnings visible.
//! Written 2026-08-12, the day WASAPI devices vanished from the list and the warn! that said
//! why was going to a logger nobody had installed.
struct P;
impl log::Log for P {
    fn enabled(&self, _: &log::Metadata) -> bool { true }
    fn log(&self, r: &log::Record) { println!("[{}] {}", r.level(), r.args()); }
    fn flush(&self) {}
}
fn main() {
    log::set_boxed_logger(Box::new(P)).unwrap();
    log::set_max_level(log::LevelFilter::Debug);
    for d in web_audio_api::media_devices::enumerate_devices_sync() {
        println!("{:?}  {}  {}", d.kind(), d.device_id(), d.label());
    }
}
