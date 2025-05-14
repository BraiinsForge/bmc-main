// Copyright (C) 2025  Braiins Systems s.r.o.

use tokio::process::Command;

#[expect(clippy::enum_variant_names)]
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("FromUtf8Error: {0}")]
    FromUtf8Error(#[from] std::string::FromUtf8Error),
    #[error("System command call failed: {0}")]
    Cmd(String),
}

#[expect(clippy::needless_pass_by_value)]
fn create_command<T>(command_name: T, args: &[T]) -> Command
where
    T: ToString,
{
    let mut cmd = Command::new(command_name.to_string());
    for arg in args {
        cmd.arg(arg.to_string());
    }
    cmd
}

pub async fn call_command<T>(command_name: T, args: &[T]) -> Result<Vec<u8>, Error>
where
    T: ToString,
{
    let cmd = create_command(command_name, args).output().await?;
    if !cmd.status.success() {
        return Err(Error::Cmd(String::from_utf8(cmd.stderr)?));
    }
    Ok(cmd.stdout)
}

pub async fn call_command_to_string<T>(command_name: T, args: &[T]) -> Result<String, Error>
where
    T: ToString,
{
    let res = call_command(command_name, args).await?;
    Ok(String::from_utf8(res)?)
}
