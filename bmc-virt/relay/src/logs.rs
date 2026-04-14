// Copyright (C) 2026  Braiins Systems s.r.o.

// Log streaming infrastructure: each log source implements a common streaming
// contract (backlog + follow), spawned per-connection and stopped on disconnect.

use bmc_virt_ipc::{GuestSender, LogSource};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// How to stream log lines from a source.
enum StreamKind {
    /// Run a shell command that outputs backlog then follows with new lines.
    /// The command must not exit on its own — it should block on new output.
    Cmd {
        program: &'static str,
        args: &'static [&'static str],
    },
    /// Tail a file from the last 16 KB, then poll for new lines.
    File { path: &'static str },
}

/// A log source definition — source ID + how to stream it.
struct LogSourceDef {
    source: LogSource,
    stream: StreamKind,
}

/// All log sources with their streaming strategy.
const SOURCES: &[LogSourceDef] = &[
    LogSourceDef {
        source: LogSource::BmcLog,
        stream: StreamKind::File {
            path: "/root/bmc.log",
        },
    },
    LogSourceDef {
        source: LogSource::Syslog,
        stream: StreamKind::Cmd {
            program: "sh",
            args: &["-c", "logread; exec logread -f"],
        },
    },
    LogSourceDef {
        source: LogSource::Dmesg,
        stream: StreamKind::Cmd {
            program: "sh",
            args: &["-c", "dmesg; exec dmesg -w"],
        },
    },
    LogSourceDef {
        source: LogSource::RelayLog,
        stream: StreamKind::File {
            path: "/tmp/relay.log",
        },
    },
];

/// Handle to stop all tailer threads for a connection.
/// Spawned per-connection, stopped on disconnect.
pub struct TailerHandle {
    cancel: Arc<AtomicBool>,
    children: Arc<Mutex<Vec<Child>>>,
}

impl TailerHandle {
    /// Signal all tailer threads to stop and kill child processes.
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        let mut children = self
            .children
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for child in children.iter_mut() {
            let _ = child.kill();
        }
        children.clear();
    }
}

/// Spawn a tailer thread per log source. Returns a handle to stop them all.
/// Each tailer reads backlog + follows, so the console gets full history.
pub fn start_tailers(sender: &GuestSender) -> TailerHandle {
    let cancel = Arc::new(AtomicBool::new(false));
    let children: Arc<Mutex<Vec<Child>>> = Arc::new(Mutex::new(Vec::new()));

    for def in SOURCES {
        let source = def.source;
        let name = source.name();
        let tx = sender.clone();
        let cancel = Arc::clone(&cancel);

        match &def.stream {
            StreamKind::File { path } => {
                let path = *path;
                std::thread::Builder::new()
                    .name(format!("log-{name}"))
                    .spawn(move || tail_file(&tx, source, path, &cancel))
                    .unwrap_or_else(|e| panic!("failed to spawn log-{name}: {e}"));
            }
            StreamKind::Cmd { program, args } => {
                let program = *program;
                let args: Vec<&str> = args.to_vec();
                let children = Arc::clone(&children);
                std::thread::Builder::new()
                    .name(format!("log-{name}"))
                    .spawn(move || tail_command(&tx, source, program, &args, &children, &cancel))
                    .unwrap_or_else(|e| panic!("failed to spawn log-{name}: {e}"));
            }
        }
        eprintln!("log tailer started: {name}");
    }

    TailerHandle { cancel, children }
}

fn tail_file(sender: &GuestSender, source: LogSource, path: &str, cancel: &AtomicBool) {
    // Wait for the file to appear (or cancellation)
    while !std::path::Path::new(path).exists() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(err) => {
            eprintln!("log {path}: open failed: {err}");
            return;
        }
    };

    let file_len = file.metadata().map_or(0, |m| m.len());
    let start_pos = file_len.saturating_sub(16 * 1024);

    let mut file = file;
    let _ = file.seek(SeekFrom::Start(start_pos));

    // If we seeked into the middle, skip to the next newline
    if start_pos > 0 {
        let mut skip_buf = [0_u8; 1];
        loop {
            match file.read(&mut skip_buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if skip_buf[0] == b'\n' {
                        break;
                    }
                }
            }
        }
    }

    let mut reader = BufReader::new(file);
    let mut line = String::new();

    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => {
                // EOF — poll for new data
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
            Ok(_) => {
                let trimmed = line.trim_end();
                if !trimmed.is_empty() {
                    sender.send_log(source, trimmed.to_owned());
                }
            }
            Err(err) => {
                eprintln!("log {path}: read error: {err}");
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
    }
}

fn tail_command(
    sender: &GuestSender,
    source: LogSource,
    program: &str,
    args: &[&str],
    children: &Mutex<Vec<Child>>,
    cancel: &AtomicBool,
) {
    let mut child = match Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("log {program}: spawn failed: {err}");
            return;
        }
    };

    let stdout = child
        .stdout
        .take()
        .unwrap_or_else(|| panic!("BUG: stdout was piped"));

    // Register child so TailerHandle::stop() can kill it
    children
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(child);

    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        match line {
            Ok(line) => {
                if !line.is_empty() {
                    sender.send_log(source, line);
                }
            }
            Err(err) => {
                eprintln!("log {program}: read error: {err}");
                break;
            }
        }
    }
}
