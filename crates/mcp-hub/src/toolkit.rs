//! A capability offered through the hub.
use crate::Grant;
use serde_json::{json, Value};

/// A set of tools and the calls behind them. A toolkit decodes input, checks
/// the caller and maps errors; the work stays in the service it wraps.
pub trait Toolkit: Send + Sync + 'static {
    /// Matches an entry in [`Grant::toolkits`]. A session sees a toolkit only
    /// when its grant names it.
    fn name(&self) -> &'static str;

    /// MCP tool definitions for this caller. Read at each `tools/list`, so a
    /// scope that changes mid-session changes the list, and again at each
    /// `tools/call` to gate it, so this must be cheap and agree with what was
    /// listed.
    fn tools(&self, grant: &Grant) -> Vec<Value>;

    /// Runs on a blocking thread and may wait on the app. An `Err` reaches the
    /// agent as a tool error, so its turn carries on.
    fn call(&self, grant: &Grant, tool: &str, arguments: Value) -> Result<Value, String>;
}

/// The `tools/call` result shape shared by the toolkits and the relay.
pub fn text_result(result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}],"isError":false}),
        Err(error) => json!({"content":[{"type":"text","text":error}],"isError":true}),
    }
}
