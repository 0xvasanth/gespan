//! One MCP client of the hub. The transport keeps it for the life of the
//! connection, so nothing about the caller is cached here: every request names
//! its grant, set by the gate from the token on that request.
use crate::{endpoint::Inner, Grant};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Implementation,
        ListToolsResult, PaginatedRequestParams, ServerCapabilities, ServerConfig,
        SubscriptionFilter, Tool,
    },
    service::{NotificationContext, RequestContext, RoleServer, SubscriptionContext},
    ErrorData, ServerHandler,
};
use serde_json::Value;
use std::sync::Arc;

pub(crate) struct Session {
    pub(crate) inner: Arc<Inner>,
}

fn grant_from(extensions: &rmcp::model::Extensions) -> Option<Grant> {
    extensions
        .get::<axum::http::request::Parts>()?
        .extensions
        .get::<Grant>()
        .cloned()
}

fn caller(context: &RequestContext<RoleServer>) -> Result<Grant, ErrorData> {
    grant_from(&context.extensions)
        .ok_or_else(|| ErrorData::invalid_request("Request carried no session", None))
}

impl ServerHandler for Session {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .build(),
        )
        .with_server_info(Implementation::new(
            self.inner.name.clone(),
            self.inner.version.clone(),
        ))
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let grant = caller(&context)?;
        let tools = self
            .inner
            .tools(&grant)
            .into_iter()
            .filter_map(|tool| {
                serde_json::from_value::<Tool>(tool)
                    .inspect_err(
                        |error| tracing::warn!(%error, "a toolkit offered a malformed tool"),
                    )
                    .ok()
            })
            .collect();
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let grant = caller(&context)?;
        let inner = self.inner.clone();
        let name = request.name.to_string();
        let arguments = request
            .arguments
            .map_or_else(|| Value::Object(Default::default()), Value::Object);
        let call = tokio::task::spawn_blocking(move || inner.call(&grant, &name, arguments));
        let value = tokio::select! {
            result = call => result.map_err(|_| ErrorData::internal_error("The tool stopped", None))?,
            () = context.ct.cancelled() => Err("The tool call was cancelled".to_owned()),
        };
        Ok(match value {
            Ok(value) => CallToolResult::success(vec![ContentBlock::text(value.to_string())]),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error)]),
        }
        .into())
    }

    async fn on_initialized(&self, context: NotificationContext<RoleServer>) {
        if let Some(grant) = grant_from(&context.extensions) {
            self.inner.watch(grant.session, context.peer);
        }
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.clone())
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        let Some(grant) = grant_from(&context.request_context().extensions) else {
            return Ok(());
        };
        let mut changes = self.inner.changes();
        loop {
            tokio::select! {
                () = context.cancelled() => return Ok(()),
                change = changes.recv() => match change {
                    Ok(session) if session != Some(grant.session) && session.is_some() => {}
                    Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        if context.sink().notify_tool_list_changed().await.is_err() {
                            return Ok(());
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
                },
            }
        }
    }
}
