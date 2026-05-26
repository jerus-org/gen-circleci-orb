use anyhow::{Context, Result};

/// Ensure a CircleCI orb is registered, creating it if it does not exist.
///
/// Uses the CircleCI GraphQL API directly so it runs in any executor that has
/// the gen-circleci-orb binary — no `circleci` developer CLI required.
#[derive(Debug, clap::Args)]
pub struct EnsureOrbRegistered {
    /// The orb name to check/register (e.g. my-org/my-orb).
    #[arg(long)]
    pub orb_name: String,

    /// Register the orb as private when creating it.
    ///
    /// Must be set correctly on first creation — orb visibility cannot be
    /// changed after the orb is created.
    #[arg(long)]
    pub private: bool,
}

/// Abstraction over orb registration for testability.
pub(crate) trait OrbRegistrar {
    /// Returns true if the orb is already registered.
    fn is_registered(&self, orb_name: &str) -> Result<bool>;
    /// Creates the orb. Returns Ok if created or already exists.
    fn create_orb(&self, orb_name: &str, private: bool) -> Result<()>;
}

/// Calls the CircleCI GraphQL API at https://circleci.com/graphql-unstable.
pub(crate) struct CircleCiApi {
    token: String,
    client: reqwest::blocking::Client,
}

impl CircleCiApi {
    const URL: &'static str = "https://circleci.com/graphql-unstable";

    pub fn new(token: String) -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { token, client })
    }

    fn graphql(&self, query: &str, variables: serde_json::Value) -> Result<serde_json::Value> {
        let resp = self
            .client
            .post(Self::URL)
            .header("Authorization", &self.token)
            .json(&serde_json::json!({ "query": query, "variables": variables }))
            .send()
            .context("GraphQL request failed")?;

        let status = resp.status();
        let body: serde_json::Value = resp.json().context("failed to parse GraphQL response")?;

        if !status.is_success() {
            anyhow::bail!("GraphQL HTTP {} : {}", status, body);
        }

        if let Some(errors) = body.get("errors").and_then(|e| e.as_array()) {
            if !errors.is_empty() {
                anyhow::bail!(
                    "GraphQL errors: {}",
                    errors
                        .iter()
                        .filter_map(|e| e["message"].as_str())
                        .collect::<Vec<_>>()
                        .join("; ")
                );
            }
        }

        Ok(body)
    }
}

impl OrbRegistrar for CircleCiApi {
    fn is_registered(&self, orb_name: &str) -> Result<bool> {
        let namespace = orb_name
            .split('/')
            .next()
            .context("orb_name must be namespace/name")?;

        let resp = self.graphql(
            r#"query ($name: String!, $namespace: String) {
                orb(name: $name) { id }
                registryNamespace(name: $namespace) { id }
            }"#,
            serde_json::json!({ "name": orb_name, "namespace": namespace }),
        )?;

        Ok(resp["data"]["orb"]["id"]
            .as_str()
            .map(|s| !s.is_empty())
            .unwrap_or(false))
    }

    fn create_orb(&self, orb_name: &str, private: bool) -> Result<()> {
        let (namespace, name) = orb_name
            .split_once('/')
            .context("orb_name must be namespace/name")?;

        let ns_resp = self.graphql(
            r#"query($name: String!) { registryNamespace(name: $name) { id } }"#,
            serde_json::json!({ "name": namespace }),
        )?;

        let ns_id = ns_resp["data"]["registryNamespace"]["id"]
            .as_str()
            .with_context(|| format!("namespace '{namespace}' not found"))?
            .to_owned();

        let create_resp = self.graphql(
            r#"mutation($name: String!, $registryNamespaceId: UUID!, $isPrivate: Boolean!) {
                createOrb(
                    name: $name,
                    registryNamespaceId: $registryNamespaceId,
                    isPrivate: $isPrivate
                ) {
                    orb { id }
                    errors { message type }
                }
            }"#,
            serde_json::json!({
                "name": name,
                "registryNamespaceId": ns_id,
                "isPrivate": private,
            }),
        )?;

        if let Some(errors) = create_resp["data"]["createOrb"]["errors"].as_array() {
            if !errors.is_empty() {
                let msg = errors[0]["message"].as_str().unwrap_or("unknown error");
                if msg.contains("already exists") {
                    return Ok(());
                }
                anyhow::bail!("createOrb failed: {msg}");
            }
        }

        Ok(())
    }
}

impl EnsureOrbRegistered {
    pub fn run(&self) -> Result<()> {
        let token =
            std::env::var("CIRCLE_TOKEN").context("CIRCLE_TOKEN environment variable not set")?;
        let api = CircleCiApi::new(token)?;
        self.run_with_registrar(&api)
    }

    pub(crate) fn run_with_registrar<R: OrbRegistrar>(&self, registrar: &R) -> Result<()> {
        if registrar.is_registered(&self.orb_name)? {
            println!("Orb is registered.");
            return Ok(());
        }

        registrar.create_orb(&self.orb_name, self.private)?;
        println!("Orb is registered.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct FakeRegistrar {
        registered: bool,
        create_should_fail: bool,
        create_call_count: Cell<u32>,
        create_private_arg: Cell<Option<bool>>,
        create_orb_name_arg: std::cell::RefCell<Option<String>>,
    }

    impl FakeRegistrar {
        fn new(registered: bool) -> Self {
            Self {
                registered,
                create_should_fail: false,
                create_call_count: Cell::new(0),
                create_private_arg: Cell::new(None),
                create_orb_name_arg: std::cell::RefCell::new(None),
            }
        }
        fn failing() -> Self {
            Self {
                create_should_fail: true,
                ..Self::new(false)
            }
        }
        fn create_call_count(&self) -> u32 {
            self.create_call_count.get()
        }
        fn create_private_arg(&self) -> Option<bool> {
            self.create_private_arg.get()
        }
        fn create_orb_name_arg(&self) -> Option<String> {
            self.create_orb_name_arg.borrow().clone()
        }
    }

    impl OrbRegistrar for FakeRegistrar {
        fn is_registered(&self, _orb_name: &str) -> Result<bool> {
            Ok(self.registered)
        }
        fn create_orb(&self, orb_name: &str, private: bool) -> Result<()> {
            self.create_call_count.set(self.create_call_count.get() + 1);
            self.create_private_arg.set(Some(private));
            *self.create_orb_name_arg.borrow_mut() = Some(orb_name.to_owned());
            if self.create_should_fail {
                anyhow::bail!("create failed");
            }
            Ok(())
        }
    }

    fn cmd(orb_name: &str) -> EnsureOrbRegistered {
        EnsureOrbRegistered {
            orb_name: orb_name.to_string(),
            private: false,
        }
    }

    #[test]
    fn already_registered_returns_ok() {
        let r = FakeRegistrar::new(true);
        assert!(cmd("my-org/my-orb").run_with_registrar(&r).is_ok());
    }

    #[test]
    fn already_registered_skips_create() {
        let r = FakeRegistrar::new(true);
        cmd("my-org/my-orb").run_with_registrar(&r).unwrap();
        assert_eq!(r.create_call_count(), 0);
    }

    #[test]
    fn not_registered_calls_create() {
        let r = FakeRegistrar::new(false);
        cmd("my-org/my-orb").run_with_registrar(&r).unwrap();
        assert_eq!(r.create_call_count(), 1);
    }

    #[test]
    fn create_receives_orb_name() {
        let r = FakeRegistrar::new(false);
        cmd("my-org/my-orb").run_with_registrar(&r).unwrap();
        assert_eq!(r.create_orb_name_arg().as_deref(), Some("my-org/my-orb"));
    }

    #[test]
    fn create_private_true_passed_through() {
        let r = FakeRegistrar::new(false);
        EnsureOrbRegistered {
            orb_name: "my-org/my-orb".into(),
            private: true,
        }
        .run_with_registrar(&r)
        .unwrap();
        assert_eq!(r.create_private_arg(), Some(true));
    }

    #[test]
    fn create_private_false_passed_through() {
        let r = FakeRegistrar::new(false);
        cmd("my-org/my-orb").run_with_registrar(&r).unwrap();
        assert_eq!(r.create_private_arg(), Some(false));
    }

    #[test]
    fn create_failure_propagates_error() {
        let r = FakeRegistrar::failing();
        assert!(cmd("my-org/my-orb").run_with_registrar(&r).is_err());
    }

    #[test]
    fn not_registered_returns_ok_when_create_succeeds() {
        let r = FakeRegistrar::new(false);
        assert!(cmd("my-org/my-orb").run_with_registrar(&r).is_ok());
    }
}
