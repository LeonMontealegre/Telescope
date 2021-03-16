//! Trait for types stored in the user's identity cookie.

use crate::error::TelescopeError;
use crate::web::api::rcos::users::UserAccountType;
use crate::web::services::auth::oauth2_providers::{
    discord::DiscordIdentity, github::GitHubIdentity,
};
use actix_identity::Identity as ActixIdentity;
use actix_web::dev::{Payload, PayloadStream};
use actix_web::{FromRequest, HttpRequest};
use futures::future::{ready, Ready, LocalBoxFuture};
use serde::Serialize;

/// The root identity that this user is authenticated with.
#[derive(Serialize, Deserialize)]
pub enum RootIdentity {
    /// Github access token
    GitHub(GitHubIdentity),

    /// Discord access and refresh tokens.
    Discord(DiscordIdentity)
}

impl RootIdentity {
    /// Refresh this identity token if necessary.
    pub async fn refresh(self) -> Result<Self, TelescopeError> {
        // If this is a discord-based identity, refresh it and construct the refreshed root identity.
        if let RootIdentity::Discord(discord) = self {
            return discord.refresh().await.map(RootIdentity::Discord);
        }
        // Otherwise no-op.
        return Ok(self);
    }

    /// Get the user account variant representing the authenticated platform.
    pub fn get_user_account_type(&self) -> UserAccountType {
        match self {
            RootIdentity::GitHub(_) => UserAccountType::GitHub,
            RootIdentity::Discord(_) => UserAccountType::Discord,
        }
    }

    /// Get the string representing the unique user identifier on this platform.
    pub async fn get_platform_id(&self) -> Result<String, TelescopeError> {
        match self {
            RootIdentity::GitHub(gh) => gh.get_user_id().await,
            RootIdentity::Discord(d) => d.get_user_id().await,
        }
    }

    /// Get the username of the RCOS account associated with the account
    /// authenticated with this access token (if one exists).
    pub async fn get_rcos_username(&self) -> Result<Option<String>, TelescopeError> {
        match self {
            RootIdentity::GitHub(gh) => gh.get_rcos_username().await,
            RootIdentity::Discord(d) => d.get_rcos_username().await
        }
    }

    /// Put this root in a top level identity cookie.
    pub fn make_authenticated_cookie(self) -> AuthenticatedIdentities {
        AuthenticatedIdentities {
            root: self,
            github: None,
            discord: None,
        }
    }
}

/// The top level object stored in the identity cookie.
#[derive(Serialize, Deserialize)]
pub struct AuthenticatedIdentities {
    /// The root authenticated identity. This identity must always exist.
    pub root: RootIdentity,

    /// An optional GitHub access token.
    pub github: Option<GitHubIdentity>,

    /// An optional Discord access and refresh token.
    pub discord: Option<DiscordIdentity>,
}

impl AuthenticatedIdentities {
    /// If necessary, refresh an identity cookie. This could include getting a
    /// new access token from an OAuth API for example.
    pub async fn refresh(mut self) -> Result<Self, TelescopeError> {
        // Refresh the root identity
        self.root = self.root.refresh().await?;

        // When there is an additional discord identity.
        if let Some(discord_identity) = self.discord {
            // Refresh the discord identity
            let refreshed = discord_identity.refresh().await?;
            // Store back and return self.
            self.discord = Some(refreshed);
            return Ok(self);
        }

        // Otherwise return self
        return Ok(self);
    }

    /// Get the RCOS username of an authenticated user. This is the same as just getting the
    /// RCOS username of the root identity.
    pub async fn get_rcos_username(&self) -> Result<Option<String>, TelescopeError> {
        self.root.get_rcos_username().await
    }

    /// Get discord credentials if authenticated.
    pub fn get_discord(&self) -> Option<&DiscordIdentity> {
        // Check the root identity first
        if let RootIdentity::Discord(discord) = &self.root {
            Some(discord)
        } else {
            // Otherwise return the child field.
            self.discord.as_ref()
        }
    }

    /// Get the github credentials if authenticated.
    pub fn get_github(&self) -> Option<&GitHubIdentity> {
        if let RootIdentity::GitHub(gh) = &self.root {
            Some(gh)
        } else {
            self.github.as_ref()
        }
    }
}

/// The identity of a user accessing telescope.
#[derive(Clone)]
pub struct Identity {
    /// The actix identity of this request. This handles cookie and
    /// security stuff.
    inner: ActixIdentity,
}

impl FromRequest for Identity {
    type Error = TelescopeError;
    type Future = Ready<Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload<PayloadStream>) -> Self::Future {
        // Extract the actix identity and convert any errors
        ready(
            ActixIdentity::extract(req)
                // Unwrap the ready future
                .into_inner()
                // Normalize the error as an ISE
                .map_err(|e| {
                    TelescopeError::ise(format!(
                        "Could not extract identity \
            object from request. Internal error: {}",
                        e
                    ))
                })
                // Wrap the extracted identity.
                .map(|inner| Self { inner }),
        )
    }
}

impl FromRequest for AuthenticatedIdentities {
    type Error = TelescopeError;
    type Future = LocalBoxFuture<'static, Result<Self, Self::Error>>;
    type Config = ();

    fn from_request(req: &HttpRequest, _: &mut Payload<PayloadStream>) -> Self::Future {
        // Clone a reference to the HTTP req, since its behind an Rc pointer.
        let owned_request: HttpRequest = req.clone();
        return Box::pin(async move {
            // Extract the telescope-identity from the request
            Identity::extract(&owned_request)
                // Wait and propagate errors
                .await?
                // Get the cookie if there is one
                .identity()
                // Wait and make error on none
                .await
                .ok_or(TelescopeError::NotAuthenticated)
        });
    }
}

impl Identity {
    /// Forget the user's identity if it exists.
    pub fn forget(&self) {
        self.inner.forget()
    }

    /// Save an identity object to the client's cookies.
    pub fn save(&self, identity: &AuthenticatedIdentities) {
        // Serialize the cookie to JSON first. This serialization should not fail.
        let cookie: String =
            serde_json::to_string(identity).expect("Could not serialize identity cookie");

        // Remember cookie.
        self.inner.remember(cookie)
    }

    /// Get the user's identity. Refresh it if necessary.
    pub async fn identity(&self) -> Option<AuthenticatedIdentities> {
        // Get the inner identity as a String.
        let id: String = self.inner.identity()?;
        // try to deserialize it
        match serde_json::from_str::<AuthenticatedIdentities>(id.as_str()) {
            // On okay, refresh the identity cookie if needed
            Ok(id) => match id.refresh().await {
                // If this succeeds
                Ok(id) => {
                    // Save and return the authenticated identity
                    self.save(&id);
                    return Some(id);
                }

                // If it fails to refresh, we have no identity. Send a warning
                // and return None.
                Err(e) => {
                    warn!("Could not refresh identity token. Error: {}", e);
                    return None;
                }
            },

            // If there is an error deserializing, the identity is malformed.
            // Forget it, and log a warning. Return no identity.
            Err(err) => {
                warn!("Bad identity forgotten. Error: {}", err);
                self.forget();
                return None;
            }
        }
    }

    /// Get the username of the authenticated RCOS account (if there is one.)
    pub async fn get_rcos_username(&self) -> Result<Option<String>, TelescopeError> {
        // If there is an identity cookie
        if let Some(id) = self.identity().await {
            // Use it to get the authenticated RCOS username.
            return id.get_rcos_username().await;
        } else {
            return Ok(None);
        }
    }
}
