use clap::{
    Arg, Command,
    builder::TypedValueParser,
    error::{ContextKind, ContextValue},
};
use serde::{
    Deserialize, Serialize, Serializer,
    de::{Deserializer, Error, Visitor},
};
use std::{ffi::OsStr, fmt, str::FromStr, time::Duration};
use strum::{Display, EnumString};

use crate::error::MetadataError;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
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

impl std::fmt::Display for Timeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
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

impl From<&Timeout> for i32 {
    fn from(t: &Timeout) -> i32 {
        t.0 as i32
    }
}

impl From<i32> for Timeout {
    fn from(t: i32) -> Timeout {
        Timeout(t as u32)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Memory(u32);

impl std::fmt::Display for Memory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<Memory> for i32 {
    fn from(m: Memory) -> i32 {
        (&m).into()
    }
}

impl From<&Memory> for i32 {
    fn from(m: &Memory) -> i32 {
        m.0 as i32
    }
}

impl From<i32> for Memory {
    fn from(t: i32) -> Memory {
        Memory(t as u32)
    }
}

impl TryFrom<i64> for Memory {
    type Error = MetadataError;

    fn try_from(m: i64) -> Result<Memory, Self::Error> {
        if !(128..=10240).contains(&m) {
            return Err(MetadataError::InvalidMemory(format!("{m}")));
        }
        Ok(Memory(m as u32))
    }
}

impl FromStr for Memory {
    type Err = MetadataError;

    fn from_str(t: &str) -> Result<Memory, Self::Err> {
        let t = i64::from_str(t).map_err(|_| MetadataError::InvalidMemory(t.into()))?;

        t.try_into()
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
                Memory::try_from(value).map_err(|e| Error::custom(e.to_string()))
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

impl Serialize for Memory {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_i32(self.into())
    }
}

#[derive(Clone)]
pub struct MemoryValueParser;
impl TypedValueParser for MemoryValueParser {
    type Value = Memory;

    fn parse_ref(
        &self,
        cmd: &Command,
        arg: Option<&Arg>,
        value: &OsStr,
    ) -> Result<Self::Value, clap::Error> {
        let val = value
            .to_str()
            .ok_or_else(|| clap::Error::new(clap::error::ErrorKind::InvalidUtf8).with_cmd(cmd))?;

        Memory::from_str(val).map_err(|_| {
            let mut err = clap::Error::new(clap::error::ErrorKind::ValueValidation).with_cmd(cmd);

            if let Some(arg) = arg {
                err.insert(
                    ContextKind::InvalidArg,
                    ContextValue::String(arg.to_string()),
                );
            }

            let context = ContextValue::String(val.to_string());
            err.insert(ContextKind::InvalidValue, context);

            err
        })
    }
}

#[derive(Clone, Debug, Default, Display, EnumString, Eq, PartialEq, Serialize)]
#[strum(ascii_case_insensitive)]
pub enum Tracing {
    Active,
    #[default]
    PassThrough,
}

impl Tracing {
    pub fn as_str(&self) -> &str {
        match self {
            Tracing::Active => "Active",
            Tracing::PassThrough => "PassThrough",
        }
    }
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
