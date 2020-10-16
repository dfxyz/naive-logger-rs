//! A young, simple and naive asynchronous logger implementation.

use std::cell::Cell;
use std::fs::{File, OpenOptions};
use std::io::{stdout, Seek, SeekFrom, Write, ErrorKind};
use std::ptr::null_mut;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};
use log::{Level, Log, Metadata, Record};

pub struct LoggerConfig {
    /// Log level.
    pub level: Level,
    /// Use stdout to output the log?
    pub use_stdout: bool,
    /// The path of the main log file. If empty, won't output log to file.
    pub file_path: String,
    /// Limit the log file size in bytes. If 0, won't rotate the log file.
    pub file_max_size: u64,
    /// Limit the number of rotated log files. If 0, won't rotate the log file.
    pub rotated_file_num: u64,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        LoggerConfig {
            level: Level::Info,
            use_stdout: true,
            file_path: "".to_string(),
            file_max_size: 0,
            rotated_file_num: 0,
        }
    }
}

enum LoggerMessage {
    Log(String),
    Flush,
    Shutdown,
}

struct Logger {
    level: Level,
    sender: Sender<LoggerMessage>,
    join_handle: JoinHandle<()>,
}

struct LoggerInternal {
    config: LoggerConfig,
    receiver: Receiver<LoggerMessage>,
    file_ptr: Cell<*mut File>,
    file_size: Cell<u64>,
    file_paths: Vec<String>,
}

unsafe impl Send for LoggerInternal {}

static mut LOGGER: Option<Logger> = None;

/// Initialize the logger. Should be called only once.
pub fn init(config: LoggerConfig) -> Result<(), String> {
    let level = config.level;
    let level_filter = level.to_level_filter();

    let file_path = config.file_path.as_str();
    let file_ptr;
    let file_size;
    let file_paths;
    if !file_path.is_empty() {
        let mut file = match OpenOptions::new().create(true).write(true).open(file_path) {
            Ok(f) => f,
            Err(e) => {
                return Err(format!(
                    "failed to open the log file '{}': {}",
                    file_path, e
                ))
            }
        };
        let metadata = match file.metadata() {
            Ok(m) => {
                if m.is_dir() {
                    return Err(format!(
                        "failed to open the log file '{}'; it's a directory",
                        file_path
                    ));
                }
                m
            }
            Err(e) => {
                return Err(format!(
                    "failed to read metadata from the log file '{}': {}",
                    file_path, e
                ))
            }
        };
        if let Err(e) = file.seek(SeekFrom::End(0)) {
            return Err(format!("failed to set the log file cursor: {}", e));
        }

        file_ptr = Box::into_raw(Box::new(file));
        file_size = metadata.len();
        file_paths = {
            let rotated_file_num = config.rotated_file_num as usize;
            let mut v = Vec::with_capacity(rotated_file_num);
            v.push(file_path.to_string());
            for i in 1..=rotated_file_num {
                v.push(format!("{}.{}", file_path, i));
            }
            v
        }
    } else {
        file_ptr = null_mut();
        file_size = 0;
        file_paths = vec![];
    }

    let (sender, receiver) = crossbeam_channel::unbounded();

    let internal = LoggerInternal {
        config,
        receiver,
        file_ptr: Cell::new(file_ptr),
        file_size: Cell::new(file_size),
        file_paths,
    };
    let join_handle = match std::thread::Builder::new()
        .name("naive_logger".to_string())
        .spawn(move || {
            internal.run();
        }) {
        Ok(h) => h,
        Err(e) => return Err(format!("failed to spawn the logger thread: {}", e)),
    };

    let logger = Logger {
        level,
        sender,
        join_handle,
    };
    unsafe { LOGGER = Some(logger) };

    log::set_max_level(level_filter);
    if log::set_logger(unsafe { LOGGER.as_ref().unwrap() }).is_err() {
        eprintln!("failed to active the logger");
    }

    Ok(())
}

/// Make sure the logs are flushed and close the logger.
/// Will block and wait for the logger thread to finish.
pub fn shutdown() {
    let logger = unsafe { LOGGER.take().unwrap() };
    logger.shutdown();
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.level >= metadata.level()
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
            let datetime = chrono::Local::now().format("%F %T%.3f");
            let message = format!(
                "[{}][{:>5}][{}]: {}\n",
                datetime,
                record.level(),
                record.target(),
                record.args()
            );
            let _ = self.sender.send(LoggerMessage::Log(message));
        }
    }

    fn flush(&self) {
        let _ = self.sender.send(LoggerMessage::Flush);
    }
}

impl Logger {
    fn shutdown(self) {
        let _ = self.sender.send(LoggerMessage::Shutdown);
        let _ = self.join_handle.join();
    }
}

impl LoggerInternal {
    fn run(&self) {
        for msg in &self.receiver {
            match msg {
                LoggerMessage::Log(message) => self.log(message),
                LoggerMessage::Flush => self.flush(),
                LoggerMessage::Shutdown => break,
            }
        }
        self.flush();
    }

    fn log(&self, message: String) {
        if self.config.use_stdout {
            print!("{}", &message);
        }
        let file_ptr = self.file_ptr.get();
        if !file_ptr.is_null() {
            let file = unsafe { file_ptr.as_mut().unwrap() };
            let msg = message.as_bytes();
            match file.write_all(msg) {
                Ok(_) => {
                    let new_size = self.file_size.get() + msg.len() as u64;
                    self.file_size.set(new_size);

                    let max_size = self.config.file_max_size;
                    if max_size != 0 && new_size > max_size {
                        self.rotate_log_files();
                    }
                }
                Err(e) => eprintln!("logger failed to write file: {}", e),
            }
        }
    }

    fn flush(&self) {
        if self.config.use_stdout {
            if let Err(e) = stdout().flush() {
                eprintln!("logger failed to flush the stdout: {}", e);
            };
        }
        let file_ptr = self.file_ptr.get();
        if !file_ptr.is_null() {
            let file = unsafe { file_ptr.as_mut().unwrap() };
            if let Err(e) = file.flush() {
                eprintln!("logger failed to flush the file: {}", e);
            }
        }
    }

    fn rotate_log_files(&self) {
        let rotated_file_num = self.config.rotated_file_num;
        if rotated_file_num == 0 {
            return;
        }

        // rotate files
        let file_paths = &self.file_paths;
        for i in (0..rotated_file_num).rev() {
            let src = file_paths[i as usize].as_str();
            let dst = file_paths[(i + 1) as usize].as_str();
            if let Err(e) = std::fs::copy(src, dst) {
                if e.kind() != ErrorKind::NotFound {
                    eprintln!("logger failed to copy '{}' to '{}': {}", src, dst, e);
                }
            }
        }

        // reset the active file
        let file_ptr = self.file_ptr.get();
        let file = unsafe { file_ptr.as_mut().unwrap() };
        if let Err(e) = file.set_len(0) {
            // fatal error, release resources, disable file outputting
            eprintln!("logger failed to truncate the file: {}", e);
            unsafe { Box::from_raw(file_ptr) };
            self.file_ptr.set(null_mut());
            return;
        }
        if let Err(e) = file.seek(SeekFrom::Start(0)) {
            // fatal error, release resources, disable file outputting
            eprintln!("logger failed to reset the log file cursor: {}", e);
            unsafe { Box::from_raw(file_ptr) };
            self.file_ptr.set(null_mut());
            return;
        }

        self.file_size.set(0);
    }
}

impl Drop for LoggerInternal {
    fn drop(&mut self) {
        let file_ptr = self.file_ptr.get();
        if !file_ptr.is_null() {
            unsafe { Box::from_raw(file_ptr) };
        }
    }
}
