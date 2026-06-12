// Copyright (C) 2026  Braiins Systems s.r.o.

use std::path::Path;

pub fn init(log_path: &Path) -> std::io::Result<()> {
    bmc_log::init_file(log_path)
}
