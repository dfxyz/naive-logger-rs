fn main() {
    let mut config = naive_logger::LoggerConfig::default();
    config.level = log::Level::Debug;
    let _ = naive_logger::init(config);
    log::debug!("too young");
    log::info!("too simple");
    log::warn!("sometimes naive");
    log::error!("I'm angry!");
    naive_logger::shutdown();
}