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
    ///
    /// An `Ok` value that is an object with a `content` array is passed on as
    /// the MCP `CallToolResult` it describes, so a toolkit can return image
    /// blocks and set `isError` itself. If it doesn't decode as a result, or
    /// is any other value, it reaches the agent as one text block of its JSON.
    fn call(&self, grant: &Grant, tool: &str, arguments: Value) -> Result<Value, String>;

    /// Claim calls for a tool this toolkit no longer lists, so it can explain
    /// why. An agent may still call a tool it saw a moment ago, after the
    /// toolkit dropped it from [`Toolkit::tools`]; without a claim the caller
    /// only learns that the tool isn't available. A toolkit that claims a
    /// tool gets the call and must refuse it itself when it may not run.
    /// Only toolkits the grant names are asked. Off by default.
    fn routes(&self, _grant: &Grant, _tool: &str) -> bool {
        false
    }
}

/// The `tools/call` result shape shared by the toolkits and the relay.
pub fn text_result(result: Result<Value, String>) -> Value {
    match result {
        Ok(value) => json!({"content":[{"type":"text","text":value.to_string()}],"isError":false}),
        Err(error) => json!({"content":[{"type":"text","text":error}],"isError":true}),
    }
}
