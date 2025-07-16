// Copyright (C) 2025  Braiins Systems s.r.o.

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
