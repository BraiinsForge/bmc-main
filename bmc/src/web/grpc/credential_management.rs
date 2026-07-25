// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

use bmc_grpc::web;
use tonic::{Request, Response, Status};

use super::scene_management::param_definition_to_proto;
use crate::credential;

pub(crate) struct CredentialManagementService;

#[async_trait::async_trait]
impl web::credential_management_service_server::CredentialManagementService
    for CredentialManagementService
{
    async fn get_credential_types(
        &self,
        _request: Request<()>,
    ) -> Result<Response<web::GetCredentialTypesResponse>, Status> {
        let credential_types = credential::builtins()
            .iter()
            .map(credential_type_to_proto)
            .collect();
        Ok(Response::new(web::GetCredentialTypesResponse {
            credential_types,
        }))
    }
}

fn credential_type_to_proto(t: &credential::CredentialType) -> web::CredentialType {
    web::CredentialType {
        id: t.id.clone(),
        name: t.name.clone(),
        description: t.description.clone(),
        fields: t
            .fields
            .iter()
            .map(|(key, def)| param_definition_to_proto(key.as_str(), def))
            .collect(),
        egress: t.egress.as_ref().map(|e| web::EgressPolicy {
            allow_hosts: e.allow_hosts.clone(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(id: &str) -> credential::CredentialType {
        credential::builtins()
            .into_iter()
            .find(|t| t.id == id)
            .expect("BUG: builtin credential type must exist")
    }

    #[test]
    fn maps_egress_pin_and_field_order() {
        let pool = credential_type_to_proto(&find("braiins-pool"));
        assert_eq!(pool.id, "braiins-pool");
        assert_eq!(
            pool.egress
                .expect("BUG: braiins-pool egress must map")
                .allow_hosts,
            vec!["api.braiins.com".to_owned()]
        );

        let userpass = credential_type_to_proto(&find("generic-userpass"));
        assert!(userpass.egress.is_none(), "generics carry no egress pin");
        let keys: Vec<_> = userpass.fields.iter().map(|f| f.key.clone()).collect();
        assert_eq!(keys, vec!["username".to_owned(), "password".to_owned()]);
    }
}
