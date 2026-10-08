// SEAM (wave 2): types are the contract; bodies are filled by the actions/destinations owner.
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Destination {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub config: serde_json::Value,
    pub created_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NewDestination {
    pub name: String,
    pub kind: String,
    pub config: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DestinationTest {
    pub ok: bool,
    pub free_bytes: Option<i64>,
    pub error: Option<String>,
}

pub fn list(_conn: &Connection) -> anyhow::Result<Vec<Destination>> {
    anyhow::bail!("destinations::list not implemented yet")
}

pub fn get(_conn: &Connection, _id: &str) -> anyhow::Result<Option<Destination>> {
    anyhow::bail!("destinations::get not implemented yet")
}

pub fn create(_conn: &Connection, _new: NewDestination) -> anyhow::Result<Destination> {
    anyhow::bail!("destinations::create not implemented yet")
}

pub fn update(_conn: &Connection, _id: &str, _new: NewDestination) -> anyhow::Result<Destination> {
    anyhow::bail!("destinations::update not implemented yet")
}

pub fn delete(_conn: &Connection, _id: &str) -> anyhow::Result<bool> {
    anyhow::bail!("destinations::delete not implemented yet")
}

pub fn test(_dest: &Destination, _secrets: &dyn crate::vault::SecretStore) -> DestinationTest {
    DestinationTest {
        ok: false,
        free_bytes: None,
        error: Some("destination test not implemented yet".into()),
    }
}
