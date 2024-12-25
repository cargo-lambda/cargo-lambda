use cargo_lambda_remote::RemoteConfig;
use clap::{ArgAction, Args, ValueHint};
use serde::{ser::SerializeStruct, Deserialize, Serialize};
use std::{collections::HashMap, fmt::Debug, path::PathBuf};
use strum_macros::{Display, EnumString};

use crate::{
    cargo::deserialize_vec_or_map,
    env::{EnvOptions, Environment},
    error::MetadataError,
    lambda::{Memory, Timeout, Tracing},
};

const DEFAULT_MANIFEST_PATH: &str = "Cargo.toml";
const DEFAULT_COMPATIBLE_RUNTIMES: &str = "provided.al2,provided.al2023";
const DEFAULT_RUNTIME: &str = "provided.al2023";

#[derive(Args, Clone, Debug, Default, Deserialize)]
#[command(
    name = "deploy",
    after_help = "Full command documentation: https://www.cargo-lambda.info/commands/deploy.html"
)]
pub struct Deploy {
    #[command(flatten)]
    #[serde(flatten)]
    pub remote_config: RemoteConfig,

    #[command(flatten)]
    #[serde(flatten)]
    pub function_config: FunctionDeployConfig,

    /// Directory where the lambda binaries are located
    #[arg(short, long, value_hint = ValueHint::DirPath)]
    #[serde(default)]
    pub lambda_dir: Option<PathBuf>,

    /// Path to Cargo.toml
    #[arg(long, value_name = "PATH", default_value = DEFAULT_MANIFEST_PATH)]
    #[serde(default)]
    pub manifest_path: Option<PathBuf>,

    /// Name of the binary to deploy if it doesn't match the name that you want to deploy it with
    #[arg(long, conflicts_with = "binary_path")]
    #[serde(default)]
    pub binary_name: Option<String>,

    /// Local path of the binary to deploy if it doesn't match the target path generated by cargo-lambda-build
    #[arg(long, conflicts_with = "binary_name")]
    #[serde(default)]
    pub binary_path: Option<PathBuf>,

    /// S3 bucket to upload the code to
    #[arg(long)]
    #[serde(default)]
    pub s3_bucket: Option<String>,

    /// Name with prefix where the code will be uploaded to in S3
    #[arg(long)]
    #[serde(default)]
    pub s3_key: Option<String>,

    /// Whether the code that you're deploying is a Lambda Extension
    #[arg(long)]
    #[serde(default)]
    pub extension: bool,

    /// Whether an extension is internal or external
    #[arg(long, requires = "extension")]
    #[serde(default)]
    pub internal: bool,

    /// Comma separated list with compatible runtimes for the Lambda Extension (--compatible_runtimes=provided.al2,nodejs16.x)
    /// List of allowed runtimes can be found in the AWS documentation: https://docs.aws.amazon.com/lambda/latest/dg/API_CreateFunction.html#SSS-CreateFunction-request-Runtime
    #[arg(
        long,
        value_delimiter = ',',
        default_value = DEFAULT_COMPATIBLE_RUNTIMES,
        requires = "extension"
    )]
    #[serde(default)]
    compatible_runtimes: Option<Vec<String>>,

    /// Format to render the output (text, or json)
    #[arg(short, long)]
    #[serde(default)]
    output_format: Option<OutputFormat>,

    /// Comma separated list of tags to apply to the function or extension (--tag organization=aws,team=lambda).
    /// It can be used multiple times to add more tags. (--tag organization=aws --tag team=lambda)
    #[arg(long, value_delimiter = ',', action = ArgAction::Append, visible_alias = "tags")]
    #[serde(default, alias = "tags", deserialize_with = "deserialize_vec_or_map")]
    pub tag: Option<Vec<String>>,

    /// Option to add one or more files and directories to include in the zip file to upload.
    #[arg(short, long)]
    #[serde(default)]
    pub include: Option<Vec<String>>,

    /// Perform all the operations to locate and package the binary to deploy, but don't do the final deploy.
    #[arg(long, alias = "dry-run")]
    #[serde(default)]
    pub dry: bool,

    /// Name of the function or extension to deploy
    #[arg(value_name = "NAME")]
    #[serde(default)]
    pub name: Option<String>,
}

impl Deploy {
    pub fn manifest_path(&self) -> PathBuf {
        self.manifest_path
            .clone()
            .unwrap_or_else(default_manifest_path)
    }

    pub fn output_format(&self) -> OutputFormat {
        self.output_format.clone().unwrap_or_default()
    }

    pub fn compatible_runtimes(&self) -> Vec<String> {
        self.compatible_runtimes
            .clone()
            .unwrap_or_else(default_compatible_runtimes)
    }
}

impl Serialize for Deploy {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;

        let len = self.manifest_path.is_some() as usize
            + self.lambda_dir.is_some() as usize
            + self.binary_path.is_some() as usize
            + self.binary_name.is_some() as usize
            + self.s3_bucket.is_some() as usize
            + self.s3_key.is_some() as usize
            + self.extension as usize
            + self.internal as usize
            + self.compatible_runtimes.is_some() as usize
            + self.output_format.is_some() as usize
            + self.tag.is_some() as usize
            + self.include.is_some() as usize
            + self.dry as usize
            + self.name.is_some() as usize
            + self.remote_config.count_fields()
            + self.function_config.count_fields();

        let mut state = serializer.serialize_struct("Deploy", len)?;

        if let Some(ref path) = self.manifest_path {
            state.serialize_field("manifest_path", path)?;
        }
        if let Some(ref dir) = self.lambda_dir {
            state.serialize_field("lambda_dir", dir)?;
        }
        if let Some(ref path) = self.binary_path {
            state.serialize_field("binary_path", path)?;
        }
        if let Some(ref name) = self.binary_name {
            state.serialize_field("binary_name", name)?;
        }
        if let Some(ref bucket) = self.s3_bucket {
            state.serialize_field("s3_bucket", bucket)?;
        }
        if let Some(ref key) = self.s3_key {
            state.serialize_field("s3_key", key)?;
        }
        if self.extension {
            state.serialize_field("extension", &self.extension)?;
        }
        if self.internal {
            state.serialize_field("internal", &self.internal)?;
        }
        if let Some(ref runtimes) = self.compatible_runtimes {
            state.serialize_field("compatible_runtimes", runtimes)?;
        }
        if let Some(ref format) = self.output_format {
            state.serialize_field("output_format", format)?;
        }
        if let Some(ref tag) = self.tag {
            state.serialize_field("tag", tag)?;
        }
        if let Some(ref include) = self.include {
            state.serialize_field("include", include)?;
        }
        if self.dry {
            state.serialize_field("dry", &self.dry)?;
        }
        if let Some(ref name) = self.name {
            state.serialize_field("name", name)?;
        }

        self.remote_config.serialize_fields::<S>(&mut state)?;
        self.function_config.serialize_fields::<S>(&mut state)?;

        state.end()
    }
}

fn default_manifest_path() -> PathBuf {
    PathBuf::from(DEFAULT_MANIFEST_PATH)
}

fn default_compatible_runtimes() -> Vec<String> {
    DEFAULT_COMPATIBLE_RUNTIMES
        .split(',')
        .map(String::from)
        .collect()
}

#[derive(Clone, Debug, Default, Deserialize, Display, EnumString, Serialize)]
#[strum(ascii_case_insensitive)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    #[default]
    Text,
    Json,
}

#[derive(Args, Clone, Debug, Default, Deserialize, Serialize)]
pub struct FunctionDeployConfig {
    /// Enable function URL for this function
    #[arg(long)]
    #[serde(default)]
    pub enable_function_url: bool,

    /// Disable function URL for this function
    #[arg(long)]
    #[serde(default)]
    pub disable_function_url: bool,

    /// Memory allocated for the function
    #[arg(long, alias = "memory-size")]
    #[serde(default)]
    pub memory: Option<Memory>,

    /// How long the function can be running for, in seconds
    #[arg(long)]
    #[serde(default)]
    pub timeout: Option<Timeout>,

    #[command(flatten)]
    #[serde(flatten)]
    pub env_options: Option<EnvOptions>,

    /// Tracing mode with X-Ray
    #[arg(long)]
    #[serde(default)]
    pub tracing: Option<Tracing>,

    /// IAM Role associated with the function
    #[arg(long, visible_alias = "iam-role")]
    #[serde(default, alias = "iam_role")]
    pub role: Option<String>,

    /// Lambda Layer ARN to associate the deployed function with.
    /// Can be used multiple times to add more layers.
    /// `--layer arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer1 --layer arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer2`.
    /// It can also be used with comma separated list of layer ARNs.
    /// `--layer arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer1,arn:aws:lambda:us-east-1:xxxxxxxx:layers:layer2`.
    #[arg(long, value_delimiter = ',', action = ArgAction::Append, visible_alias = "layer-arn")]
    #[serde(default, alias = "layers")]
    pub layer: Option<Vec<String>>,

    #[command(flatten)]
    #[serde(flatten)]
    pub vpc: Option<VpcConfig>,

    /// Choose a different Lambda runtime to deploy with.
    /// The only other option that might work is `provided.al2`.
    #[arg(long, default_value = DEFAULT_RUNTIME)]
    #[serde(default)]
    pub runtime: Option<String>,
}

fn default_runtime() -> String {
    DEFAULT_RUNTIME.to_string()
}

impl FunctionDeployConfig {
    pub fn runtime(&self) -> String {
        self.runtime.clone().unwrap_or_else(default_runtime)
    }

    pub fn should_update(&self) -> bool {
        let Ok(val) = serde_json::to_value(self) else {
            return false;
        };
        let Ok(default) = serde_json::to_value(FunctionDeployConfig::default()) else {
            return false;
        };
        val != default
    }

    fn count_fields(&self) -> usize {
        self.disable_function_url as usize
            + self.enable_function_url as usize
            + self.layer.as_ref().is_some_and(|l| !l.is_empty()) as usize
            + self.tracing.is_some() as usize
            + self.role.is_some() as usize
            + self.memory.is_some() as usize
            + self.timeout.is_some() as usize
            + self.runtime.is_some() as usize
            + self.vpc.as_ref().map_or(0, |vpc| vpc.count_fields())
            + self
                .env_options
                .as_ref()
                .map_or(0, |env| env.count_fields())
    }

    fn serialize_fields<S>(
        &self,
        state: &mut <S as serde::Serializer>::SerializeStruct,
    ) -> Result<(), S::Error>
    where
        S: serde::Serializer,
    {
        if self.disable_function_url {
            state.serialize_field("disable_function_url", &true)?;
        }

        if self.enable_function_url {
            state.serialize_field("enable_function_url", &true)?;
        }

        if let Some(memory) = &self.memory {
            state.serialize_field("memory", &memory)?;
        }

        if let Some(timeout) = &self.timeout {
            state.serialize_field("timeout", &timeout)?;
        }

        if let Some(runtime) = &self.runtime {
            state.serialize_field("runtime", &runtime)?;
        }

        if let Some(tracing) = &self.tracing {
            state.serialize_field("tracing", &tracing)?;
        }

        if let Some(role) = &self.role {
            state.serialize_field("role", &role)?;
        }

        if let Some(layer) = &self.layer {
            if !layer.is_empty() {
                state.serialize_field("layer", &layer)?;
            }
        }

        if let Some(vpc) = &self.vpc {
            vpc.serialize_fields::<S>(state)?;
        }

        if let Some(env_options) = &self.env_options {
            env_options.serialize_fields::<S>(state)?;
        }

        Ok(())
    }
}

#[derive(Args, Clone, Debug, Default, Deserialize, Serialize)]
pub struct VpcConfig {
    /// Subnet IDs to associate the deployed function with a VPC
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub subnet_ids: Option<Vec<String>>,

    /// Security Group IDs to associate the deployed function
    #[arg(long, value_delimiter = ',')]
    #[serde(default)]
    pub security_group_ids: Option<Vec<String>>,

    /// Allow outbound IPv6 traffic on VPC functions that are connected to dual-stack subnets
    #[arg(long)]
    #[serde(default)]
    pub ipv6_allowed_for_dual_stack: bool,
}

impl VpcConfig {
    fn count_fields(&self) -> usize {
        self.subnet_ids.is_some() as usize
            + self.security_group_ids.is_some() as usize
            + self.ipv6_allowed_for_dual_stack as usize
    }

    fn serialize_fields<S>(
        &self,
        state: &mut <S as serde::Serializer>::SerializeStruct,
    ) -> Result<(), S::Error>
    where
        S: serde::Serializer,
    {
        if let Some(subnet_ids) = &self.subnet_ids {
            state.serialize_field("subnet_ids", &subnet_ids)?;
        }
        if let Some(security_group_ids) = &self.security_group_ids {
            state.serialize_field("security_group_ids", &security_group_ids)?;
        }
        state.serialize_field(
            "ipv6_allowed_for_dual_stack",
            &self.ipv6_allowed_for_dual_stack,
        )?;
        Ok(())
    }

    pub fn should_update(&self) -> bool {
        let Ok(val) = serde_json::to_value(self) else {
            return false;
        };
        let Ok(default) = serde_json::to_value(VpcConfig::default()) else {
            return false;
        };
        val != default
    }
}

impl Deploy {
    pub fn lambda_tags(&self) -> Option<HashMap<String, String>> {
        match &self.tag {
            None => None,
            Some(tags) if tags.is_empty() => None,
            Some(tags) => Some(extract_tags(tags)),
        }
    }

    pub fn s3_tags(&self) -> Option<String> {
        match &self.tag {
            None => None,
            Some(tags) if tags.is_empty() => None,
            Some(tags) => Some(tags.join("&")),
        }
    }

    pub fn environment(
        &self,
        base: &HashMap<String, String>,
    ) -> Result<Environment, MetadataError> {
        let Some(env_options) = &self.function_config.env_options else {
            return Ok(HashMap::new());
        };
        env_options.lambda_environment(base)
    }
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
