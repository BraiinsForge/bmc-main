// Copyright (C) 2025  Braiins Systems s.r.o.

// files
pub const BOS_VERSION: &str = "/etc/bos_version";
pub const BOS_MAJOR: &str = "/etc/bos_major";
pub const BOS_MODE: &str = "/etc/bos_mode";
pub const BOS_PLATFORM: &str = "/etc/bos_platform";
pub const BOARD: &str = "/etc/board.json";
pub const BMC_CONFIG: &str = "/etc/bmc/config.json";
/// Pre-migration config path, deliberately kept on disk for downgrade
/// safety (see `bmc::config_migration`). Collected so a bad migration
/// can still be diagnosed from the original file.
pub const BMC_CONFIG_LEGACY: &str = "/etc/bmc_config.json";
pub const FACTORY_DEFAULT: &str = "/etc/factory-default";
pub const SETUP_PENDING: &str = "/etc/setup-pending";
pub const PROC_MTD: &str = "/proc/mtd";
pub const PROC_CPUINFO: &str = "/proc/cpuinfo";
pub const ETC_HOSTS: &str = "/etc/hosts";
pub const ETC_RESOLV_CONF: &str = "/etc/resolv.conf";
pub const ETC_DNSMASQ_CONF: &str = "/etc/dnsmasq.conf";

// directories
pub const SRC_LOGS: &str = "/var/log";
pub const SRC_ETC_CONF: &str = "/etc/config";
/// Directory holding the current config and its timestamped backups.
/// Collected wholesale so `config.json.backup.<ts>` snapshots ride
/// along in the support archive.
pub const BMC_CONFIG_DIR: &str = "/etc/bmc";
