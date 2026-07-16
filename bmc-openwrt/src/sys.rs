// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use tokio::process::Command;

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
