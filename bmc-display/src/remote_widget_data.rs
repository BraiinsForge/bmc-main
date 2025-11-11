// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::SharedImageBuffer;

#[derive(Debug)]
pub enum RemoteWidgetState {
    Initial,
    ConfigurationError,
    Loading,
    LoadingSuccess(SharedImageBuffer),
    LoadingError(anyhow::Error),
    UnexpectedError,
}
