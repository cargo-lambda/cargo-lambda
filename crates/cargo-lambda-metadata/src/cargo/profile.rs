use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct CargoProfile {
    pub release: Option<CargoProfileRelease>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
pub struct CargoProfileRelease {
    pub strip: Option<toml::Value>,
    pub lto: Option<toml::Value>,
    #[serde(rename = "codegen-units")]
    pub codegen_units: Option<toml::Value>,
    pub panic: Option<toml::Value>,
    #[serde(default = "default_cargo_bool")]
    pub debug: CargoBool,
}

impl CargoProfileRelease {
    pub fn debug_enabled(&self) -> bool {
        !(self.debug == CargoBool::Str("none".to_string())
            || self.debug == CargoBool::Num(0)
            || self.debug == CargoBool::Bool(false))
    }
}

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum CargoBool {
    Bool(bool),
    Num(u8),
    Str(String),
}

impl Default for CargoBool {
    fn default() -> Self {
        default_cargo_bool()
    }
}

fn default_cargo_bool() -> CargoBool {
    CargoBool::Bool(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cargo::*;

    #[test]
    fn test_release_config_exclude_strip() {
        let meta = Metadata {
            profile: Some(CargoProfile {
                release: Some(CargoProfileRelease {
                    strip: Some(toml::Value::String("none".into())),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let config = cargo_release_profile_config_from_metadata(meta);
        assert!(!config.contains(STRIP_CONFIG));

        let meta = Metadata {
            profile: Some(CargoProfile {
                release: Some(CargoProfileRelease {
                    debug: CargoBool::Bool(true),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let config = cargo_release_profile_config_from_metadata(meta);
        assert!(!config.contains(STRIP_CONFIG));
    }

    #[test]
    fn test_release_config_exclude_lto() {
        let meta = Metadata {
            profile: Some(CargoProfile {
                release: Some(CargoProfileRelease {
                    lto: Some(toml::Value::String("none".into())),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let config = cargo_release_profile_config_from_metadata(meta);
        assert!(!config.contains(LTO_CONFIG));
    }

    #[test]
    fn test_release_config_exclude_codegen() {
        let meta = Metadata {
            profile: Some(CargoProfile {
                release: Some(CargoProfileRelease {
                    codegen_units: Some(toml::Value::Integer(2)),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let config = cargo_release_profile_config_from_metadata(meta);
        assert!(!config.contains(CODEGEN_CONFIG));
    }

    #[test]
    fn test_release_config_exclude_panic() {
        let meta = Metadata {
            profile: Some(CargoProfile {
                release: Some(CargoProfileRelease {
                    panic: Some(toml::Value::String("none".into())),
                    ..Default::default()
                }),
            }),
            ..Default::default()
        };

        let config = cargo_release_profile_config_from_metadata(meta);
        assert!(!config.contains(PANIC_CONFIG));
    }

    #[test]
    fn test_release_debug_info() {
        let data = r#"
        [profile.release]
        overflow-checks = true
        debug = 1
        debug-assertions = false
        panic = "abort"
        lto = true
        "#;
        let metadata: Metadata = toml::from_str(data).unwrap();
        let profile = metadata.profile.unwrap().release.unwrap();
        assert!(profile.debug_enabled());
    }
}
