//! An organization's people and teams, as its owners and its teams'
//! maintainers manage them on the website, and the invitations sent to
//! you. Reading is [`GitHub::members`], [`GitHub::org_access`] and
//! [`GitHub::invitations`]; every change is an [`OrgChange`], sent by
//! [`GitHub::change_org`].

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api::browse::Results;
use crate::api::client::{ApiError, GitHub, encode_path};
use crate::api::model::RepoId;
use crate::api::raw::raw;
use crate::api::settings::{Listed, Permission};
use crate::api::{has_scope, scope_missing};

raw! {
    /// An organization's members with their roles (seen by its members),
    /// 30 at a time from `after`.
    MEMBERS = "query($org: String!, $after: String) { organization(login: $org) { membersWithRole(first: 30, after: $after) { totalCount pageInfo { hasNextPage endCursor } edges { role node { login name } } } } }";

    /// What GitHub lets you do in an organization.
    ORG_ACCESS = "query($org: String!) { organization(login: $org) { viewerCanAdminister viewerCanCreateTeams viewerIsAMember } }";
}

words! {
    /// A role in an organization, as its owners give it.
    #[derive(Hash, Serialize, Deserialize)]
    pub enum OrgRole {
        pub fn name,
        /// What it lets someone do, in GitHub's words.
        pub fn about,
        /// The word REST takes for it.
        fn wire,
        /// GraphQL's word for it.
        fn graphql -> fn from_graphql,
    }
    Member "Member" "can see every member, and be given access to repositories and teams" "member" "MEMBER";
    Owner  "Owner"  "has full administrative access to the organization"                 "admin"  "ADMIN";
}

/// Someone in an organization: their role, where GitHub shows it (to its
/// members).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub login: String,
    pub name: Option<String>,
    pub role: Option<OrgRole>,
}

/// What you may do in an organization, and, for its owners, the
/// invitations waiting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgAccess {
    /// You own it.
    pub admin: bool,
    pub create_teams: bool,
    pub member: bool,
    /// The scope your token lacks to change its people, said with the
    /// command that adds it, when its answer says so.
    pub lacks: Option<String>,
    /// Its pending invitations, for an owner, or why GitHub doesn't list
    /// them.
    pub invitations: Option<Result<Listed<OrgInvitation>, String>>,
}

/// An invitation to join an organization, as its owners see it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgInvitation {
    pub id: u64,
    /// Whom it's to: a login, or the email it was sent to.
    pub login: Option<String>,
    pub email: Option<String>,
    /// GitHub's role, as the website names it ("Member", "Owner").
    pub role: String,
    /// ISO 8601.
    pub created_at: String,
    pub inviter: Option<String>,
    /// GitHub couldn't deliver it.
    pub failed: bool,
}

impl OrgInvitation {
    /// Whom it's to, as the website names them.
    pub fn who(&self) -> String {
        match (&self.login, &self.email) {
            (Some(login), _) => format!("@{login}"),
            (None, Some(email)) => email.clone(),
            (None, None) => format!("invitation {}", self.id),
        }
    }
}

/// The invitations sent to you: to collaborate on repositories, and to
/// join organizations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Invitations {
    pub repos: Vec<RepoInvitation>,
    /// The organizations whose invitations wait, or why GitHub didn't say.
    pub orgs: Result<Vec<OrgInvited>, String>,
}

/// An invitation to collaborate on a repository, sent to you.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoInvitation {
    pub id: u64,
    pub repo: RepoId,
    pub inviter: Option<String>,
    pub permission: Option<Permission>,
    /// ISO 8601.
    pub created_at: String,
    pub expired: bool,
}

/// An organization that invited you to join.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgInvited {
    pub org: String,
    pub role: OrgRole,
}

/// A change to an organization's people or teams, or to an invitation
/// sent to you.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrgChange {
    /// Invite someone with a role, or change a member's: GitHub's one
    /// request for both.
    SetMember {
        org: String,
        login: String,
        role: OrgRole,
        invite: bool,
    },
    /// Invite someone by email, with a role.
    InviteEmail {
        org: String,
        email: String,
        role: OrgRole,
    },
    RemoveMember {
        org: String,
        login: String,
    },
    CancelInvitation {
        org: String,
        id: u64,
        /// Whom it was to, as said.
        who: String,
    },
    CreateTeam {
        org: String,
        name: String,
        secret: bool,
    },
    /// Add someone to a team, or change their role in it.
    SetTeamMember {
        org: String,
        team: String,
        login: String,
        maintainer: bool,
        add: bool,
    },
    RemoveTeamMember {
        org: String,
        team: String,
        login: String,
    },
    /// Give a team access to a repository, or change it.
    SetTeamRepo {
        org: String,
        team: String,
        repo: RepoId,
        permission: Permission,
        add: bool,
    },
    RemoveTeamRepo {
        org: String,
        team: String,
        repo: RepoId,
    },
    /// Accept a repository's invitation to you, or decline it.
    AnswerRepo {
        id: u64,
        repo: RepoId,
        accept: bool,
    },
    /// Join an organization that invited you.
    JoinOrg {
        org: String,
    },
}

/// How many a REST list asks for.
const PER_PAGE: usize = 100;

/// The website's name for a role an organization's invitation gives.
fn invitation_role(word: &str) -> String {
    match word {
        "direct_member" | "member" => "Member".into(),
        "admin" => "Owner".into(),
        "billing_manager" => "Billing manager".into(),
        "hiring_manager" => "Hiring manager".into(),
        "reinstate" => "Member (reinstated)".into(),
        other => other.replace('_', " "),
    }
}

/// Whether a classic token's scopes can change an organization's people
/// and teams.
fn can_write_org(scopes: &str) -> bool {
    has_scope(scopes, "admin:org") || has_scope(scopes, "write:org")
}

mod wire {
    use serde::Deserialize;

    pub use crate::api::browse::wire::PageInfo;

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Members {
        pub members_with_role: Edges,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Edges {
        pub total_count: u64,
        pub page_info: PageInfo,
        pub edges: Vec<Option<Edge>>,
    }

    #[derive(Deserialize)]
    pub struct Edge {
        pub role: Option<String>,
        pub node: Option<Person>,
    }

    #[derive(Deserialize)]
    pub struct Person {
        pub login: String,
        pub name: Option<String>,
    }

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    pub struct Access {
        pub viewer_can_administer: bool,
        pub viewer_can_create_teams: bool,
        pub viewer_is_a_member: bool,
    }

    #[derive(Deserialize)]
    pub struct Login {
        pub login: String,
    }

    #[derive(Deserialize)]
    pub struct Invitation {
        pub id: u64,
        pub login: Option<String>,
        pub email: Option<String>,
        #[serde(default)]
        pub role: String,
        #[serde(default)]
        pub created_at: String,
        pub failed_at: Option<String>,
        pub inviter: Option<Login>,
    }

    #[derive(Deserialize)]
    pub struct FullName {
        pub full_name: String,
    }

    #[derive(Deserialize)]
    pub struct RepoInvitation {
        pub id: u64,
        pub repository: FullName,
        pub inviter: Option<Login>,
        #[serde(default)]
        pub permissions: String,
        #[serde(default)]
        pub created_at: String,
        #[serde(default)]
        pub expired: bool,
    }

    #[derive(Deserialize)]
    pub struct Membership {
        #[serde(default)]
        pub role: String,
        pub organization: Login,
    }
}

impl GitHub {
    /// An organization's members, with their roles where GitHub shows
    /// them, 30 at a time from `after`; the first page is cached.
    pub async fn members(
        &self,
        org: &str,
        after: Option<String>,
    ) -> Result<Results<Member>, ApiError> {
        let vars = json!({ "org": org, "after": after });
        let wire: wire::Members = self
            .graphql_json(MEMBERS, vars, org, "/organization")
            .await?;
        let list = wire.members_with_role;
        let items = (list.edges.into_iter().flatten())
            .filter_map(|e| {
                let node = e.node?;
                Some(Member {
                    login: node.login,
                    name: node.name,
                    role: e.role.as_deref().and_then(OrgRole::from_graphql),
                })
            })
            .collect();
        Ok(Results {
            total: list.total_count,
            items,
            next: list.page_info.next(),
        })
    }

    /// What you may do in an organization; for its owners, the
    /// invitations waiting, and whether the token may change its people.
    pub async fn org_access(&self, org: &str) -> Result<OrgAccess, ApiError> {
        let vars = json!({ "org": org });
        let access: wire::Access = self
            .graphql_json(ORG_ACCESS, vars, org, "/organization")
            .await?;
        let mut found = OrgAccess {
            admin: access.viewer_can_administer,
            create_teams: access.viewer_can_create_teams,
            member: access.viewer_is_a_member,
            lacks: None,
            invitations: None,
        };
        if !found.admin {
            // A member may make teams, or maintain one: whether the token
            // may, its scopes say.
            if found.member {
                let answer = self.rest_answer(&format!("/orgs/{org}")).await?;
                found.lacks = (answer.header("x-oauth-scopes"))
                    .filter(|s| !can_write_org(s))
                    .map(|_| scope_missing(&format!("Changing @{org}'s teams"), "admin:org"));
            }
            return Ok(found);
        }
        let path = format!("/orgs/{org}/invitations?per_page={PER_PAGE}");
        let what = format!("@{org}'s invitations");
        let mut scopes = None;
        let answer = self
            .rest_refusable(&path, &what, |status, _, answer| {
                let lacks = (answer.header("x-oauth-scopes"))
                    .is_some_and(|s| !can_write_org(s) && !has_scope(s, "read:org"));
                matches!(status, 403 | 404).then(|| {
                    if lacks {
                        scope_missing(&what, "admin:org")
                    } else {
                        format!("GitHub shows {what} only to its owners")
                    }
                })
            })
            .await?;
        let listed = match answer {
            Ok(answer) => {
                scopes = answer.header("x-oauth-scopes").map(str::to_owned);
                let wire: Vec<wire::Invitation> = serde_json::from_str(answer.body())?;
                Ok(Listed {
                    more: answer.next_page().is_some(),
                    items: wire.into_iter().map(invitation).collect(),
                })
            }
            Err(why) => Err(why),
        };
        found.lacks = scopes
            .filter(|s| !can_write_org(s))
            .map(|_| scope_missing(&format!("Changing @{org}'s people and teams"), "admin:org"));
        found.invitations = Some(listed);
        Ok(found)
    }

    /// The invitations sent to you: repositories' (the first 100), and
    /// organizations'.
    pub async fn invitations(&self) -> Result<Invitations, ApiError> {
        let repos = format!("/user/repository_invitations?per_page={PER_PAGE}");
        let orgs = format!("/user/memberships/orgs?state=pending&per_page={PER_PAGE}");
        let (repos, orgs) = tokio::join!(
            self.rest_json::<Vec<wire::RepoInvitation>>(&repos, "your invitations"),
            self.rest_refusable(&orgs, "your organizations' invitations", |status, _, a| {
                let lacks = (a.header("x-oauth-scopes"))
                    .is_some_and(|s| !has_scope(s, "read:org") && !can_write_org(s));
                (lacks && matches!(status, 403 | 404))
                    .then(|| scope_missing("Organizations' invitations", "read:org"))
            }),
        );
        let repos = (repos?.into_iter())
            .filter_map(|i| {
                Some(RepoInvitation {
                    id: i.id,
                    repo: RepoId::parse(&i.repository.full_name)?,
                    inviter: i.inviter.map(|l| l.login),
                    permission: Permission::parse(&i.permissions),
                    created_at: i.created_at,
                    expired: i.expired,
                })
            })
            .collect();
        let orgs = match orgs? {
            Ok(answer) => {
                let wire: Vec<wire::Membership> = serde_json::from_str(answer.body())?;
                Ok((wire.into_iter())
                    .map(|m| OrgInvited {
                        org: m.organization.login,
                        role: if m.role == "admin" {
                            OrgRole::Owner
                        } else {
                            OrgRole::Member
                        },
                    })
                    .collect())
            }
            Err(why) => Err(why),
        };
        Ok(Invitations { repos, orgs })
    }

    /// Makes a change to an organization's people or teams, or answers an
    /// invitation sent to you.
    pub(crate) async fn change_org(&self, change: &OrgChange) -> Result<(), ApiError> {
        use OrgChange as C;
        use http::Method;
        let team = |org: &str, team: &str| format!("/orgs/{org}/teams/{}", encode_path(team));
        match change {
            C::SetMember {
                org, login, role, ..
            } => {
                let body = json!({ "role": role.wire() });
                let path = format!("/orgs/{org}/memberships/{login}");
                self.rest_write(Method::PUT, &path, Some(&body)).await
            }
            C::InviteEmail { org, email, role } => {
                let role = match role {
                    OrgRole::Owner => "admin",
                    OrgRole::Member => "direct_member",
                };
                let body = json!({ "email": email, "role": role });
                let path = format!("/orgs/{org}/invitations");
                self.rest_write(Method::POST, &path, Some(&body)).await
            }
            C::RemoveMember { org, login } => {
                let path = format!("/orgs/{org}/members/{login}");
                self.rest_write(Method::DELETE, &path, None).await
            }
            C::CancelInvitation { org, id, .. } => {
                let path = format!("/orgs/{org}/invitations/{id}");
                self.rest_write(Method::DELETE, &path, None).await
            }
            C::CreateTeam { org, name, secret } => {
                let privacy = if *secret { "secret" } else { "closed" };
                let body = json!({ "name": name, "privacy": privacy });
                let path = format!("/orgs/{org}/teams");
                self.rest_write(Method::POST, &path, Some(&body)).await
            }
            C::SetTeamMember {
                org,
                team: slug,
                login,
                maintainer,
                ..
            } => {
                let role = if *maintainer { "maintainer" } else { "member" };
                let path = format!("{}/memberships/{login}", team(org, slug));
                let body = json!({ "role": role });
                self.rest_write(Method::PUT, &path, Some(&body)).await
            }
            C::RemoveTeamMember {
                org,
                team: slug,
                login,
            } => {
                let path = format!("{}/memberships/{login}", team(org, slug));
                self.rest_write(Method::DELETE, &path, None).await
            }
            C::SetTeamRepo {
                org,
                team: slug,
                repo,
                permission,
                ..
            } => {
                let path = format!("{}/repos/{repo}", team(org, slug));
                let body = json!({ "permission": permission.wire() });
                self.rest_write(Method::PUT, &path, Some(&body)).await
            }
            C::RemoveTeamRepo {
                org,
                team: slug,
                repo,
            } => {
                let path = format!("{}/repos/{repo}", team(org, slug));
                self.rest_write(Method::DELETE, &path, None).await
            }
            C::AnswerRepo { id, accept, .. } => {
                let method = if *accept {
                    Method::PATCH
                } else {
                    Method::DELETE
                };
                let path = format!("/user/repository_invitations/{id}");
                self.rest_write(method, &path, None).await
            }
            C::JoinOrg { org } => {
                let body = json!({ "state": "active" });
                let path = format!("/user/memberships/orgs/{org}");
                self.rest_write(Method::PATCH, &path, Some(&body)).await
            }
        }
    }
}

/// An organization's invitation, as its owners see it.
fn invitation(i: wire::Invitation) -> OrgInvitation {
    OrgInvitation {
        id: i.id,
        login: i.login,
        email: i.email,
        role: invitation_role(&i.role),
        created_at: i.created_at,
        inviter: i.inviter.map(|l| l.login),
        failed: i.failed_at.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invitations_role_is_named_as_the_website_names_it() {
        assert_eq!(invitation_role("direct_member"), "Member");
        assert_eq!(invitation_role("admin"), "Owner");
        assert_eq!(invitation_role("billing_manager"), "Billing manager");
        assert_eq!(OrgRole::from_graphql("ADMIN"), Some(OrgRole::Owner));
        assert!(can_write_org("repo, admin:org") && !can_write_org("repo, read:org"));
    }
}
