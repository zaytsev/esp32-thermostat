use core::array::TryFromSliceError;

use defmt::Format;
use embedded_storage::{ReadStorage, Storage};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, Format)]
pub struct HeaterConfig {
    pub target_temp: f32,
    pub temp_tolerance: f32,
}

impl HeaterConfig {
    pub const DEFAULT_TARGET_TEMPERATURE: f32 = 19.0;
    pub const DEFAULT_TEMPERATURE_TOLERANCE: f32 = 0.5;
}

impl Default for HeaterConfig {
    fn default() -> Self {
        Self {
            target_temp: Self::DEFAULT_TARGET_TEMPERATURE,
            temp_tolerance: Self::DEFAULT_TEMPERATURE_TOLERANCE,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, Format)]
pub struct Config {
    pub heater: HeaterConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            heater: HeaterConfig::default(),
        }
    }
}

#[derive(Format)]
pub enum Error {
    FormatError,
    InvalidMagic,
    StoredDataTooBig(usize),
    PayloadTooBig(usize),
    ReadError,
    WriteError,
    InvalidLength,
}

impl From<postcard::Error> for Error {
    fn from(_value: postcard::Error) -> Self {
        Self::FormatError
    }
}

impl From<TryFromSliceError> for Error {
    fn from(_value: TryFromSliceError) -> Self {
        Self::InvalidLength
    }
}

impl Config {
    const MAGIC: [u8; 2] = *b"C1";
    const LENGTH_LEN: usize = 2;
    const HEADER_LEN: usize = Self::MAGIC.len() + Self::LENGTH_LEN;
    const MAX_SERIALIZED: usize = 124; // plenty for this struct
    const TOTAL_BUF: usize = Self::HEADER_LEN + Self::MAX_SERIALIZED;

    pub fn try_load<S: ReadStorage>(storage: &mut S) -> Result<Self, Error> {
        let mut buf = [0u8; Self::TOTAL_BUF];

        storage.read(0, &mut buf).map_err(|_| Error::ReadError)?;

        if &buf[0..Self::MAGIC.len()] != &Self::MAGIC {
            return Err(Error::InvalidMagic);
        }

        let len = u16::from_le_bytes(buf[Self::MAGIC.len()..Self::HEADER_LEN].try_into()?) as usize;

        if len > Self::MAX_SERIALIZED {
            return Err(Error::StoredDataTooBig(len));
        }

        let result = postcard::from_bytes::<Config>(&buf[Self::HEADER_LEN..][..len])?;
        Ok(result)
    }

    pub fn load<S>(storage: &mut S) -> Self
    where
        S: ReadStorage,
    {
        Self::try_load(storage).unwrap_or_default()
    }

    pub fn save<S: Storage>(&self, storage: &mut S) -> Result<(), Error> {
        let mut buf = [0xffu8; Self::TOTAL_BUF];

        let mut payload_buf = &mut buf[Self::HEADER_LEN..];

        let len = postcard::to_slice(self, &mut payload_buf)?.len();
        if len > Self::MAX_SERIALIZED {
            return Err(Error::PayloadTooBig(len));
        }

        buf[0..Self::MAGIC.len()].copy_from_slice(&Self::MAGIC);
        buf[Self::MAGIC.len()..Self::HEADER_LEN].copy_from_slice(&(len as u16).to_le_bytes());

        storage.write(0, &buf).map_err(|_| Error::WriteError)?;
        Ok(())
    }
}
