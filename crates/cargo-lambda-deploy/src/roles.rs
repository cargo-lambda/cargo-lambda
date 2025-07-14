use aws_sdk_iam::Client as IamClient;
use aws_sdk_sts::{Client as StsClient, Error};
use aws_smithy_types::error::metadata::ProvideErrorMetadata;
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_metadata::cargo::deploy::Deploy;
use cargo_lambda_remote::aws_sdk_config::SdkConfig;
use miette::{IntoDiagnostic, Result, WrapErr};
use tokio::time::{Duration, sleep};

const BASIC_LAMBDA_EXECUTION_POLICY: &str =
    "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole";

#[derive(Debug)]
pub(crate) struct FunctionRole(String, bool);

impl FunctionRole {
    /// Create a new function role.
    pub(crate) fn new(arn: String) -> FunctionRole {
        FunctionRole(arn, true)
    }

    /// Create a function role from an existing role.
    pub(crate) fn from_existing(arn: String) -> FunctionRole {
        FunctionRole(arn, false)
    }

    pub(crate) fn arn(&self) -> &str {
        &self.0
    }

    pub(crate) fn is_new(&self) -> bool {
        self.1
    }
}

pub(crate) async fn create(
    deploy: &Deploy,
    config: &SdkConfig,
    progress: &Progress,
) -> Result<FunctionRole> {
    progress.set_message("creating execution role");

    let role_name = format!("cargo-lambda-role-{}", uuid::Uuid::new_v4());
    let client = IamClient::new(config);
    let sts_client = StsClient::new(config);
    let identity = sts_client
        .get_caller_identity()
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to get caller identity")?;

    let mut policy = serde_json::json!({
        "Version": "2012-10-17",
        "Statement": [
            {
                "Effect": "Allow",
                "Action": ["sts:AssumeRole"],
                "Principal": {
                    "Service": "lambda.amazonaws.com"
                }
            },
            {
                "Effect": "Allow",
                "Action": ["sts:AssumeRole", "sts:SetSourceIdentity", "sts:TagSession"],
                "Principal": {
                    "AWS": identity.arn().expect("missing account arn"),
                }
            }
        ]
    });

    tracing::trace!(policy = ?policy, "creating role with assume policy");

    let role = client
        .create_role()
        .role_name(&role_name)
        .assume_role_policy_document(policy.to_string())
        .set_tags(deploy.iam_tags())
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to create function role")?
        .role
        .expect("missing role information");

    client
        .attach_role_policy()
        .role_name(&role_name)
        .policy_arn(BASIC_LAMBDA_EXECUTION_POLICY)
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to attach policy AWSLambdaBasicExecutionRole to function role")?;

    let role_arn = role.arn();

    progress.set_message("verifying role access, this can take up to 20 seconds");

    try_assume_role(&sts_client, role_arn).await?;

    // remove the current identity from the trust policy
    policy["Statement"]
        .as_array_mut()
        .expect("missing statement array")
        .pop();

    tracing::trace!(policy = ?policy, "updating assume policy");

    client
        .update_assume_role_policy()
        .role_name(&role_name)
        .policy_document(policy.to_string())
        .send()
        .await
        .into_diagnostic()
        .wrap_err("failed to restrict service policy")?;

    tracing::debug!(role = ?role, "function role created");

    Ok(FunctionRole::new(role_arn.to_string()))
}

async fn try_assume_role(client: &StsClient, role_arn: &str) -> Result<()> {
    sleep(Duration::from_secs(5)).await;

    for attempt in 1..3 {
        let session_id = format!("cargo_lambda_session_{}", uuid::Uuid::new_v4());

        let result = client
            .assume_role()
            .role_arn(role_arn)
            .role_session_name(session_id)
            .send()
            .await
            .map_err(Error::from);

        tracing::trace!(attempt = attempt, result = ?result, "attempted to assume new role");

        match result {
            Ok(_) => return Ok(()),
            Err(err) if attempt < 3 => match err.code() {
                Some("AccessDenied") => {
                    tracing::trace!(
                        ?err,
                        "role might not be fully propagated yet, waiting before retrying"
                    );
                    sleep(Duration::from_secs(attempt * 5)).await
                }
                _ => {
                    return Err(err)
                        .into_diagnostic()
                        .wrap_err("failed to assume new lambda role");
                }
            },
            Err(err) => {
                return Err(err)
                    .into_diagnostic()
                    .wrap_err("failed to assume new lambda role");
            }
        }
    }

    Err(miette::miette!(
        "failed to assume new lambda role.\nTry deploying using the flag `--iam-role {}`",
        role_arn
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use aws_credential_types::Credentials;
    use aws_sdk_s3::config::{Region, SharedCredentialsProvider};
    use aws_smithy_runtime::client::http::test_util::{ReplayEvent, StaticReplayClient};
    use aws_smithy_types::body::SdkBody;
    use cargo_lambda_interactive::progress::Progress;
    use http::{Request, Response};

    #[tokio::test]
    async fn test_create_function_role() {
        let get_caller_identity_request = Request::builder()
            .uri("https://sts.us-east-1.amazonaws.com/")
            .body(SdkBody::from(
                serde_urlencoded::to_string([
                    ("Action", "GetCallerIdentity"),
                    ("Version", "2011-06-15"),
                ])
                .unwrap(),
            ))
            .unwrap();
        let get_caller_identity_response = Response::builder()
            .status(200)
            .body(SdkBody::from(
                r#"<GetCallerIdentityResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
                    <GetCallerIdentityResult>
                        <Arn>arn:aws:iam::123456789012:user/ExampleUser</Arn>
                    </GetCallerIdentityResult>
                </GetCallerIdentityResponse>"#,
            ))
            .unwrap();

        let doc = serde_json::json!({
            "Version": "2012-10-17",
            "Statement": [
                {
                    "Effect": "Allow",
                    "Action": ["sts:AssumeRole"],
                    "Principal": {
                        "Service": "lambda.amazonaws.com"
                    }
                },
                {
                    "Effect": "Allow",
                    "Action": ["sts:AssumeRole", "sts:SetSourceIdentity", "sts:TagSession"],
                    "Principal": {
                        "AWS": "arn:aws:iam::123456789012:user/ExampleUser"
                    }
                }
            ]
        })
        .to_string();

        let create_role_request = Request::builder()
            .uri("https://iam.amazonaws.com/")
            .body(SdkBody::from(
                serde_urlencoded::to_string([
                    ("Action", "CreateRole"),
                    ("Version", "2010-05-08"),
                    ("RoleName", "cargo-lambda-role-12345678"),
                    ("AssumeRolePolicyDocument", &doc),
                    ("Tags.member.1.Key", "env"),
                    ("Tags.member.1.Value", "test"),
                ])
                .unwrap(),
            ))
            .unwrap();
        let create_role_response = Response::builder()
            .status(200)
            .body(SdkBody::from(
                r#"<CreateRoleResponse xmlns="https://iam.amazonaws.com/doc/2010-05-08/">
                    <CreateRoleResult>
                        <Role>
                            <Path>/</Path>
                            <RoleName>cargo-lambda-role-12345678</RoleName>
                            <RoleId>AROAIEXAMPLEROLEID</RoleId>
                            <Arn>arn:aws:iam::123456789012:role/cargo-lambda-role-12345678</Arn>
                            <CreateDate>2023-10-01T12:00:00Z</CreateDate>
                        </Role>
                    </CreateRoleResult>
                </CreateRoleResponse>"#,
            ))
            .unwrap();

        let attach_role_policy_request = Request::builder()
            .uri("https://iam.amazonaws.com/")
            .body(SdkBody::from(
                serde_urlencoded::to_string([
                    ("Action", "AttachRolePolicy"),
                    ("Version", "2010-05-08"),
                    ("RoleName", "cargo-lambda-role-12345678"),
                    (
                        "PolicyArn",
                        "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole",
                    ),
                ])
                .unwrap(),
            ))
            .unwrap();
        let attach_role_policy_response = Response::builder()
            .status(200)
            .body(SdkBody::empty())
            .unwrap();

        let assume_role_request = Request::builder()
            .uri("https://sts.us-east-1.amazonaws.com/")
            .body(SdkBody::from(
                serde_urlencoded::to_string([
                    ("Action", "AssumeRole"),
                    ("Version", "2011-06-15"),
                    (
                        "RoleArn",
                        "arn:aws:iam::123456789012:role/cargo-lambda-role-12345678",
                    ),
                    (
                        "RoleSessionName",
                        "cargo_lambda_session_fc79d50b-56a7-4634-86e0-8b2503f6c049",
                    ),
                ])
                .unwrap(),
            ))
            .unwrap();
        let assume_role_response = Response::builder()
            .status(200)
            .body(SdkBody::from(
                r#"<AssumeRoleResponse xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
                    <AssumeRoleResult>
                        <Credentials>
                            <AccessKeyId>EXAMPLEACCESSKEYID</AccessKeyId>
                            <SecretAccessKey>EXAMPLESECRETACCESSKEY</SecretAccessKey>
                            <SessionToken>EXAMPLESESSIONTOKEN</SessionToken>
                            <Expiration>2023-10-01T12:00:00Z</Expiration>
                        </Credentials>
                    </AssumeRoleResult>
                </AssumeRoleResponse>"#,
            ))
            .unwrap();

        let doc = serde_json::json!({
          "Version": "2012-10-17",
          "Statement": [
            {
              "Effect": "Allow",
              "Action": ["sts:AssumeRole"],
              "Principal": {
                "Service": "lambda.amazonaws.com"
              }
            }
          ]
        })
        .to_string();

        let update_assume_role_policy_request = Request::builder()
            .uri("https://iam.amazonaws.com/")
            .body(SdkBody::from(
                serde_urlencoded::to_string([
                    ("Action", "UpdateAssumeRolePolicy"),
                    ("Version", "2010-05-08"),
                    ("RoleName", "cargo-lambda-role-12345678"),
                    ("PolicyDocument", &doc),
                ])
                .unwrap(),
            ))
            .unwrap();
        let update_assume_role_policy_response = Response::builder()
            .status(200)
            .body(SdkBody::empty())
            .unwrap();

        let http_client = StaticReplayClient::new(vec![
            ReplayEvent::new(get_caller_identity_request, get_caller_identity_response),
            ReplayEvent::new(create_role_request, create_role_response),
            ReplayEvent::new(attach_role_policy_request, attach_role_policy_response),
            ReplayEvent::new(assume_role_request, assume_role_response),
            ReplayEvent::new(
                update_assume_role_policy_request,
                update_assume_role_policy_response,
            ),
        ]);

        // Setup SDK config with mock client
        let sdk_config = SdkConfig::builder()
            .credentials_provider(SharedCredentialsProvider::new(Credentials::for_tests()))
            .region(Region::new("us-east-1"))
            .http_client(http_client.clone())
            .build();

        let progress = Progress::start("deploying function role");

        let mut deploy = Deploy::default();
        deploy.tag = Some(vec!["env=test".to_string()]);

        let role = create(&deploy, &sdk_config, &progress).await.unwrap();

        assert!(role.is_new());
        assert!(!role.arn().is_empty());
        assert!(
            role.arn()
                .eq("arn:aws:iam::123456789012:role/cargo-lambda-role-12345678")
        );
        // TODO: find a way to seed the uuid::Uuid::new_v4() so we can assert requests
        // http_client.assert_requests_match(&[]);
    }
}
