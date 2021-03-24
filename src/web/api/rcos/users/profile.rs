//! Profile query.

use crate::error::TelescopeError;
use crate::web::api::rcos::{
    send_query,
    prelude::*
};

#[derive(GraphQLQuery)]
#[graphql(
    schema_path = "graphql/rcos/schema.json",
    query_path = "graphql/rcos/users/profile.graphql",
    response_derives = "Debug,Clone,Serialize"
)]
pub struct Profile;

// import generated types.
use profile::{
    ResponseData,
    //ProfileUsersByPk
    Variables,
};

impl Profile {
    /// Get the profile data for a given username.
    pub async fn for_user(username: String) -> Result<ResponseData, TelescopeError> {
        send_query::<Self>(Variables { username }).await
    }
}
