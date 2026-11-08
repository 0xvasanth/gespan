//! One MCP client of the hub. The transport keeps it for the life of the
//! connection, so nothing about the caller is cached here: every request names
//! its grant, set by the gate from the token on that request.
use crate::{
    endpoint::{Inner, Listing},
    pool::{split_resource_uri, PROMPTS, RESOURCES, SEPARATOR, TOOLS},
    upstream::MALFORMED,
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

/// Marks a result as final. Protocol 2026-07-28 requires it of the hub whatever
/// version the server behind it speaks, and older agents ignore it.
fn complete(mut result: Value) -> Value {
    if let Some(object) = result.as_object_mut() {
        object
            .entry("resultType")
            .or_insert_with(|| "complete".into());
    }
    result
}

/// A server's list items. One that doesn't parse is left out, not fatal.
fn decode_all<T: serde::de::DeserializeOwned>(items: Vec<Value>) -> Vec<T> {
    items
        .into_iter()
        .filter_map(|item| {
            serde_json::from_value(item)
                .inspect_err(|error| tracing::warn!(%error, "a list item didn't parse"))
                .ok()
        })
        .collect()
}

fn decode_answer<T: serde::de::DeserializeOwned>(
    answer: Option<Result<Value, Value>>,
    unknown: &str,
) -> Result<T, ErrorData> {
    match answer {
        None => Err(ErrorData::invalid_params(unknown.to_owned(), None)),
        Some(Ok(result)) => serde_json::from_value(complete(result)).map_err(internal),
        Some(Err(error)) => Err(ErrorData::internal_error(
            error["message"]
                .as_str()
                .unwrap_or("The server failed")
                .to_owned(),
            None,
        )),
    }
}

async fn announce(context: &SubscriptionContext, listing: Listing) -> bool {
    let sink = context.sink();
    match listing {
        Listing::Tools => sink.notify_tool_list_changed().await,
        Listing::Prompts => sink.notify_prompt_list_changed().await,
        Listing::Resources => sink.notify_resource_list_changed().await,
    }
    .is_ok()
}

impl Session {
    /// The caller of anything that acts on a server or reads from one. A
    /// bound credential serves only calls that name its own caller; lists
    /// aren't held to it, since a client that shares its connections lists
    /// for all of them at once.
    fn acting(&self, context: &RequestContext<RoleServer>) -> Result<Grant, ErrorData> {
        let grant = caller(context)?;
        self.inner
            .check_caller(grant.session, &context.meta.0 .0)
            .map_err(|message| ErrorData::invalid_request(message, None))?;
        Ok(grant)
    }

    /// A request to the server behind a name. The caller giving up stops it.
    async fn forward(
        &self,
        grant: &Grant,
        context: &RequestContext<RoleServer>,
        server: &str,
        method: &str,
        mut params: Value,
    ) -> Result<Option<Result<Value, Value>>, ErrorData> {
        self.inner
            .calling(grant.session, context.peer.clone(), context.id.clone());
        // rmcp lifts `_meta`, with any progress token, out of the params.
        if !context.meta.is_empty() {
            params["_meta"] = serde_json::to_value(&context.meta).map_err(internal)?;
        }
        tokio::select! {
            answer = self.inner.pool.forward(grant, server, method, params) => Ok(answer),
            () = context.ct.cancelled() => {
                Err(ErrorData::internal_error("The request was cancelled", None))
            }
        }
    }

    async fn watch_resource(
        &self,
        uri: &str,
        on: bool,
        context: &RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        let grant = self.acting(context)?;
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
        let grant = self.acting(&context)?;
        let name = request.name.to_string();
        if let Some((server, tool)) = name.split_once(SEPARATOR) {
            let mut params = serde_json::to_value(&request).map_err(internal)?;
            params["name"] = tool.into();
            let answer = self
                .forward(&grant, &context, server, "tools/call", params)
                .await?;
            if let Some(answer) = answer {
                let result = match answer {
                    Ok(result) => serde_json::from_value::<CallToolResult>(complete(result))
                        .map_err(internal)?,

                    // An answer that isn't one fails the call, not the tool.
                    Err(error) if error["code"] == MALFORMED => {
                        return Err(ErrorData::internal_error(
                            error["message"].as_str().unwrap_or("").to_owned(),
                            None,
                        ));
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
            Ok(value) => tool_result(value),
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
        let grant = self.acting(&context)?;
        let (server, prompt) = request
            .name
            .split_once(SEPARATOR)
            .ok_or_else(|| ErrorData::invalid_params("Unknown prompt", None))?;
        let mut params = serde_json::to_value(&request).map_err(internal)?;
        params["name"] = prompt.into();
        let answer = self
            .forward(&grant, &context, server, "prompts/get", params)
            .await?;
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
        let grant = self.acting(&context)?;
        let (server, uri) = split_resource_uri(&request.uri)
            .ok_or_else(|| ErrorData::invalid_params("Unknown resource", None))?;
        let mut params = serde_json::to_value(&request).map_err(internal)?;
        params["uri"] = uri.into();
        let answer = self
            .forward(&grant, &context, server, "resources/read", params)
            .await?;
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
                    Ok((session, _)) if session != Some(grant.session) && session.is_some() => {}
                    Ok((_, listing)) => {
                        if !announce(&context, listing).await {
                            return Ok(());
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        for listing in Listing::ALL {
                            if !announce(&context, listing).await {
                                return Ok(());
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return Ok(()),
                },
            }
        }
    }
}

/// A toolkit's value as a result. An object with a `content` array that
/// decodes as a result is one already, so it keeps its blocks and `isError`;
/// anything else, or a decode failure, is the value as one text block.
fn tool_result(value: Value) -> CallToolResult {
    if value["content"].is_array() {
        if let Ok(result) = serde_json::from_value::<CallToolResult>(value.clone()) {
            return result;
        }
    }
    CallToolResult::success(vec![ContentBlock::text(value.to_string())])
}
