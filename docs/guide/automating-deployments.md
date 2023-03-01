# Automated deployments

This page explains how we can use automate the lambda deployment process, using CI. 
All we need is credentials to an AWS user with the correct permissions.

To read more about the `cargo lambda deploy` command see the [commands](/commands/deploy) documentation.

## Step 1: Create an AWS service account

First we need a set of user credentials, to be able to authenticate to AWS when deploying.
Our user needs to be able to create, update, and retrieve lambda functions, and it needs to be 
able to publish new versions.

Here's how you might define the user, using terraform:

```shell
resource "aws_iam_user" "lambda-service-user" {
  name = "lambda-service-user"
}

resource "aws_iam_access_key" "lambda-service-user" {
  user = aws_iam_user.lambda-service-user.name
}

resource "aws_iam_policy" "lambda-service-policy" {
  name   = "lambda-service-policy"
  policy = jsonencode({
    Version   = "2012-10-17"
    Statement = [
      {
        Effect = "Allow"
        Action = [
          "lambda:GetFunction",
          "lambda:CreateFunction",
          "lambda:UpdateFunctionCode",
          "lambda:UpdateFunctionConfiguration",
          "lambda:PublishVersion",
          "lambda:TagResource"
        ]
        Resource = [
          "arn:aws:lambda:<region>:<account-id>:function:<function-name>",
        ]
      }
    ]
  })
}

resource "aws_iam_user_policy_attachment" "lambda-service-user-policy-attachment" {
  user       = aws_iam_user.lambda-service-user.name
  policy_arn = aws_iam_policy.lambda-service-policy.arn
}

output "aws_access_key_id" {
  value = aws_iam_access_key.lambda-service.id
}

output "aws_secret_access_key" {
  value     = aws_iam_access_key.lambda-service.secret
  sensitive = true
}
```

When applied, the secret access key can be read with `terraform output -raw aws_secret_access_key`.

If you prefer to do this without the use of terraform, feel free to use another
tool like it, or just create the user directly in the AWS console.

## Step 2: Add credentials to your repository\'s secret

If you're using Github, go to `github.com/<YOUR-ORG-OR-USERNAME>/<REPO>/settings/secrets/actions`, 
and add the key and secret we just created:

- `AWS_ACCESS_KEY_ID`, and 
- `AWS_SECRET_ACCESS_KEY`

Feel free to name the secrets what you like as long as they're named correctly in the workflow below.

## Step 3: Creating the release workflow

Once we have the necessary credentials set up, we can create our workflow.
Here we'll define a workflow which releases a new version of our lambda when code is pushed to our main branch.

```yaml
name: release

on:
  push:
    branches:
      - main  # or master
  
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3

      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
          profile: minimal
          override: true

      - name: Cache cargo registry
        uses: actions/cache@v3
        continue-on-error: false
        with:
          path: |
            ~/.cargo/registry
            ~/.cargo/git
          key: cargo-build-cache

      - name: Release lambda
        run: |
          pip install cargo-lambda
          cargo lambda build --release
          cargo lambda deploy <FUNCTION-NAME>
        env:
          AWS_DEFAULT_REGION: <YOUR-REGION>
          AWS_ACCESS_KEY_ID: ${{ secrets.AWS_ACCESS_KEY_ID }}
          AWS_SECRET_ACCESS_KEY: ${{ secrets.AWS_SECRET_ACCESS_KEY }}
```

Assuming you've followed all the steps correctly, this should result in a new lambda being created
if it's the first time the lambda is deployed this way; otherwise a new version is pushed.

Note that you don't need to use Github actions for this. This is only meant
as an example that is comprehensive enough to get you started.

If you have suggestion for how this documentation can be improved, please feel free to submit a PR.
