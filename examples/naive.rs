fn main() {
    let mut config = naive_logger::LoggerConfig::default();
    config.level = log::Level::Debug;
    config.file_path = "naive.log".to_string();
    config.file_max_size = 65536;
    config.rotated_file_num = 4;
    let _ = naive_logger::init(config);
    for i in 0..512 {
        log::info!("I will do whatever it takes to serve my country even at the cost of my own life, regardless of fortune or misfortune to myself. ({})", i);
        log::warn!("I will do whatever it takes to serve my country even at the cost of my own life, regardless of fortune or misfortune to myself. ({})", i);
        log::error!("I will do whatever it takes to serve my country even at the cost of my own life, regardless of fortune or misfortune to myself. ({})", i);
    }
    naive_logger::shutdown();
}
