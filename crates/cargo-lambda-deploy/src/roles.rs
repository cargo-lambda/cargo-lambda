use aws_sdk_iam::Client as IamClient;
use aws_sdk_sts::{Client as StsClient, Error};
use aws_smithy_types::error::metadata::ProvideErrorMetadata;
use cargo_lambda_interactive::progress::Progress;
use cargo_lambda_remote::aws_sdk_config::SdkConfig;
use miette::{IntoDiagnostic, Result, WrapErr};
use tokio::time::{sleep, Duration};

const BASIC_LAMBDA_EXECUTION_POLICY: &str =
    "arn:aws:iam::aws:policy/service-role/AWSLambdaBasicExecutionRole";

pub(crate) async fn create(config: &SdkConfig, progress: &Progress) -> Result<String> {
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
                "Action": ["sts:AssumeRole","sts:SetSourceIdentity"],
                "Principal": {
                    "AWS": identity.arn().expect("missing account arn"),
                    "Service": "lambda.amazonaws.com"
                }
            }
        ]
    });

    tracing::trace!(policy = ?policy, "creating role with assume policy");

    let role = client
        .create_role()
        .role_name(&role_name)
        .assume_role_policy_document(policy.to_string())
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

    policy["Statement"][0]["Action"] = serde_json::json!("sts:AssumeRole");
    policy["Statement"][0]["Principal"] = serde_json::json!({"Service": "lambda.amazonaws.com"});
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

    Ok(role_arn.to_string())
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
                        .wrap_err("failed to assume new lambda role")
                }
            },
            Err(err) => {
                return Err(err)
                    .into_diagnostic()
                    .wrap_err("failed to assume new lambda role")
            }
        }
    }

    Err(miette::miette!(
        "failed to assume new lambda role.\nTry deploying using the flag `--iam-role {}`",
        role_arn
    ))
}
