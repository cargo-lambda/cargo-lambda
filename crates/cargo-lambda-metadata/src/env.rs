use clap::{ArgAction, Args, ValueHint};
use env_file_reader::read_file;
use miette::Result;
use serde::{Deserialize, Serialize, ser::SerializeStruct};
use std::{collections::HashMap, env::VarError, path::PathBuf};

use crate::{cargo::deserialize_vec_or_map, error::MetadataError};

pub type Environment = HashMap<String, String>;

#[derive(Args, Clone, Debug, Default, Deserialize, Serialize)]
pub struct EnvOptions {
    /// Option to add one or many environment variables,
    /// allows multiple repetitions (--env-var KEY=VALUE --env-var OTHER=NEW-VALUE).
    /// It also allows to set a list of environment variables separated by commas
    /// (e.g. --env-var KEY=VALUE,OTHER=NEW-VALUE).
    #[arg(long, value_delimiter = ',', action = ArgAction::Append, visible_alias = "env-vars")]
    #[serde(default, alias = "env", deserialize_with = "deserialize_vec_or_map")]
    pub env_var: Option<Vec<String>>,

    /// Read environment variables from a file.
    /// Variables are separated by new lines in KEY=VALUE format.
    #[arg(long, value_hint = ValueHint::FilePath)]
    #[serde(default)]
    pub env_file: Option<PathBuf>,
}

impl EnvOptions {
    pub fn lambda_environment(
        &self,
        base: &HashMap<String, String>,
    ) -> Result<Environment, MetadataError> {
        lambda_environment(Some(base), &self.env_file, self.env_var.as_ref())
    }

    pub fn count_fields(&self) -> usize {
        self.env_var.is_some() as usize + self.env_file.is_some() as usize
    }

    pub fn serialize_fields<S>(
        &self,
        state: &mut <S as serde::Serializer>::SerializeStruct,
    ) -> Result<(), S::Error>
    where
        S: serde::Serializer,
    {
        if let Some(env_var) = &self.env_var {
            state.serialize_field("env_var", env_var)?;
        }
        if let Some(env_file) = &self.env_file {
            state.serialize_field("env_file", env_file)?;
        }
        Ok(())
    }
}

pub(crate) fn lambda_environment(
    base: Option<&HashMap<String, String>>,
    env_file: &Option<PathBuf>,
    vars: Option<&Vec<String>>,
) -> Result<Environment, MetadataError> {
    let mut env = HashMap::new();

    if let Some(base) = base.cloned() {
        env.extend(base);
    }

    if let Some(path) = env_file {
        if path.is_file() {
            let env_variables =
                read_file(path).map_err(|e| MetadataError::InvalidEnvFile(path.into(), e))?;
            for (key, value) in env_variables {
                env.insert(key, value);
            }
        }
    }

    if let Some(vars) = vars {
        for var in vars {
            let (key, value) = extract_var(var)?;
            env.insert(key.to_string(), value.to_string());
        }
    }

    Ok(env)
}

fn extract_var(line: &str) -> Result<(&str, &str), MetadataError> {
    let mut iter = line.trim().splitn(2, '=');

    let key = iter
        .next()
        .map(|s| s.trim())
        .ok_or_else(|| MetadataError::InvalidEnvVar(line.into()))?;
    if key.is_empty() {
        Err(MetadataError::InvalidEnvVar(line.into()))?;
    }

    let value = iter
        .next()
        .map(|s| s.trim())
        .ok_or_else(|| MetadataError::InvalidEnvVar(line.into()))?;
    if value.is_empty() {
        Err(MetadataError::InvalidEnvVar(line.into()))?;
    }

    Ok((key, value))
}

pub trait EnvVarExtractor {
    fn var(&self, name: &str) -> Result<String, VarError>;
}

pub struct SystemEnvExtractor;

impl EnvVarExtractor for SystemEnvExtractor {
    fn var(&self, name: &str) -> Result<String, VarError> {
        std::env::var(name)
    }
}

pub struct HashMapEnvExtractor {
    env: HashMap<String, String>,
}

impl From<Vec<(&str, &str)>> for HashMapEnvExtractor {
    fn from(env: Vec<(&str, &str)>) -> Self {
        Self {
            env: env
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }
}

impl EnvVarExtractor for HashMapEnvExtractor {
    fn var(&self, name: &str) -> Result<String, VarError> {
        self.env.get(name).cloned().ok_or(VarError::NotPresent)
    }
}

#[cfg(test)]
mod test {
    use std::env::temp_dir;

    use super::*;

    #[test]
    fn test_extract_var() {
        let (k, v) = extract_var("FOO=BAR").unwrap();
        assert_eq!("FOO", k);
        assert_eq!("BAR", v);

        let (k, v) = extract_var(" FOO = BAR ").unwrap();
        assert_eq!("FOO", k);
        assert_eq!("BAR", v);

        extract_var("=BAR").expect_err("missing key");
        extract_var("FOO=").expect_err("missing value");
        extract_var("  ").expect_err("missing variable");
    }

    #[test]
    fn test_empty_environment() {
        let env = lambda_environment(None, &None, None).unwrap();
        assert!(env.is_empty());
    }

    #[test]
    fn test_base_environment() {
        let mut base = HashMap::new();
        base.insert("FOO".into(), "BAR".into());
        let env = lambda_environment(Some(&base), &None, None).unwrap();

        assert_eq!("BAR".to_string(), env["FOO"]);
    }

    #[test]
    fn test_environment_with_flags() {
        let mut base = HashMap::new();
        base.insert("FOO".into(), "BAR".into());

        let flags = vec!["FOO=QUX".to_string(), "BAZ=QUUX".to_string()];
        let env = lambda_environment(Some(&base), &None, Some(&flags)).unwrap();

        assert_eq!("QUX".to_string(), env["FOO"]);
        assert_eq!("QUUX".to_string(), env["BAZ"]);
    }

    #[test]
    fn test_environment_with_file() {
        let file = temp_dir().join(".env");
        std::fs::write(&file, "BAR=BAZ\n\nexport QUUX = 'QUUUX'\n#IGNORE=ME").unwrap();

        let mut base = HashMap::new();
        base.insert("FOO".into(), "BAR".into());

        let flags = vec!["FOO=QUX".to_string(), "BAZ=QUUX".to_string()];
        let vars = lambda_environment(Some(&base), &Some(file), Some(&flags)).unwrap();

        assert_eq!("QUX".to_string(), vars["FOO"]);
        assert_eq!("QUUX".to_string(), vars["BAZ"]);
        assert_eq!("BAZ".to_string(), vars["BAR"]);
        assert_eq!("QUUUX".to_string(), vars["QUUX"]);
        assert!(!vars.contains_key("IGNORE"));
        assert!(!vars.contains_key(""));
    }
}
