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

use anyhow::Context;
use embedded_hal::spi::{ErrorType, SpiBus};
use spidev::{Spidev, SpidevTransfer};
use std::io::{Read, Write};

#[derive(Debug)]
pub struct SpidevHalWrapper(pub Spidev);

#[derive(Debug)]
#[expect(dead_code)]
pub struct SpiError(pub anyhow::Error);

impl From<anyhow::Error> for SpiError {
    fn from(e: anyhow::Error) -> Self {
        SpiError(e)
    }
}

impl embedded_hal::spi::Error for SpiError {
    fn kind(&self) -> embedded_hal::spi::ErrorKind {
        embedded_hal::spi::ErrorKind::Other
    }
}

impl ErrorType for SpidevHalWrapper {
    type Error = SpiError;
}

impl SpiBus<u8> for SpidevHalWrapper {
    fn read(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        self.0
            .read_exact(buf)
            .context("SPI operation failed")
            .map_err(Into::into)
    }

    fn write(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        self.0
            .write_all(buf)
            .context("SPI operation failed")
            .map_err(Into::into)
    }

    fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        if read.len() != write.len() {
            return Err(SpiError(anyhow::anyhow!("read/write length mismatch")));
        }

        let mut transfer = SpidevTransfer::read_write(write, read);
        self.0
            .transfer(&mut transfer)
            .context("SPI operation failed")
            .map_err(Into::into)
    }

    fn transfer_in_place(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        let write_data = buf.to_vec();
        let mut transfer = SpidevTransfer::read_write(&write_data, buf);
        self.0
            .transfer(&mut transfer)
            .context("SPI operation failed")
            .map_err(Into::into)
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
