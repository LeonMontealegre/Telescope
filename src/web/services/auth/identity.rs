//! Trait for types stored in the user's identity cookie.

use crate::error::TelescopeError;
use crate::web::api::rcos::users::UserAccountType;
use crate::web::services::auth::oauth2_providers::{
    discord::DiscordIdentity, github::GitHubIdentity,
};
use actix_identity::Identity as ActixIdentity;
use actix_web::dev::{Payload, PayloadStream};
use actix_web::{FromRequest, HttpRequest};
use futures::future::{ready, Ready};
use serde::Serialize;

/// The top level enum stored in the identity cookie.
#[derive(Serialize, Deserialize)]
pub enum IdentityCookie {
    /// A GitHub access token.
    Github(GitHubIdentity),

    /// A Discord access and refresh token.
    Discord(DiscordIdentity),
}

impl IdentityCookie {
    /// If necessary, refresh an identity cookie. This could include getting a
    /// new access token from an OAuth API for example.
    pub fn refresh(self) -> Result<Self, TelescopeError> {
        // Destructure on discord identity.
        if let IdentityCookie::Discord(discord_identity) = self {
            return discord_identity
                .refresh()
                // wrap discord identity
                .map(IdentityCookie::Discord);
        }

        // Otherwise return self -- Github does not need to be refreshed.
        return Ok(self);
    }

    /// Get the central RCOS API value representing this identity provider.
    pub fn user_account_type(&self) -> UserAccountType {
        match self {
            IdentityCookie::Discord(_) => UserAccountType::Discord,
            IdentityCookie::Github(_) => UserAccountType::GitHub,
        }
    }

    /// Get the platform's identity of the user who logged in to produce
    /// this access token.
    pub async fn get_account_identity(&self) -> Result<String, TelescopeError> {
        match self {
            IdentityCookie::Github(i) => i.get_user_id().await,
            IdentityCookie::Discord(i) => i.get_user_id().await,
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

impl Identity {
    /// Forget the user's identity if it exists.
    pub fn forget(&self) {
        self.inner.forget()
    }

    /// Save an identity object to the client's cookies.
    pub fn save(&self, identity: &IdentityCookie) {
        // Serialize the cookie to JSON first. This serialization should not fail.
        let cookie: String =
            serde_json::to_string(identity).expect("Could not serialize identity cookie");

        // Remember cookie.
        self.inner.remember(cookie)
    }

    /// Get the user's identity. Refresh it if necessary.
    pub fn identity(&self) -> Option<IdentityCookie> {
        // Get the inner identity as a String.
        let id: String = self.inner.identity()?;
        // try to deserialize it
        match serde_json::from_str::<IdentityCookie>(id.as_str()) {
            // On okay, refresh the identity cookie if needed
            Ok(id) => match id.refresh() {
                // If this succeeds, save and return the new identity.
                Ok(id) => {
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
}
