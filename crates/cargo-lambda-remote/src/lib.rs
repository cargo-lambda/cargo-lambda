use aws_config::{
    meta::region::RegionProviderChain,
    profile::{ProfileFileCredentialsProvider, ProfileFileRegionProvider},
    provider_config::ProviderConfig,
    retry::RetryConfig,
    BehaviorVersion,
};
use aws_types::{region::Region, SdkConfig};
use clap::Args;
use serde::{ser::SerializeStruct, Deserialize, Serialize};
pub mod tls;

const DEFAULT_REGION: &str = "us-east-1";

#[derive(Args, Clone, Debug, Default, Deserialize, Serialize)]
pub struct RemoteConfig {
    /// AWS configuration profile to use for authorization
    #[arg(short, long)]
    #[serde(default)]
    pub profile: Option<String>,

    /// AWS region to deploy, if there is no default
    #[arg(short, long)]
    #[serde(default)]
    pub region: Option<String>,

    /// AWS Lambda alias to associate the function to
    #[arg(short, long)]
    #[serde(default)]
    pub alias: Option<String>,

    /// Number of attempts to try failed operations
    #[arg(long, default_value = "1")]
    #[serde(default)]
    pub retry_attempts: Option<u32>,

    /// Custom endpoint URL to target
    #[arg(long)]
    #[serde(default)]
    pub endpoint_url: Option<String>,
}

impl RemoteConfig {
    fn retry_policy(&self) -> RetryConfig {
        let attempts = self.retry_attempts.unwrap_or(1);
        RetryConfig::standard().with_max_attempts(attempts)
    }

    pub async fn sdk_config(&self, retry: Option<RetryConfig>) -> SdkConfig {
        let explicit_region = self.region.clone().map(Region::new);

        let region_provider = RegionProviderChain::first_try(explicit_region.clone())
            .or_default_provider()
            .or_else(Region::new(DEFAULT_REGION));

        let retry = retry.unwrap_or_else(|| self.retry_policy());
        let mut config_loader = if let Some(ref endpoint_url) = self.endpoint_url {
            aws_config::defaults(BehaviorVersion::latest())
                .endpoint_url(endpoint_url)
                .region(region_provider)
                .retry_config(retry)
        } else {
            aws_config::defaults(BehaviorVersion::latest())
                .region(region_provider)
                .retry_config(retry)
        };

        if let Some(profile) = &self.profile {
            let profile_region = ProfileFileRegionProvider::builder()
                .profile_name(profile)
                .build();

            let region_provider =
                RegionProviderChain::first_try(explicit_region).or_else(profile_region);
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

    pub fn count_fields(&self) -> usize {
        self.profile.is_some() as usize
            + self.region.is_some() as usize
            + self.alias.is_some() as usize
            + self.retry_attempts.is_some() as usize
            + self.endpoint_url.is_some() as usize
    }

    pub fn serialize_fields<S>(
        &self,
        state: &mut <S as serde::Serializer>::SerializeStruct,
    ) -> Result<(), S::Error>
    where
        S: serde::Serializer,
    {
        if let Some(ref profile) = self.profile {
            state.serialize_field("profile", profile)?;
        }
        if let Some(ref region) = self.region {
            state.serialize_field("region", region)?;
        }
        if let Some(ref alias) = self.alias {
            state.serialize_field("alias", alias)?;
        }
        if let Some(ref retry_attempts) = self.retry_attempts {
            state.serialize_field("retry_attempts", retry_attempts)?;
        }
        if let Some(ref endpoint_url) = self.endpoint_url {
            state.serialize_field("endpoint_url", endpoint_url)?;
        }

        Ok(())
    }
}

pub mod aws_sdk_config {
    pub use aws_types::SdkConfig;
}
pub use aws_sdk_lambda;

#[cfg(test)]
mod tests {
    use aws_sdk_lambda::config::{ProvideCredentials, Region};

    use crate::RemoteConfig;

    fn setup() {
        let manifest_dir = env!("CARGO_MANIFEST_DIR");
        std::env::set_var(
            "AWS_CONFIG_FILE",
            format!("{manifest_dir}/test-data/aws_config"),
        );
        std::env::set_var(
            "AWS_SHARED_CREDENTIALS_FILE",
            format!("{manifest_dir}/test-data/aws_credentials"),
        );
    }

    /// Specify a profile which does not exist
    /// Expectations:
    /// - Region is undefined
    /// - Credentials are undefined
    #[tokio::test]
    async fn undefined_profile() {
        setup();

        let args = RemoteConfig {
            profile: Some("durian".to_owned()),
            region: None,
            alias: None,
            retry_attempts: Some(1),
            endpoint_url: None,
        };

        let config = args.sdk_config(None).await;
        let creds = config
            .credentials_provider()
            .unwrap()
            .provide_credentials()
            .await;

        assert_eq!(config.region(), None);
        assert!(creds.is_err());
    }

    /// Specify a profile which exists in the credentials file but not in the config file
    /// Expectations:
    /// - Region is undefined
    /// - Credentials are used from the profile
    #[tokio::test]
    async fn undefined_profile_with_creds() {
        setup();

        let args = RemoteConfig {
            profile: Some("cherry".to_owned()),
            region: None,
            alias: None,
            retry_attempts: Some(1),
            endpoint_url: None,
        };

        let config = args.sdk_config(None).await;
        let creds = config
            .credentials_provider()
            .unwrap()
            .provide_credentials()
            .await
            .unwrap();

        assert_eq!(config.region(), None);
        assert_eq!(creds.access_key_id(), "CCCCCCCCCCCCCCCCCCCC");
    }

    /// Specify a profile which has a region associated to it
    /// Expectations:
    /// - Region is used from the profile
    /// - Credentials are used from the profile
    #[tokio::test]
    async fn profile_with_region() {
        setup();

        let args = RemoteConfig {
            profile: Some("apple".to_owned()),
            region: None,
            alias: None,
            retry_attempts: Some(1),
            endpoint_url: None,
        };

        let config = args.sdk_config(None).await;
        let creds = config
            .credentials_provider()
            .unwrap()
            .provide_credentials()
            .await
            .unwrap();

        assert_eq!(config.region(), Some(&Region::from_static("ca-central-1")));
        assert_eq!(creds.access_key_id(), "AAAAAAAAAAAAAAAAAAAA");
    }

    /// Specify a profile which does not have a region associated to it
    /// Expectations:
    /// - Region is undefined
    /// - Credentials are used from the profile
    #[tokio::test]
    async fn profile_without_region() {
        setup();

        let args = RemoteConfig {
            profile: Some("banana".to_owned()),
            region: None,
            alias: None,
            retry_attempts: Some(1),
            endpoint_url: None,
        };

        let config = args.sdk_config(None).await;
        let creds = config
            .credentials_provider()
            .unwrap()
            .provide_credentials()
            .await
            .unwrap();

        assert_eq!(config.region(), None);
        assert_eq!(creds.access_key_id(), "BBBBBBBBBBBBBBBBBBBB");
    }

    /// Use the default profile which has a region associated to it
    /// Expectations:
    /// - Region is used from the profile
    /// - Credentials are used from the profile
    #[tokio::test]
    async fn default_profile() {
        setup();

        let args = RemoteConfig {
            profile: None,
            region: None,
            alias: None,
            retry_attempts: Some(1),
            endpoint_url: None,
        };

        let config = args.sdk_config(None).await;
        let creds = config
            .credentials_provider()
            .unwrap()
            .provide_credentials()
            .await
            .unwrap();

        assert_eq!(config.region(), Some(&Region::from_static("af-south-1")));
        assert_eq!(creds.access_key_id(), "DDDDDDDDDDDDDDDDDDDD");
    }
}
