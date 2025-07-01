// Copyright (C) 2025  Braiins Systems s.r.o.

use embedded_hal::spi::{Error as HalError, ErrorType, SpiBus};
use spidev::{Spidev, SpidevTransfer};
use std::io::{Read, Write};

#[derive(Debug)]
#[expect(dead_code)]
pub struct SpiWrapperError(pub std::io::Error);

impl From<std::io::Error> for SpiWrapperError {
    fn from(e: std::io::Error) -> Self {
        Self(e)
    }
}

impl HalError for SpiWrapperError {
    fn kind(&self) -> embedded_hal::spi::ErrorKind {
        // Můžeš později rozlišovat podle `self.0.kind()`
        embedded_hal::spi::ErrorKind::Other
    }
}

pub struct SpidevHalWrapper(pub Spidev);

impl ErrorType for SpidevHalWrapper {
    type Error = SpiWrapperError;
}

impl SpiBus<u8> for SpidevHalWrapper {
    fn read(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        self.0.read_exact(buf)?;
        Ok(())
    }

    fn write(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        self.0.write_all(buf)?;
        Ok(())
    }

    fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        if read.len() != write.len() {
            return Err(SpiWrapperError(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "read/write length mismatch",
            )));
        }

        let mut transfer = SpidevTransfer::read_write(write, read);
        self.0.transfer(&mut transfer)?;
        Ok(())
    }

    fn transfer_in_place(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        let (ptr, len) = (buf.as_mut_ptr(), buf.len());
        let write = unsafe { std::slice::from_raw_parts(ptr, len) };
        let read = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        let mut transfer = SpidevTransfer::read_write(write, read);
        self.0.transfer(&mut transfer)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
