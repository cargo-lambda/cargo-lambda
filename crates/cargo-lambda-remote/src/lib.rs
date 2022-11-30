use aws_config::{
    meta::region::RegionProviderChain,
    profile::{ProfileFileCredentialsProvider, ProfileFileRegionProvider},
    provider_config::ProviderConfig,
};
use aws_sdk_lambda::RetryConfig;
use aws_types::{region::Region, SdkConfig};
use clap::Args;

const DEFAULT_REGION: &str = "us-east-1";

#[derive(Args, Clone, Debug)]
pub struct RemoteConfig {
    /// AWS configuration profile to use for authorization
    #[arg(short, long)]
    pub profile: Option<String>,

    /// AWS region to deploy, if there is no default
    #[arg(short, long)]
    pub region: Option<String>,

    /// AWS Lambda alias to associate the function to
    #[arg(short, long)]
    pub alias: Option<String>,

    /// Number of attempts to try failed operations
    #[arg(long, default_value = "1")]
    retry_attempts: u32,
}

impl RemoteConfig {
    pub async fn sdk_config(&self, retry: Option<RetryConfig>) -> SdkConfig {
        let explicit_region = self.region.clone().map(Region::new);

        let region_provider = RegionProviderChain::first_try(explicit_region.clone())
            .or_default_provider()
            .or_else(Region::new(DEFAULT_REGION));
        let region = region_provider.region().await;

        let retry =
            retry.unwrap_or_else(|| RetryConfig::default().with_max_attempts(self.retry_attempts));
        let mut config_loader = aws_config::from_env()
            .region(region_provider)
            .retry_config(retry);

        if let Some(profile) = &self.profile {
            let profile_region = ProfileFileRegionProvider::builder()
                .profile_name(profile)
                .build();

            let region_provider = RegionProviderChain::first_try(explicit_region)
                .or_else(profile_region)
                .or_else(region);
            let region = region_provider.region().await;

            let conf = ProviderConfig::default().with_region(region);

            let creds_provider = ProfileFileCredentialsProvider::builder()
                .profile_name(profile)
                .configure(&conf)
                .build();

            config_loader = config_loader
                .region(region_provider)
                .credentials_provider(creds_provider);
        }

        config_loader.load().await
    }
}

pub mod aws_sdk_config {
    pub use aws_types::SdkConfig;
}
pub use aws_sdk_lambda;
