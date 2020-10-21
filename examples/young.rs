fn main() {
    let _logger = naive_logger::init(naive_logger::LoggerConfig::default());

    log::info!("too young");
    log::warn!("too simple");
    log::error!("sometimes naive");
}
