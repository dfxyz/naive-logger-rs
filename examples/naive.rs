use std::thread::spawn;

fn main() {
    let mut config = naive_logger::LoggerConfig::default();
    config.level = log::Level::Debug;
    config.log_file_path = "naive.log".to_string();
    config.log_file_max_size = 65536;
    config.rotated_log_file_num = 4;
    let _logger = naive_logger::init(config);

    let handle0 = spawn(|| {
        for i in 0..512 {
            log::info!("I will do whatever it takes to serve my country even at the cost of my own life, regardless of fortune or misfortune to myself. ({})", i);
        }
    });
    let handle1 = spawn(|| {
        for i in 0..512 {
            log::warn!("I will do whatever it takes to serve my country even at the cost of my own life, regardless of fortune or misfortune to myself. ({})", i);
        }
    });
    let handle2 = spawn(|| {
        for i in 0..512 {
            log::error!("I will do whatever it takes to serve my country even at the cost of my own life, regardless of fortune or misfortune to myself. ({})", i);
        }
    });
    handle0.join().unwrap();
    handle1.join().unwrap();
    handle2.join().unwrap();
}
