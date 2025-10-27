// Copyright (C) 2025  Braiins Systems s.r.o.

pub mod constants;
pub mod encrypt;
pub mod network;

use crate::constants::{
    BMC_CONFIG, BOARD, BOS_MAJOR, BOS_MODE, BOS_PLATFORM, BOS_VERSION, ETC_DNSMASQ_CONF, ETC_HOSTS,
    ETC_RESOLV_CONF, FACTORY_DEFAULT, PROC_CPUINFO, PROC_MTD, SETUP_PENDING, SRC_ETC_CONF,
    SRC_LOGS,
};
use crate::network::PcapError;
use anyhow::Result;
use std::fs::File;
use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};
use walkdir::WalkDir;
use zip::result::ZipResult;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

const PCAP_DURATION: Duration = Duration::from_secs(5);

/// These commands will be executed and their stdout will be included in the support archive.
const COMMANDS: &[&[&str]] = &[
    &["dmesg"],
    &["fw_printenv"],
    &["env"],
    &["ifconfig", "-a"],
    &["ip", "addr"],
    &["ip", "route"],
    &["ps", "aux"],
    &["df"],
    &["ls", "-l", "/tmp"],
    &["killall", "-SIGUSR1", "dnsmasq"],
];

/// All contents of these paths will be included in the support archive.
const FS_PATHS: &[&str] = &[
    // files
    BOS_VERSION,
    BOS_MAJOR,
    BOS_MODE,
    BOS_PLATFORM,
    ETC_HOSTS,
    ETC_RESOLV_CONF,
    ETC_DNSMASQ_CONF,
    BOARD,
    BMC_CONFIG,
    FACTORY_DEFAULT,
    SETUP_PENDING,
    // directories
    SRC_LOGS,
    SRC_ETC_CONF,
    // additional procfs items
    PROC_MTD,
    PROC_CPUINFO,
    "/proc/loadavg",
    "/proc/cmdline",
    "/proc/crypto",
    "/proc/devices",
    "/proc/iomem",
    "/proc/ioports",
    "/proc/filesystems",
    "/proc/interrupts",
    "/proc/meminfo",
    "/proc/misc",
    "/proc/modules",
    "/proc/partitions",
    "/proc/stat",
    "/proc/uptime",
    "/proc/version",
    "/proc/net/arp",
];

pub const PING_HOSTS: &[&str] = &[
    "127.0.0.1",
    "8.8.8.8",
    "google.com",
    "downloads.braiins.com",
    "public-api.braiins.com",
];

#[derive(Debug)]
pub enum SupportArchiveFormat {
    Zip,
    ZipEncrypted,
}

pub fn collect(
    writer: &mut impl Write,
    format: SupportArchiveFormat,
    compress: bool,
) -> Result<()> {
    let mut archive = SupportZipWriter::new(writer, format, compress);

    // include outputs of commands
    // Commands must run before logs are collected since some of them can print something to the syslog (e.g. dnsmasq)
    for &cmdline in COMMANDS {
        match archive.add_cmd_output(cmdline) {
            Ok(()) => info!("Added output of '{}'", cmdline.join(" ")),
            Err(err) => error!("{}: '{}'", err, cmdline.join(" ")),
        }
    }

    // spawn minipcap for each interface...
    let pcap_handles: Vec<_> = pnet::datalink::interfaces()
        .into_iter()
        .map(|interface| {
            thread::spawn(move || {
                network::pcap(&interface, PCAP_DURATION).map(|pcap| (interface.name, pcap))
            })
        })
        .collect();

    // include output of builtin routines
    // Again these commands may produce some logs so log collection must be done after this.
    #[expect(clippy::type_complexity)]
    let builtin_items: &[(&str, fn() -> Option<String>)] = &[
        ("ifconfig", || Some(network::ifconfig())),
        ("public_ip", || network::public_ip().ok()),
        ("ping_report", || network::ping_report(PING_HOSTS).ok()),
        ("timestamp", || {
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_secs()
                .to_string()
                .into()
        }),
    ];
    for (name, function) in builtin_items {
        if let (name, Some(content)) = (name, function()) {
            match archive.add_builtin(name, &content) {
                Ok(()) => info!("Added output of <{}>", name),
                Err(err) => error!("{}: <{}>", err, name),
            }
        }
    }

    // include files from the filesystem
    for path in FS_PATHS.iter().map(Path::new) {
        assert!(path.is_absolute());

        let entries = WalkDir::new(path)
            .into_iter()
            .filter_map(|res| res.map_err(|err| warn!("{}", err)).ok())
            .filter(|entry| entry.path().is_file());

        for entry in entries {
            match archive.add_fs_file(entry.path()) {
                Ok(()) => info!("Added file {}", entry.path().display()),
                Err(err) => error!("{}: {}", err, entry.path().display()),
            }
        }
    }

    // collect pcap results and include them as well
    for handle in pcap_handles {
        let Ok(pcap_result) = handle.join() else {
            error!("Thread panicked while capturing pcap data");
            continue;
        };

        match archive.add_pcap(pcap_result) {
            Ok(interface_name) => info!("Added pcap of interface '{}'", interface_name),
            Err(err) => error!("Error adding pcap: {:#}", err),
        }
    }

    archive.finish()?;

    Ok(())
}

struct SupportZipWriter<'w, W: Write> {
    writer: &'w mut W,
    format: SupportArchiveFormat,
    archive: ZipWriter<Cursor<Vec<u8>>>,
    options: SimpleFileOptions,
}

impl<'w, W: Write> SupportZipWriter<'w, W> {
    pub fn new(writer: &'w mut W, format: SupportArchiveFormat, compress: bool) -> Self {
        let options = SimpleFileOptions::default().compression_method(if compress {
            CompressionMethod::Deflated
        } else {
            CompressionMethod::Stored
        });

        Self {
            writer,
            format,
            archive: ZipWriter::new(Cursor::new(vec![])),
            options,
        }
    }

    pub fn finish(self) -> ZipResult<()> {
        let mut buffer = self.archive.finish()?.into_inner();

        buffer = match self.format {
            SupportArchiveFormat::ZipEncrypted => encrypt::encrypt(&buffer),
            SupportArchiveFormat::Zip => buffer,
        };

        self.writer.write_all(&buffer)?;

        Ok(())
    }

    pub fn add_fs_file(&mut self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();

        let mut file = File::open(path)?;
        let mut buf = vec![];
        file.read_to_end(&mut buf)?;

        let name = Path::new("filesystem")
            .join(path.strip_prefix("/")?)
            .display()
            .to_string();

        self.write_file(&name, &buf)?;

        Ok(())
    }

    pub fn add_cmd_output(&mut self, cmdline: &[&str]) -> Result<()> {
        let (&program, args) = cmdline.split_first().expect("BUG: empty command");

        let output = Command::new(program).args(args).output()?;

        let name = cmdline.join("_").replace(['/', '.'], "_");

        let stdout = format!("command/{name}");
        self.write_file(&stdout, &output.stdout)?;

        if !output.stderr.is_empty() {
            let stderr = format!("command/{name}.stderr");
            self.write_file(&stderr, &output.stderr)?;
        }

        Ok(())
    }

    pub fn add_builtin(&mut self, name: impl AsRef<str>, content: impl AsRef<str>) -> Result<()> {
        let file_name = format!("builtin/{}", name.as_ref());
        self.write_file(&file_name, content.as_ref().as_bytes())?;
        Ok(())
    }

    pub fn add_pcap(
        &mut self,
        pcap_result: Result<(String, Vec<u8>), PcapError>,
    ) -> Result<String> {
        let (interface_name, pcap) = pcap_result?;
        let file_name = format!("pcap/{interface_name}.pcap",);
        self.write_file(&file_name, &pcap)?;
        Ok(interface_name)
    }

    fn write_file(&mut self, name: &str, buf: &[u8]) -> Result<()> {
        self.archive.start_file(name, self.options)?;
        self.archive.write_all(buf)?;
        Ok(())
    }
}
