use serde::Deserialize;
use std::{
    collections::HashMap,
    fmt::Debug,
    path::{Path, PathBuf},
};
use urlencoding::encode;

use crate::{
    cargo::{load_metadata, Metadata},
    env::{lambda_environment, Environment},
    error::MetadataError,
    lambda::{Memory, Timeout, Tracing},
};

#[derive(Clone, Debug, Default, Deserialize)]
pub struct DeployConfig {
    #[serde(default)]
    pub memory: Option<Memory>,
    #[serde(default)]
    pub timeout: Option<Timeout>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub env_file: Option<PathBuf>,
    #[serde(default)]
    pub tracing: Tracing,
    #[serde(default, alias = "role")]
    pub iam_role: Option<String>,
    #[serde(default)]
    pub layers: Option<Vec<String>>,
    #[serde(default)]
    pub tags: Option<HashMap<String, String>>,
    #[serde(skip)]
    pub use_for_update: bool,
    #[serde(default = "default_runtime")]
    pub runtime: String,
    #[serde(default)]
    pub include: Option<Vec<String>>,
    #[serde(default)]
    pub s3_bucket: Option<String>,
    #[serde(default)]
    pub s3_key: Option<String>,
    #[serde(flatten)]
    pub vpc: Option<VpcConfig>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct VpcConfig {
    #[serde(default)]
    pub subnet_ids: Option<Vec<String>>,
    #[serde(default)]
    pub security_group_ids: Option<Vec<String>>,
    #[serde(default)]
    pub ipv6_allowed_for_dual_stack: bool,
}

impl VpcConfig {
    pub fn build(
        subnet_ids: Option<Vec<String>>,
        security_group_ids: Option<Vec<String>>,
        ipv6_allowed_for_dual_stack: bool,
    ) -> Option<Self> {
        if subnet_ids.is_some() || security_group_ids.is_some() || ipv6_allowed_for_dual_stack {
            Some(Self {
                subnet_ids,
                security_group_ids,
                ipv6_allowed_for_dual_stack,
            })
        } else {
            None
        }
    }
}

fn default_runtime() -> String {
    "provided.al2023".to_string()
}

impl DeployConfig {
    pub fn append_tags(&mut self, tags: HashMap<String, String>) {
        self.tags = match &self.tags {
            None => Some(tags),
            Some(base) => {
                let mut new_tags = base.clone();
                new_tags.extend(tags);
                Some(new_tags)
            }
        }
    }

    pub fn s3_tags(&self) -> Option<String> {
        match &self.tags {
            None => None,
            Some(tags) if tags.is_empty() => None,
            Some(tags) => {
                let mut vec = Vec::new();
                for (k, v) in tags {
                    vec.push(format!("{}={}", encode(k), encode(v)));
                }
                Some(vec.join("&"))
            }
        }
    }

    pub fn lambda_environment(&self) -> Result<Environment, MetadataError> {
        let base = if self.env.is_empty() {
            None
        } else {
            Some(&self.env)
        };
        lambda_environment(base, &self.env_file, None)
    }

    pub fn extend_environment(
        &mut self,
        extra: &HashMap<String, String>,
    ) -> Result<Environment, MetadataError> {
        let mut env = lambda_environment(Some(&self.env), &self.env_file, None)?;
        for (key, value) in extra {
            env.insert(key.clone(), value.clone());
        }
        Ok(env)
    }
}

pub fn merge_deploy_config(base: &mut DeployConfig, package_deploy: &Option<DeployConfig>) {
    let Some(package_deploy) = package_deploy else {
        return;
    };

    if package_deploy.memory.is_some() {
        base.memory.clone_from(&package_deploy.memory);
    }
    if let Some(package_timeout) = &package_deploy.timeout {
        if !package_timeout.is_zero() {
            base.timeout = Some(package_timeout.clone());
        }
    }
    base.env.extend(package_deploy.env.clone());
    if package_deploy.env_file.is_some() && base.env_file.is_none() {
        base.env_file.clone_from(&package_deploy.env_file);
    }
    if package_deploy.tracing != Tracing::default() {
        base.tracing = package_deploy.tracing.clone();
    }
    if package_deploy.iam_role.is_some() {
        base.iam_role.clone_from(&package_deploy.iam_role);
    }
    if package_deploy.layers.is_some() {
        base.layers.clone_from(&package_deploy.layers);
    }

    if let Some(vpc) = &package_deploy.vpc {
        let mut base_vpc = base.vpc.clone().unwrap_or_default();
        if vpc.subnet_ids.is_some() {
            base_vpc.subnet_ids.clone_from(&vpc.subnet_ids);
        }
        if vpc.security_group_ids.is_some() {
            base_vpc
                .security_group_ids
                .clone_from(&vpc.security_group_ids);
        }
        if vpc.ipv6_allowed_for_dual_stack {
            base_vpc.ipv6_allowed_for_dual_stack = vpc.ipv6_allowed_for_dual_stack;
        }

        base.vpc = Some(base_vpc);
    }

    base.runtime.clone_from(&package_deploy.runtime);
    if let Some(package_include) = &package_deploy.include {
        let mut include = base.include.clone().unwrap_or_default();
        include.extend(package_include.clone());
        base.include = Some(include);
    }
    if package_deploy.s3_bucket.is_some() {
        base.s3_bucket.clone_from(&package_deploy.s3_bucket);
    }
    if let Some(package_tags) = &package_deploy.tags {
        let mut tags = base.tags.clone().unwrap_or_default();
        tags.extend(package_tags.clone());
        base.tags = Some(tags);
    }

    tracing::debug!(ws_metadata = ?base, package_metadata = ?package_deploy, "finished merging deploy metadata");
}

/// Create a `DeployConfig` struct from Cargo metadata.
/// This configuration can be overwritten by flags from the cli.
#[tracing::instrument(target = "cargo_lambda")]
pub fn function_deploy_metadata<P: AsRef<Path> + Debug>(
    manifest_path: P,
    name: &str,
    tags: &Option<Vec<String>>,
    s3_bucket: &Option<String>,
    s3_key: &Option<String>,
    default: DeployConfig,
) -> Result<DeployConfig, MetadataError> {
    let metadata = load_metadata(manifest_path)?;
    let ws_metadata: Metadata =
        serde_json::from_value(metadata.workspace_metadata).unwrap_or_default();

    let mut config = ws_metadata.lambda.package.deploy.unwrap_or(default);

    if let Some(package_metadata) = ws_metadata.lambda.bin.get(name) {
        merge_deploy_config(&mut config, &package_metadata.deploy);
    }

    for pkg in &metadata.packages {
        for target in &pkg.targets {
            let target_matches = target.name == name
                && target.kind.iter().any(|kind| kind == "bin")
                && pkg.metadata.is_object();

            tracing::debug!(name, target_matches, "searching package metadata");

            if target_matches {
                let package_metadata: Metadata = serde_json::from_value(pkg.metadata.clone())
                    .map_err(MetadataError::InvalidCargoMetadata)?;
                let package_config = package_metadata.lambda.package.deploy;
                merge_deploy_config(&mut config, &package_config);

                break;
            }
        }
    }

    if let Some(tags) = tags {
        config.append_tags(extract_tags(tags));
    }

    if config.s3_bucket.is_none() {
        config.s3_bucket.clone_from(s3_bucket);
    }

    if config.s3_key.is_none() {
        config.s3_key.clone_from(s3_key);
    }

    tracing::debug!(?config, "using deploy configuration from metadata");
    Ok(config)
}

fn extract_tags(tags: &Vec<String>) -> HashMap<String, String> {
    let mut map = HashMap::new();

    for var in tags {
        let mut split = var.splitn(2, '=');
        if let (Some(k), Some(v)) = (split.next(), split.next()) {
            map.insert(k.to_string(), v.to_string());
        }
    }

    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cargo::tests::fixture;

    #[test]
    fn test_deploy_metadata_packages() {
        let env = function_deploy_metadata(
            fixture("single-binary-package"),
            "basic-lambda",
            &None,
            &None,
            &None,
            DeployConfig::default(),
        )
        .unwrap();

        let layers = [
            "arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer1".to_string(),
            "arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer2".to_string(),
        ];

        let mut vars = HashMap::new();
        vars.insert("VAR1".to_string(), "VAL1".to_string());

        assert_eq!(Some(Memory::Mb512), env.memory);
        assert_eq!(Some(Timeout::new(60)), env.timeout);
        assert_eq!(Some(Path::new(".env.production")), env.env_file.as_deref());
        assert_eq!(Some(layers.to_vec()), env.layers);
        assert_eq!(Tracing::Active, env.tracing);
        assert_eq!(vars, env.env);
        assert_eq!(
            Some("arn:aws:lambda:us-east-1:xxxxxxxx:iam:role1".to_string()),
            env.iam_role
        );

        let mut tags = HashMap::new();
        tags.insert("organization".to_string(), "aws".to_string());
        tags.insert("team".to_string(), "lambda".to_string());

        assert_eq!(Some(tags), env.tags);
        let s3_tags = env.s3_tags().unwrap();
        assert_eq!(2, s3_tags.split('&').collect::<Vec<_>>().len());
        assert!(s3_tags.contains("organization=aws"), "{s3_tags}");
        assert!(s3_tags.contains("team=lambda"), "{s3_tags}");
    }

    #[test]
    fn test_deploy_metadata_packages_with_tags() {
        let tags = vec!["FOO=bar".into()];
        let env = function_deploy_metadata(
            fixture("single-binary-package"),
            "basic-lambda",
            &Some(tags),
            &None,
            &None,
            DeployConfig::default(),
        )
        .unwrap();

        let mut tags = HashMap::new();
        tags.insert("organization".to_string(), "aws".to_string());
        tags.insert("team".to_string(), "lambda".to_string());
        tags.insert("FOO".to_string(), "bar".to_string());

        assert_eq!(Some(tags), env.tags);
    }

    #[test]
    fn test_deploy_metadata_packages_with_s3_bucket() {
        let env = function_deploy_metadata(
            fixture("single-binary-package"),
            "basic-lambda",
            &None,
            &Some("deploy-bucket".into()),
            &None,
            DeployConfig::default(),
        )
        .unwrap();

        assert_eq!(Some("deploy-bucket".to_string()), env.s3_bucket);
    }

    #[test]
    fn test_deploy_metadata_packages_with_s3_bucket_and_key() {
        let env = function_deploy_metadata(
            fixture("single-binary-package"),
            "basic-lambda",
            &None,
            &Some("deploy-bucket".into()),
            &Some("prefix/name".into()),
            DeployConfig::default(),
        )
        .unwrap();

        assert_eq!(Some("deploy-bucket".to_string()), env.s3_bucket);
        assert_eq!(Some("prefix/name".to_string()), env.s3_key);
    }

    #[test]
    fn test_deploy_lambda_env() {
        let mut d = DeployConfig::default();
        let env = d.lambda_environment().unwrap();
        assert!(env.is_empty());

        let mut extra = HashMap::new();
        extra.insert("FOO".to_string(), "BAR".to_string());

        let vars = d.extend_environment(&extra).unwrap();
        assert_eq!(1, vars.len());
        assert_eq!("BAR", vars["FOO"]);

        let mut base = HashMap::new();
        base.insert("BAZ".to_string(), "QUX".to_string());
        d.env = base;

        let env = d.extend_environment(&extra).unwrap();
        assert_eq!(2, env.len());
        assert_eq!("BAR", env["FOO"]);
        assert_eq!("QUX", env["BAZ"]);
    }

    #[test]
    fn test_s3_tags_encoding() {
        let mut tags = HashMap::new();
        tags.insert(
            "organization".to_string(),
            "Amazon Web Services".to_string(),
        );
        tags.insert("team".to_string(), "Simple Storage Service".to_string());

        let config = DeployConfig {
            tags: Some(tags),
            ..Default::default()
        };

        let s3_tags = config.s3_tags().unwrap();
        assert_eq!(2, s3_tags.split('&').collect::<Vec<_>>().len());
        assert!(
            s3_tags.contains("organization=Amazon%20Web%20Services"),
            "{s3_tags}"
        );
        assert!(
            s3_tags.contains("team=Simple%20Storage%20Service"),
            "{s3_tags}"
        );
    }
}
