//! A repository's custom properties: the values its organization's
//! properties have for it, and what each property allows.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::api::client::{ApiError, GitHub};
use crate::api::model::RepoId;

/// A property's value: one string, or several (a multi select).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropValue {
    One(String),
    Many(Vec<String>),
}

impl PropValue {
    /// As the page and the form write it.
    pub fn text(&self) -> String {
        match self {
            PropValue::One(s) => s.clone(),
            PropValue::Many(v) => v.join(", "),
        }
    }
}

words! {
    #[derive(Serialize, Deserialize)]
    pub enum PropKind { pub fn word -> pub fn from_word, pub fn label }
    Text   "string"        "Text";
    Select "single_select" "Single select";
    Multi  "multi_select"  "Multi select";
    Bool   "true_false"    "True/false";
    Url    "url"           "URL";
}

/// One of the organization's properties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Property {
    pub name: String,
    pub kind: PropKind,
    pub required: bool,
    pub description: Option<String>,
    /// A select's values, in order.
    pub allowed: Vec<String>,
    /// The repository's admins may set it, not only the organization's.
    pub repo_may: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Properties {
    /// Each property's value, by its name; `None` is unset.
    pub values: Vec<(String, Option<PropValue>)>,
    /// The organization's properties, or why GitHub won't say them.
    pub schema: Result<Vec<Property>, String>,
}

impl Properties {
    pub fn value(&self, name: &str) -> Option<&PropValue> {
        (self.values.iter())
            .find(|(n, _)| n == name)
            .and_then(|(_, v)| v.as_ref())
    }
}

/// Values to set, by property name; `None` unsets one.
pub type PropertyValues = Vec<(String, Option<PropValue>)>;

mod wire {
    use super::PropValue;
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub(super) struct Value {
        pub property_name: String,
        pub value: Option<PropValue>,
    }

    #[derive(Deserialize)]
    pub(super) struct Property {
        pub property_name: String,
        #[serde(default)]
        pub value_type: String,
        #[serde(default)]
        pub required: bool,
        pub description: Option<String>,
        pub allowed_values: Option<Vec<String>>,
        pub values_editable_by: Option<String>,
    }
}

impl GitHub {
    pub(super) async fn properties(&self, repo: &RepoId) -> Result<Properties, ApiError> {
        let values = format!("/repos/{repo}/properties/values");
        let schema = format!("/orgs/{}/properties/schema", repo.owner);
        let schema = self.rest_refusable(&schema, "its properties", |status, _, _| {
            matches!(status, 403 | 404).then(|| {
                format!(
                    "GitHub says {}'s properties only to its members, and only an organization has them",
                    repo.owner
                )
            })
        });
        let (values, schema) = tokio::join!(
            self.rest_json::<Vec<wire::Value>>(&values, format!("{repo}'s custom properties")),
            schema
        );
        let values = values?
            .into_iter()
            .map(|v| (v.property_name, v.value))
            .collect();
        let schema = match schema? {
            Err(why) => Err(why),
            Ok(answer) => {
                let found: Vec<wire::Property> = serde_json::from_str(answer.body())?;
                Ok(found
                    .into_iter()
                    .map(|p| Property {
                        name: p.property_name,
                        kind: PropKind::from_word(&p.value_type).unwrap_or(PropKind::Text),
                        required: p.required,
                        description: p.description.filter(|d| !d.is_empty()),
                        allowed: p.allowed_values.unwrap_or_default(),
                        repo_may: p.values_editable_by.as_deref() == Some("org_and_repo_actors"),
                    })
                    .collect())
            }
        };
        Ok(Properties { values, schema })
    }

    pub(super) async fn set_properties(
        &self,
        repo: &RepoId,
        values: &PropertyValues,
    ) -> Result<(), ApiError> {
        let properties: Vec<_> = (values.iter())
            .map(|(name, value)| json!({ "property_name": name, "value": value }))
            .collect();
        let body = json!({ "properties": properties });
        let path = format!("/repos/{repo}/properties/values");
        self.rest_write(http::Method::PATCH, &path, Some(&body))
            .await
    }
}
