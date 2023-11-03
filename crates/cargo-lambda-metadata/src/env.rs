use aws_sdk_lambda::types::{builders::EnvironmentBuilder, Environment};
use clap::{Args, ValueHint};
use env_file_reader::read_file;
use miette::Result;
use std::{collections::HashMap, path::PathBuf};

use crate::error::MetadataError;

#[derive(Args, Clone, Debug, Default)]
pub struct EnvOptions {
    /// Option to add one or many environment variables, allows multiple repetitions (--env-var KEY=VALUE --env-var OTHER=NEW-VALUE)
    /// This option overrides any values set with the --env-vars flag.
    #[arg(long)]
    pub env_var: Option<Vec<String>>,

    /// Command separated list of environment variables (--env-vars KEY=VALUE,OTHER=NEW-VALUE)
    /// This option overrides any values set with the --env-var flag that match the same key.
    #[arg(long, value_delimiter = ',')]
    pub env_vars: Option<Vec<String>>,

    /// Read environment variables from a file.
    /// Variables are separated by new lines in KEY=VALUE format.
    #[arg(long, value_hint = ValueHint::FilePath)]
    pub env_file: Option<PathBuf>,
}

impl EnvOptions {
    pub fn flag_vars(&self) -> Option<Vec<String>> {
        match (&self.env_var, &self.env_vars) {
            (None, None) => None,
            (Some(v), None) => Some(v.clone()),
            (None, Some(v)) => Some(v.clone()),
            (Some(v1), Some(v2)) => {
                let mut base = v1.clone();
                base.extend(v2.clone());
                Some(base)
            }
        }
    }

    pub fn lambda_environment(&self) -> Result<Environment, MetadataError> {
        lambda_environment(None, &self.env_file, self.flag_vars()).map(|e| e.build())
    }
}

pub(crate) fn lambda_environment(
    base: Option<&HashMap<String, String>>,
    env_file: &Option<PathBuf>,
    vars: Option<Vec<String>>,
) -> Result<EnvironmentBuilder, MetadataError> {
    let mut env = Environment::builder().set_variables(base.cloned());

    if let Some(path) = env_file {
        if path.is_file() {
            let env_variables =
                read_file(path).map_err(|e| MetadataError::InvalidEnvFile(path.into(), e))?;
            for (key, value) in env_variables {
                env = env.variables(key, value);
            }
        }
    }

    if let Some(vars) = vars {
        for var in vars {
            let (key, value) = extract_var(&var)?;
            env = env.variables(key, value);
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
        let env = lambda_environment(None, &None, None).unwrap().build();
        assert_eq!(None, env.variables());
    }

    #[test]
    fn test_base_environment() {
        let mut base = HashMap::new();
        base.insert("FOO".into(), "BAR".into());
        let env = lambda_environment(Some(&base), &None, None)
            .unwrap()
            .build();

        assert_eq!("BAR".to_string(), env.variables().unwrap()["FOO"]);
    }

    #[test]
    fn test_environment_with_flags() {
        let mut base = HashMap::new();
        base.insert("FOO".into(), "BAR".into());

        let flags = vec!["FOO=QUX".to_string(), "BAZ=QUUX".to_string()];
        let env = lambda_environment(Some(&base), &None, Some(flags))
            .unwrap()
            .build();

        assert_eq!("QUX".to_string(), env.variables().unwrap()["FOO"]);
        assert_eq!("QUUX".to_string(), env.variables().unwrap()["BAZ"]);
    }

    #[test]
    fn test_environment_with_file() {
        let file = temp_dir().join(".env");
        std::fs::write(&file, "BAR=BAZ\n\nexport QUUX = 'QUUUX'\n#IGNORE=ME").unwrap();

        let mut base = HashMap::new();
        base.insert("FOO".into(), "BAR".into());

        let flags = vec!["FOO=QUX".to_string(), "BAZ=QUUX".to_string()];
        let env = lambda_environment(Some(&base), &Some(file), Some(flags))
            .unwrap()
            .build();

        let vars = env.variables().unwrap();

        assert_eq!("QUX".to_string(), vars["FOO"]);
        assert_eq!("QUUX".to_string(), vars["BAZ"]);
        assert_eq!("BAZ".to_string(), vars["BAR"]);
        assert_eq!("QUUUX".to_string(), vars["QUUX"]);
        assert!(!vars.contains_key("IGNORE"));
        assert!(!vars.contains_key(""));
    }
}
