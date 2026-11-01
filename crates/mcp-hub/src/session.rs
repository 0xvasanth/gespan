//! One MCP client of the hub. The transport keeps it for the life of the
//! connection, so nothing about the caller is cached here: every request names
//! its grant, set by the gate from the token on that request.
use crate::{
    endpoint::Inner,
    pool::{split_resource_uri, PROMPTS, RESOURCES, SEPARATOR, TOOLS},
    Grant,
};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
        GetPromptRequestParams, GetPromptResponse, GetPromptResult, Implementation,
        ListPromptsResult, ListResourcesResult, ListToolsResult, PaginatedRequestParams, Prompt,
        ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
        ServerCapabilities, ServerConfig, SetLevelRequestParams, SubscribeRequestParams,
        SubscriptionFilter, Tool, UnsubscribeRequestParams,
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

fn internal(error: impl std::fmt::Display) -> ErrorData {
    ErrorData::internal_error(error.to_string(), None)
}

/// A server's list items. One that doesn't parse is left out, not fatal.
fn decode_all<T: serde::de::DeserializeOwned>(items: Vec<Value>) -> Vec<T> {
    items
        .into_iter()
        .filter_map(|item| serde_json::from_value(item).ok())
        .collect()
}

fn decode_answer<T: serde::de::DeserializeOwned>(
    answer: Option<Result<Value, Value>>,
    unknown: &str,
) -> Result<T, ErrorData> {
    match answer {
        None => Err(ErrorData::invalid_params(unknown.to_owned(), None)),
        Some(Ok(result)) => serde_json::from_value(result).map_err(internal),
        Some(Err(error)) => Err(ErrorData::internal_error(
            error["message"]
                .as_str()
                .unwrap_or("The server failed")
                .to_owned(),
            None,
        )),
    }
}

impl Session {
    async fn watch_resource(
        &self,
        uri: &str,
        on: bool,
        context: &RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let grant = caller(context)?;
        let (server, uri) = split_resource_uri(uri)
            .ok_or_else(|| ErrorData::invalid_params("Unknown resource", None))?;
        self.inner
            .pool
            .subscribe(&grant, server, uri, on)
            .await
            .map_err(internal)
    }
}

impl ServerHandler for Session {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .enable_prompts()
                .enable_prompts_list_changed()
                .enable_resources()
                .enable_resources_list_changed()
                .enable_resources_subscribe()
                .enable_logging()
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
        let mut tools = self.inner.tools(&grant);
        tools.extend(self.inner.pool.listed(&grant, TOOLS).await);
        let tools = decode_all::<Tool>(tools);
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let grant = caller(&context)?;
        let name = request.name.to_string();
        if let Some((server, tool)) = name.split_once(SEPARATOR) {
            let mut params = serde_json::to_value(&request).map_err(internal)?;
            params["name"] = tool.into();
            self.inner.calling(grant.session, context.peer.clone());
            let call = self
                .inner
                .pool
                .forward(&grant, server, "tools/call", params);
            let answer = tokio::select! {
                answer = call => answer,
                () = context.ct.cancelled() => {
                    return Err(ErrorData::internal_error("The tool call was cancelled", None));
                }
            };
            if let Some(answer) = answer {
                let result = match answer {
                    Ok(result) => {
                        serde_json::from_value::<CallToolResult>(result).map_err(internal)?
                    }
                    Err(error) => CallToolResult::error(vec![ContentBlock::text(
                        error["message"]
                            .as_str()
                            .unwrap_or("The tool failed")
                            .to_owned(),
                    )]),
                };
                return Ok(result.into());
            }
        }
        let inner = self.inner.clone();
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

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        let grant = caller(&context)?;
        let prompts = self.inner.pool.listed(&grant, PROMPTS).await;
        Ok(ListPromptsResult::with_all_items(decode_all::<Prompt>(
            prompts,
        )))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        let grant = caller(&context)?;
        let (server, prompt) = request
            .name
            .split_once(SEPARATOR)
            .ok_or_else(|| ErrorData::invalid_params("Unknown prompt", None))?;
        let mut params = serde_json::to_value(&request).map_err(internal)?;
        params["name"] = prompt.into();
        let answer = self
            .inner
            .pool
            .forward(&grant, server, "prompts/get", params)
            .await;
        decode_answer::<GetPromptResult>(answer, "Unknown prompt").map(Into::into)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let grant = caller(&context)?;
        let resources = self.inner.pool.listed(&grant, RESOURCES).await;
        Ok(ListResourcesResult::with_all_items(decode_all::<Resource>(
            resources,
        )))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let grant = caller(&context)?;
        let (server, uri) = split_resource_uri(&request.uri)
            .ok_or_else(|| ErrorData::invalid_params("Unknown resource", None))?;
        let params = serde_json::json!({"uri":uri});
        let answer = self
            .inner
            .pool
            .forward(&grant, server, "resources/read", params)
            .await;
        decode_answer::<ReadResourceResult>(answer, "Unknown resource").map(Into::into)
    }

    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        self.watch_resource(&request.uri, true, &context).await
    }

    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        self.watch_resource(&request.uri, false, &context).await
    }

    /// Kept per session: a shared server's level is never set for one agent.
    async fn set_level(
        &self,
        request: SetLevelRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let grant = caller(&context)?;
        let level = serde_json::to_value(request.level).map_err(internal)?;
        self.inner
            .set_level(grant.session, level.as_str().unwrap_or("info"));
        Ok(())
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
