// Copyright (C) 2024  Braiins Systems s.r.o.

use crate::error_chain::ErrorChain;
use fdlimit::Outcome;
use tracing::{debug, warn};

/// Raise the file descriptor limit to the max possible value.
pub fn raise_fd_limit() {
    if cfg!(windows) {
        debug!("raise_fd_limit is a no-op on windows");
        return;
    }

    match fdlimit::raise_fd_limit() {
        Ok(Outcome::LimitRaised { from, to }) => debug!(from, to, "raised fd limit"),
        Ok(Outcome::Unsupported) => warn!("raising fd limit is not supported"),
        Err(err) => warn!("{}", err.error_chain()),
    }
}
