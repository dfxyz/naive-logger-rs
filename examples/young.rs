fn main() {
    let config = naive_logger::LoggerConfig::default();
    let _ = naive_logger::init(config);
    log::info!("too young");
    log::warn!("too simple");
    log::error!("sometimes naive");
    naive_logger::shutdown();
}