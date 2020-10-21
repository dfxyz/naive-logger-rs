//! A young, simple and naive asynchronous logger implementation.
//!
//! Workflow:
//! * call "init()" in "main()":
//!     * allocate an [`Arc<Logger>`] in main thread's stack
//!     * allocate an static [`Weak<Logger>`] which implements [`Log`] trait
//!     * spawn the logger thread and send a [`LoggerInternal`] into it
//!     * call [`log::set_logger`] and [`log::set_max_level`]
//! * spawn other threads and use macros in [`log`] to record logs
//! * when "main()" returns, the only [`Arc<Logger>`] instance will be dropped and
//! the shutdown procedure will be executed:
//!     * send a shutdown message to the log thread
//!     * wait log thread to finish

use std::cell::{Cell, RefCell};
use std::fs::{File, OpenOptions};
use std::io::{stdout, ErrorKind, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender};
use log::{Level, Log, Metadata, Record};

pub struct LoggerConfig {
    /// Log level.
    pub level: Level,
    /// Use stdout to output the log?
    pub use_stdout: bool,
    /// The path of the main log file. If empty, won't output log to file.
    pub log_file_path: String,
    /// Limit the log file size in bytes. If 0, won't rotate the log file.
    pub log_file_max_size: u64,
    /// Limit the number of rotated log files. If 0, won't rotate the log file.
    pub rotated_log_file_num: u64,
}

impl Default for LoggerConfig {
    fn default() -> Self {
        LoggerConfig {
            level: Level::Info,
            use_stdout: true,
            log_file_path: "".to_string(),
            log_file_max_size: 0,
            rotated_log_file_num: 0,
        }
    }
}

enum LoggerMessage {
    Log(String),
    Flush,
    Shutdown,
}

pub struct Logger {
    level: Level,
    sender: Sender<LoggerMessage>,
    join_handle: Option<JoinHandle<()>>,
}

struct LoggerWeakRef(Weak<Logger>);

struct LoggerInternal {
    config: LoggerConfig,
    receiver: Receiver<LoggerMessage>,
    log_file: RefCell<Option<File>>,
    log_file_size: Cell<u64>,
    log_file_paths: Vec<String>,
}

unsafe impl Send for LoggerInternal {}

static INITIALIZED: AtomicBool = AtomicBool::new(false);
static mut LOGGER_WEAK_REF: Option<LoggerWeakRef> = None;

pub fn init(config: LoggerConfig) -> Result<Arc<Logger>, String> {
    if INITIALIZED.compare_and_swap(false, true, Ordering::Relaxed) {
        return Err("already initialized".into());
    }

    let level = config.level;
    let level_filter = level.to_level_filter();

    let log_file_path = config.log_file_path.as_str();
    let log_file;
    let log_file_size;
    let log_file_paths;
    if !log_file_path.is_empty() {
        let mut file = match OpenOptions::new()
            .create(true)
            .write(true)
            .open(log_file_path)
        {
            Ok(f) => f,
            Err(e) => {
                return Err(format!(
                    "failed to open the log file '{}': {}",
                    log_file_path, e
                ))
            }
        };

        let metadata = match file.metadata() {
            Ok(m) => {
                if m.is_dir() {
                    return Err(format!(
                        "failed to open the log file '{}'; it's a directory",
                        log_file_path
                    ));
                }
                m
            }
            Err(e) => {
                return Err(format!(
                    "failed to read metadata from the log file '{}': {}",
                    log_file_path, e
                ))
            }
        };

        if let Err(e) = file.seek(SeekFrom::End(0)) {
            return Err(format!("failed to set the log file cursor: {}", e));
        }

        log_file = RefCell::new(Some(file));
        log_file_size = Cell::new(metadata.len());
        log_file_paths = {
            let rotated_file_num = config.rotated_log_file_num as usize;
            let mut v = Vec::with_capacity(rotated_file_num);
            v.push(log_file_path.to_string());
            for i in 1..=rotated_file_num {
                v.push(format!("{}.{}", log_file_path, i));
            }
            v
        }
    } else {
        log_file = RefCell::new(None);
        log_file_size = Cell::new(0);
        log_file_paths = vec![];
    }

    let (sender, receiver) = crossbeam_channel::unbounded();

    let internal = LoggerInternal {
        config,
        receiver,
        log_file,
        log_file_size,
        log_file_paths,
    };
    let join_handle = match std::thread::Builder::new()
        .name("naive_logger".to_string())
        .spawn(move || {
            internal.run();
        }) {
        Ok(h) => Some(h),
        Err(e) => return Err(format!("failed to spawn the logger thread: {}", e)),
    };

    let logger = Arc::new(Logger {
        level,
        sender,
        join_handle,
    });
    let weak = LoggerWeakRef(Arc::downgrade(&logger));
    unsafe { LOGGER_WEAK_REF = Some(weak) };

    log::set_max_level(level_filter);
    if log::set_logger(unsafe { LOGGER_WEAK_REF.as_ref().unwrap() }).is_err() {
        eprintln!("failed to set as the global logger");
    }

    Ok(logger)
}

impl LoggerWeakRef {
    #[inline]
    fn upgrade(&self) -> Option<Arc<Logger>> {
        self.0.upgrade()
    }
}

impl Log for LoggerWeakRef {
    fn enabled(&self, metadata: &Metadata) -> bool {
        if let Some(logger) = self.upgrade() {
            logger.level >= metadata.level()
        } else {
            false
        }
    }

    fn log(&self, record: &Record) {
        if let Some(logger) = self.upgrade() {
            if logger.level >= record.level() {
                let datetime = chrono::Local::now().format("%F %T%.3f");
                let message = format!(
                    "[{}][{:>5}][{}]: {}\n",
                    datetime,
                    record.level(),
                    record.target(),
                    record.args()
                );
                let _ = logger.sender.send(LoggerMessage::Log(message));
            }
        }
    }

    fn flush(&self) {
        if let Some(logger) = self.upgrade() {
            let _ = logger.sender.send(LoggerMessage::Flush);
        }
    }
}

impl Drop for Logger {
    fn drop(&mut self) {
        let _ = self.sender.send(LoggerMessage::Shutdown);
        let _ = self.join_handle.take().unwrap().join();
    }
}

impl LoggerInternal {
    #[inline]
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
        let mut log_file_ref = self.log_file.borrow_mut();
        if let Some(file) = log_file_ref.as_mut() {
            let msg = message.as_bytes();
            match file.write_all(msg) {
                Ok(_) => {
                    let max_size = self.config.log_file_max_size;
                    if max_size == 0 || self.config.rotated_log_file_num == 0 {
                        return; // rotating disabled, no need to update the log file size
                    }

                    let new_size = self.log_file_size.get() + msg.len() as u64;
                    if new_size >= max_size {
                        drop(log_file_ref);
                        self.rotate_log_files();
                    } else {
                        self.log_file_size.set(new_size);
                    }
                }
                Err(e) => eprintln!("logger failed to write file: {}", e),
            }
        }
    }

    fn rotate_log_files(&self) {
        let rotated_file_num = self.config.rotated_log_file_num;
        if rotated_file_num == 0 {
            return;
        }

        // rotate files
        let file_paths = &self.log_file_paths;
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
        if let Some(file) = self.log_file.borrow_mut().as_mut() {
            if let Err(e) = file.set_len(0) {
                // fatal error, release resources, disable file outputting
                eprintln!("logger failed to truncate the file: {}", e);
                self.log_file.borrow_mut().take();
                return;
            }
            if let Err(e) = file.seek(SeekFrom::Start(0)) {
                // fatal error, release resources, disable file outputting
                eprintln!("logger failed to reset the log file cursor: {}", e);
                self.log_file.borrow_mut().take();
                return;
            }
        }

        self.log_file_size.set(0);
    }

    fn flush(&self) {
        if self.config.use_stdout {
            if let Err(e) = stdout().flush() {
                eprintln!("logger failed to flush the stdout: {}", e);
            };
        }
        if let Some(file) = self.log_file.borrow_mut().as_mut() {
            if let Err(e) = file.flush() {
                eprintln!("logger failed to flush the file: {}", e);
            }
        }
    }
}
