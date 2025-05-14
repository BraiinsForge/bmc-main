// Copyright (C) 2025  Braiins Systems s.r.o.

use crate::sys;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Sys error: {0}")]
    Sys(#[from] sys::Error),
}

pub async fn call_command<T>(command_name: T, args: &[T]) -> Result<(), Error>
where
    T: ToString + Sync + Send,
{
    sys::call_command_to_string(command_name, args)
        .await
        .map(|_| ())
        .map_err(Into::into)
}
