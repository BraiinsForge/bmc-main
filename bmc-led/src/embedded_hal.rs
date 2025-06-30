// Embedded-HAL wrapper for Spidev
use embedded_hal::spi::ErrorType;
use embedded_hal::spi::SpiBus;
use spidev::Spidev;

pub struct SpidevHalWrapper(pub Spidev);

impl ErrorType for SpidevHalWrapper {
    type Error = core::convert::Infallible;
}

impl SpiBus<u8> for SpidevHalWrapper {
    fn read(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        use std::io::Read;
        self.0.read_exact(buf).unwrap();
        Ok(())
    }

    fn write(&mut self, buf: &[u8]) -> Result<(), Self::Error> {
        use std::io::Write;
        self.0.write_all(buf).unwrap();
        Ok(())
    }

    fn transfer(&mut self, read: &mut [u8], write: &[u8]) -> Result<(), Self::Error> {
        use spidev::SpidevTransfer;
        assert_eq!(read.len(), write.len());
        let mut transfer = SpidevTransfer::read_write(write, read);
        self.0.transfer(&mut transfer).unwrap();
        Ok(())
    }

    fn transfer_in_place(&mut self, buf: &mut [u8]) -> Result<(), Self::Error> {
        use spidev::SpidevTransfer;
        let (ptr, len) = (buf.as_mut_ptr(), buf.len());
        // Safety: no aliasing of buf while creating temporary slices
        let write = unsafe { std::slice::from_raw_parts(ptr, len) };
        let read = unsafe { std::slice::from_raw_parts_mut(ptr, len) };
        let mut transfer = SpidevTransfer::read_write(write, read);
        self.0.transfer(&mut transfer).unwrap();
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
