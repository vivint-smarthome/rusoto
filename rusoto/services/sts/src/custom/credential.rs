use async_trait::async_trait;
use chrono::prelude::*;
use chrono::Duration;

use rusoto_core;
use rusoto_core::RusotoError;
use rusoto_core::credential::{AwsCredentials, CredentialsError, ProvideAwsCredentials};

use crate::{
    AssumeRoleError, AssumeRoleRequest, AssumeRoleWithWebIdentityError,
    AssumeRoleWithWebIdentityRequest, GetSessionTokenError, GetSessionTokenRequest,
    GetSessionTokenResponse, Sts, StsClient,
};

pub const DEFAULT_DURATION_SECONDS: i64 = 3600;
pub const DEFAULT_ROLE_DURATION_SECONDS: i64 = 900;

/// Trait for conversions from STS Credentials to AWS Credentials.
pub trait NewAwsCredsForStsCreds {
    /// Creates an [AwsCredentials](../rusoto_credential/struct.AwsCredentials.html) from a [Credentials](struct.Credentials.html)
    /// Returns a [CredentialsError](../rusoto_credential/struct.CredentialsError.html) in case of an error.
    fn new_for_credentials(
        sts_creds: crate::generated::Credentials,
    ) -> Result<AwsCredentials, CredentialsError>;
}

impl NewAwsCredsForStsCreds for AwsCredentials {
    fn new_for_credentials(
        sts_creds: crate::generated::Credentials,
    ) -> Result<AwsCredentials, CredentialsError> {
        let expires_at = Some(
            sts_creds
                .expiration
                .parse::<DateTime<Utc>>()
                .map_err(CredentialsError::from)?,
        );

        Ok(AwsCredentials::new(
            sts_creds.access_key_id,
            sts_creds.secret_access_key,
            Some(sts_creds.session_token),
            expires_at,
        ))
    }
}

/// [AwsCredentials](../rusoto_credential/struct.AwsCredentials.html) provider that calls
/// `GetSessionToken` using the provided [StsClient](struct.StsClient.html).
/// To use with MFA, pass in the MFA serial number then set the MFA code.
/// You will need to ensure the provider has a valid code each time you
/// acquire a new STS token.
pub struct StsSessionCredentialsProvider {
    sts_client: Box<dyn Sts + Send + Sync>,
    session_duration: Duration,
    mfa_serial: Option<String>,
    mfa_code: Option<String>,
}

impl StsSessionCredentialsProvider {
    /// Creates a new `StsSessionCredentialsProvider` with the given
    /// [StsClient](struct.StsClient.html) and session parameters.
    ///
    /// * `sts_client` - The [StsClient](struct.StsClient.html) to use to acquire session tokens.
    /// * `duration` - The duration of the session tokens. Default 1 hour.
    /// * `mfa_serial` - Optional MFA hardware device serial number or virtual device ARN. Set the MFA code with `set_mfa_code`.
    pub fn new(
        sts_client: StsClient,
        duration: Option<Duration>,
        mfa_serial: Option<String>,
    ) -> StsSessionCredentialsProvider {
        StsSessionCredentialsProvider {
            sts_client: Box::new(sts_client),
            session_duration: duration
                .unwrap_or_else(|| Duration::seconds(DEFAULT_DURATION_SECONDS)),
            mfa_serial,
            mfa_code: None,
        }
    }

    /// Set the MFA code for use when acquiring session tokens.
    pub fn set_mfa_code<S>(&mut self, code: S)
    where
        S: Into<String>,
    {
        self.mfa_code = Some(code.into());
    }

    /// Clear the MFA code.
    pub fn clear_mfa_code(&mut self) {
        self.mfa_code = None;
    }

    /// Calls `GetSessionToken` to get a session token from the STS Api.
    /// Optionally uses MFA if the MFA serial number and code are set.
    pub async fn get_session_token(&self) -> Result<GetSessionTokenResponse, RusotoError<GetSessionTokenError>> {
        let request = GetSessionTokenRequest {
            serial_number: self.mfa_serial.clone(),
            token_code: self.mfa_code.clone(),
            duration_seconds: Some(self.session_duration.num_seconds() as i64),
        };
        self.sts_client.get_session_token(request).await
    }
}

#[async_trait]
impl ProvideAwsCredentials for StsSessionCredentialsProvider {
    async fn credentials(&self) -> Result<AwsCredentials, CredentialsError> {
        let resp = self.get_session_token().await
            .map_err(|err| CredentialsError::new(format!(
                "StsProvider get_session_token error: {:?}",
                err
            )))?;
        let creds = resp
            .credentials
            .ok_or_else(|| CredentialsError::new("no credentials in response"))?;

        AwsCredentials::new_for_credentials(creds)
    }
}

/// [AwsCredentials](../rusoto_credential/struct.AwsCredentials.html) provider that calls
/// `AssumeRole` using the provided [StsClient](struct.StsClient.html).
/// To use with MFA, pass in the MFA serial number then set the MFA code.
/// You will need to ensure the provider has a valid code each time you
/// acquire a new STS token.
pub struct StsAssumeRoleSessionCredentialsProvider {
    sts_client: Box<dyn Sts + Send + Sync>,
    role_arn: String,
    session_name: String,
    external_id: Option<String>,
    session_duration: Duration,
    scope_down_policy: Option<String>,
    mfa_serial: Option<String>,
    mfa_code: Option<String>,
}

impl StsAssumeRoleSessionCredentialsProvider {
    /// Creates a new `StsAssumeRoleSessionCredentialsProvider` with the given
    /// [StsClient](struct.StsClient.html) and session parameters.
    ///
    /// * `sts_client` - [StsClient](struct.StsClient.html) to use to acquire session tokens.
    /// * `role_arn` - The ARN of the role to assume.
    /// * `session_name` - An identifier for the assumed role session. Minimum length of 2. Maximum length of 64. Pattern: `[\w+=,.@-]*`
    /// * `external_id` -
    /// * `session_duration` - Duration of session tokens. Default 1 hour.
    /// * `scope_down_policy` - Optional inline IAM policy in JSON format to further restrict the access granted to the negotiated session.
    /// * `mfa_serial` - Optional MFA hardware device serial number or virtual device ARN. Use `set_mfa_code` to set the MFA code.
    pub fn new(
        sts_client: StsClient,
        role_arn: String,
        session_name: String,
        external_id: Option<String>,
        session_duration: Option<Duration>,
        scope_down_policy: Option<String>,
        mfa_serial: Option<String>,
    ) -> StsAssumeRoleSessionCredentialsProvider {
        StsAssumeRoleSessionCredentialsProvider {
            sts_client: Box::new(sts_client),
            role_arn,
            session_name,
            external_id,
            session_duration: session_duration
                .unwrap_or_else(|| Duration::seconds(DEFAULT_ROLE_DURATION_SECONDS)),
            scope_down_policy,
            mfa_serial,
            mfa_code: None,
        }
    }

    /// Set the MFA code for use when acquiring session tokens.
    pub fn set_mfa_code<S>(&mut self, code: S)
    where
        S: Into<String>,
    {
        self.mfa_code = Some(code.into());
    }

    /// Clear the MFA code.
    pub fn clear_mfa_code(&mut self) {
        self.mfa_code = None;
    }

    /// Calls `AssumeRole` to get a session token from the STS Api.
    /// Optionally uses MFA if the MFA serial number and code are set.
    pub async fn assume_role(&self) -> Result<AwsCredentials, RusotoError<AssumeRoleError>> {
        let request = AssumeRoleRequest {
            role_arn: self.role_arn.clone(),
            role_session_name: self.session_name.clone(),
            duration_seconds: Some(self.session_duration.num_seconds() as i64),
            external_id: self.external_id.clone(),
            policy: self.scope_down_policy.clone(),
            serial_number: self.mfa_serial.clone(),
            token_code: self.mfa_code.clone(),
            ..Default::default()
        };
        let resp = self.sts_client.assume_role(request).await?;

        let creds = resp
            .credentials
            .ok_or(CredentialsError::new("no credentials in response"))?;

        Ok(AwsCredentials::new_for_credentials(
            creds
        )?)
    }
}

#[async_trait]
impl ProvideAwsCredentials for StsAssumeRoleSessionCredentialsProvider {
    async fn credentials(&self) -> Result<AwsCredentials, CredentialsError> {
        self.assume_role().await
            .map_err(|err| CredentialsError::new(format!(
                "StsProvider get_session_token error: {:?}",
                err
            )))
    }
}

/// [AwsCredentials](../rusoto_credential/struct.AwsCredentials.html) provider that calls
/// `AssumeRoleWithWebIdentity` using the provided [StsClient](struct.StsClient.html).
pub struct StsWebIdentityFederationSessionCredentialsProvider {
    sts_client: Box<dyn Sts + Send + Sync>,
    wif_token: String,
    wif_provider: Option<String>,
    role_arn: String,
    session_name: String,
    session_duration: Duration,
    scope_down_policy: Option<String>,
}

impl StsWebIdentityFederationSessionCredentialsProvider {
    /// Creates a new `StsWebIdentityFederationSessionCredentialsProvider` with the given
    /// [StsClient](struct.StsClient.html) and session parameters.
    ///
    /// * `sts_client` - The [StsClient](struct.StsClient.html) to use to acquire session tokens.
    /// * `wif_token` - The OAuth 2.0 access token or OpenID Connect ID token that is provided by the identity provider.
    /// * `wif_provider` - The fully qualified host component of the domain name of the identity provider. Only for OAuth 2.0 access tokens. Do not include URL schemes and port numbers.
    /// * `role_arn` - The ARN of the role to assume.
    /// * `session_name` - An identifier for the assumed role session. Minimum length of 2. Maximum length of 64. Pattern: `[\w+=,.@-]*`
    /// * `session_duration` - Duration of session tokens. Default 1 hour.
    /// * `scope_down_policy` - Optional inline IAM policy in JSON format to further restrict the access granted to the negotiated session.
    pub fn new(
        sts_client: StsClient,
        wif_token: String,
        wif_provider: Option<String>,
        role_arn: String,
        session_name: String,
        session_duration: Option<Duration>,
        scope_down_policy: Option<String>,
    ) -> StsWebIdentityFederationSessionCredentialsProvider {
        StsWebIdentityFederationSessionCredentialsProvider {
            sts_client: Box::new(sts_client),
            wif_token,
            wif_provider,
            role_arn,
            session_name,
            session_duration: session_duration
                .unwrap_or_else(|| Duration::seconds(DEFAULT_DURATION_SECONDS)),
            scope_down_policy,
        }
    }

    /// Calls `AssumeRoleWithWebIdentity` to get a session token from the STS Api.
    pub async fn assume_role_with_web_identity(
        &self,
    ) -> Result<AwsCredentials, RusotoError<AssumeRoleWithWebIdentityError>> {
        let request = AssumeRoleWithWebIdentityRequest {
            web_identity_token: self.wif_token.clone(),
            provider_id: self.wif_provider.clone(),
            role_arn: self.role_arn.clone(),
            role_session_name: self.session_name.clone(),
            duration_seconds: Some(self.session_duration.num_seconds() as i64),
            policy: self.scope_down_policy.clone(),
            ..Default::default()
        };

        let resp = self.sts_client.assume_role_with_web_identity(request).await?;

        let creds = resp
            .credentials
            .ok_or(CredentialsError::new("no credentials in response"))?;

        let mut aws_creds = AwsCredentials::new_for_credentials(creds)?;

        if let Some(subject_from_wif) = resp.subject_from_web_identity_token {
            aws_creds.claims_mut().insert(
                rusoto_core::credential::claims::SUBJECT.to_owned(),
                subject_from_wif,
            );
        }

        if let Some(audience) = resp.audience {
            aws_creds.claims_mut().insert(
                rusoto_core::credential::claims::AUDIENCE.to_owned(),
                audience,
            );
        }

        if let Some(issuer) = resp.provider {
            aws_creds
                .claims_mut()
                .insert(rusoto_core::credential::claims::ISSUER.to_owned(), issuer);
        }

        Ok(aws_creds)
    }
}

#[test]
fn sts_futures_are_send() {
    fn is_send<T: Send>() {}

    is_send::<StsSessionCredentialsProvider>();
    is_send::<StsAssumeRoleSessionCredentialsProvider>();
    is_send::<StsWebIdentityFederationSessionCredentialsProvider>();
}
