use serde::{
    de::{Deserializer, Error, Visitor},
    Deserialize,
};
use std::{fmt, str::FromStr, time::Duration};
use strum_macros::{Display, EnumString};

use crate::error::MetadataError;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct Timeout(u32);

impl Timeout {
    pub fn new(i: u32) -> Timeout {
        Timeout(i)
    }

    pub fn is_zero(&self) -> bool {
        self.0 == 0
    }

    pub fn duration(&self) -> Duration {
        Duration::from_secs(self.0 as u64)
    }
}

impl Default for Timeout {
    fn default() -> Self {
        Timeout(30)
    }
}

impl FromStr for Timeout {
    type Err = MetadataError;

    fn from_str(t: &str) -> Result<Timeout, Self::Err> {
        let t = u32::from_str(t).map_err(MetadataError::InvalidTimeout)?;

        Ok(Timeout(t))
    }
}

impl From<Timeout> for i32 {
    fn from(t: Timeout) -> i32 {
        t.0 as i32
    }
}

#[derive(Clone, Debug, Display, EnumString, Eq, PartialEq)]
pub enum Memory {
    #[strum(to_string = "128")]
    Mb128,
    #[strum(to_string = "256")]
    Mb256,
    #[strum(to_string = "512")]
    Mb512,
    #[strum(to_string = "640")]
    Mb640,
    #[strum(to_string = "1024")]
    Mb1024,
    #[strum(to_string = "1536")]
    Mb1536,
    #[strum(to_string = "2048")]
    Mb2048,
    #[strum(to_string = "3072")]
    Mb3072,
    #[strum(to_string = "4096")]
    Mb4096,
    #[strum(to_string = "5120")]
    Mb5120,
    #[strum(to_string = "6144")]
    Mb6144,
    #[strum(to_string = "7168")]
    Mb7168,
    #[strum(to_string = "8192")]
    Mb8192,
    #[strum(to_string = "9216")]
    Mb9216,
    #[strum(to_string = "10240")]
    Mb10240,
}

impl From<Memory> for i32 {
    fn from(m: Memory) -> i32 {
        match m {
            Memory::Mb128 => 128,
            Memory::Mb256 => 256,
            Memory::Mb512 => 512,
            Memory::Mb640 => 640,
            Memory::Mb1024 => 1024,
            Memory::Mb1536 => 1536,
            Memory::Mb2048 => 2048,
            Memory::Mb3072 => 3072,
            Memory::Mb4096 => 4096,
            Memory::Mb5120 => 5120,
            Memory::Mb6144 => 6144,
            Memory::Mb7168 => 7168,
            Memory::Mb8192 => 8192,
            Memory::Mb9216 => 9216,
            Memory::Mb10240 => 10240,
        }
    }
}

impl TryFrom<i32> for Memory {
    type Error = MetadataError;

    fn try_from(m: i32) -> Result<Memory, Self::Error> {
        match m {
            128 => Ok(Memory::Mb128),
            256 => Ok(Memory::Mb256),
            512 => Ok(Memory::Mb512),
            640 => Ok(Memory::Mb640),
            1024 => Ok(Memory::Mb1024),
            1536 => Ok(Memory::Mb1536),
            2048 => Ok(Memory::Mb2048),
            3072 => Ok(Memory::Mb3072),
            4096 => Ok(Memory::Mb4096),
            5120 => Ok(Memory::Mb5120),
            6144 => Ok(Memory::Mb6144),
            7168 => Ok(Memory::Mb7168),
            8192 => Ok(Memory::Mb8192),
            9216 => Ok(Memory::Mb9216),
            10240 => Ok(Memory::Mb10240),
            other => Err(MetadataError::InvalidMemory(other)),
        }
    }
}

impl<'de> Deserialize<'de> for Memory {
    fn deserialize<D>(deserializer: D) -> Result<Memory, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MemoryVisitor;
        impl Visitor<'_> for MemoryVisitor {
            type Value = Memory;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an integer that matches Lambda's memory values")
            }

            fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Memory::try_from(value as i32).map_err(|e| Error::custom(e.to_string()))
            }

            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                self.visit_i64(value as i64)
            }
        }

        deserializer.deserialize_i64(MemoryVisitor)
    }
}

#[derive(Clone, Debug, Default, Display, EnumString, Eq, PartialEq)]
#[strum(ascii_case_insensitive)]
pub enum Tracing {
    Active,
    #[default]
    PassThrough,
}

impl TryFrom<String> for Tracing {
    type Error = MetadataError;

    fn try_from(s: String) -> Result<Tracing, Self::Error> {
        match s.to_lowercase().as_str() {
            "active" => Ok(Self::Active),
            "passthrough" => Ok(Self::PassThrough),
            _ => Err(MetadataError::InvalidTracing(s)),
        }
    }
}

impl<'de> Deserialize<'de> for Tracing {
    fn deserialize<D>(deserializer: D) -> Result<Tracing, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct TracingVisitor;
        impl Visitor<'_> for TracingVisitor {
            type Value = Tracing;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str(
                    "a string that matches Lambda's tracing options: `active` or `passthrough`",
                )
            }

            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
            where
                E: Error,
            {
                self.visit_string(v.to_string())
            }

            fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
            where
                E: Error,
            {
                Tracing::try_from(v).map_err(|e| Error::custom(e.to_string()))
            }
        }

        deserializer.deserialize_string(TracingVisitor)
    }
}
