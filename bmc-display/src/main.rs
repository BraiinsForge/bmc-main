// Copyright (C) 2025  Braiins Systems s.r.o.
use crate::generated::*;

#[allow(warnings)]
mod generated {
    slint::include_modules!();
}

fn main() -> Result<(), slint::PlatformError> {
    let main_window = MainWindow::new()?;

    main_window.run()
}
