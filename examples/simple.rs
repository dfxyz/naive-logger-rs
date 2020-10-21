use std::thread::spawn;

fn main() {
    let mut config = naive_logger::LoggerConfig::default();
    config.level = log::Level::Debug;
    let _logger = naive_logger::init(config);

    spawn(|| {
        log::debug!("too young");
        log::info!("too simple");
        log::warn!("sometimes naive");
    })
    .join()
    .unwrap();
}
