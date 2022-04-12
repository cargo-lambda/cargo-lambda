use aws_config::{
    meta::region::RegionProviderChain, profile::ProfileFileCredentialsProvider,
    provider_config::ProviderConfig,
};
use aws_sdk_lambda::{Client, Region, RetryConfig};
use clap::Args;

const DEFAULT_REGION: &str = "us-east-1";

#[derive(Args, Clone, Debug)]
pub struct RemoteConfig {
    /// AWS configuration profile to use for authorization
    #[clap(short, long)]
    pub profile: Option<String>,

    /// AWS region to deploy, if there is no default
    #[clap(short, long)]
    pub region: Option<String>,

    /// IAM Role associated with the function
    #[clap(long)]
    pub iam_role: Option<String>,

    /// AWS Lambda alias to associate the function to
    #[clap(short, long)]
    pub alias: Option<String>,

    /// Number of attempts to try failed operations, default 1
    #[clap(long, default_value = "1")]
    retry_attempts: u32,
}

/// Initialize an AWS Lambda client.
/// Uses us-east-1 as the default region if no region is provided.
pub async fn init_client(config: &RemoteConfig) -> Client {
    let region_provider = RegionProviderChain::first_try(config.region.clone().map(Region::new))
        .or_default_provider()
        .or_else(Region::new(DEFAULT_REGION));
    let region = region_provider.region().await;

    let mut config_loader = aws_config::from_env()
        .region(region_provider)
        .retry_config(RetryConfig::default().with_max_attempts(config.retry_attempts));

    if let Some(profile) = &config.profile {
        let conf = ProviderConfig::without_region().with_region(region);
        let creds_provider = ProfileFileCredentialsProvider::builder()
            .profile_name(profile)
            .configure(&conf)
            .build();
        config_loader = config_loader.credentials_provider(creds_provider);
    }

    let config = config_loader.load().await;
    Client::new(&config)
}

pub use aws_sdk_lambda;
