// Copyright (C) 2025  Braiins Systems s.r.o.

use std::path::PathBuf;

pub use clap::Parser;

#[derive(Debug, Parser)]
pub struct Args {
    #[clap(long)]
    pub log_to_file: bool,

    /// Path to widget directories (can be specified multiple times)
    #[clap(long = "widgets-path")]
    pub widgets_paths: Vec<PathBuf>,

    /// Path to glibc dynamic linker for widget spawning
    #[clap(long)]
    pub widget_linker: Option<String>,

    /// Library search path for widget spawning (passed to linker via --library-path)
    #[clap(long)]
    pub widget_library_path: Option<String>,

    /// Mesa GBM backends path for widget GPU rendering (GBM_BACKENDS_PATH)
    #[clap(long)]
    pub widget_gbm_backends_path: Option<String>,

    /// Mesa DRI drivers path for widget GPU rendering (LIBGL_DRIVERS_PATH)
    #[clap(long)]
    pub widget_libgl_drivers_path: Option<String>,

    /// EGL vendor library for widget GPU rendering (__EGL_VENDOR_LIBRARY_FILENAMES)
    #[clap(long)]
    pub widget_egl_vendor_library: Option<String>,
}
