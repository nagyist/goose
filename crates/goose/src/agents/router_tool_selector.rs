use mcp_core::ToolError;
use rmcp::model::Content;
use rmcp::model::Tool;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::env;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::agents::tool_vectordb::ToolVectorDB;
use crate::message::Message;
use crate::model::ModelConfig;
use crate::providers::{self, base::Provider};

#[derive(Debug, Clone, PartialEq)]
pub enum RouterToolSelectionStrategy {
    Vector,
    Llm,
}

#[async_trait]
pub trait RouterToolSelector: Send + Sync {
    async fn select_tools(&self, params: Value) -> Result<Vec<Content>, ToolError>;
    async fn index_tools(&self, tools: &[Tool], extension_name: &str) -> Result<(), ToolError>;
    async fn remove_tool(&self, tool_name: &str) -> Result<(), ToolError>;
    async fn record_tool_call(&self, tool_name: &str) -> Result<(), ToolError>;
    async fn get_recent_tool_calls(&self, limit: usize) -> Result<Vec<String>, ToolError>;
    fn selector_type(&self) -> RouterToolSelectionStrategy;
}

pub struct VectorToolSelector {
    vector_db: Arc<RwLock<ToolVectorDB>>,
    embedding_provider: Arc<dyn Provider>,
    recent_tool_calls: Arc<RwLock<VecDeque<String>>>,
}

impl VectorToolSelector {
    pub async fn new(provider: Arc<dyn Provider>, table_name: String) -> Result<Self> {
        let vector_db = ToolVectorDB::new(Some(table_name)).await?;

        let embedding_provider = if env::var("GOOSE_EMBEDDING_MODEL_PROVIDER").is_ok() {
            // If env var is set, create a new provider for embeddings
            // Get embedding model and provider from environment variables
            let embedding_model = env::var("GOOSE_EMBEDDING_MODEL")
                .unwrap_or_else(|_| "text-embedding-3-small".to_string());
            let embedding_provider_name =
                env::var("GOOSE_EMBEDDING_MODEL_PROVIDER").unwrap_or_else(|_| "openai".to_string());

            // Create the provider using the factory
            let model_config = ModelConfig::new(embedding_model.as_str())
                .context("Failed to create model config for embedding provider")?;
            providers::create(&embedding_provider_name, model_config).context(format!(
                "Failed to create {} provider for embeddings. If using OpenAI, make sure OPENAI_API_KEY env var is set or that you have configured the OpenAI provider via Goose before.",
                embedding_provider_name
            ))?
        } else {
            // Otherwise fall back to using the same provider instance as used for base goose model
            provider.clone()
        };

        Ok(Self {
            vector_db: Arc::new(RwLock::new(vector_db)),
            embedding_provider,
            recent_tool_calls: Arc::new(RwLock::new(VecDeque::with_capacity(100))),
        })
    }
}

#[async_trait]
impl RouterToolSelector for VectorToolSelector {
    async fn select_tools(&self, params: Value) -> Result<Vec<Content>, ToolError> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'query' parameter".to_string()))?;

        let k = params.get("k").and_then(|v| v.as_u64()).unwrap_or(5) as usize;

        // Extract extension_name from params if present
        let extension_name = params.get("extension_name").and_then(|v| v.as_str());

        // Check if provider supports embeddings
        if !self.embedding_provider.supports_embeddings() {
            return Err(ToolError::ExecutionError(
                "Embedding provider does not support embeddings".to_string(),
            ));
        }

        let embeddings = self
            .embedding_provider
            .create_embeddings(vec![query.to_string()])
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to generate query embedding: {}", e))
            })?;

        let query_embedding = embeddings
            .into_iter()
            .next()
            .ok_or_else(|| ToolError::ExecutionError("No embedding returned".to_string()))?;

        let vector_db = self.vector_db.read().await;
        let tools = vector_db
            .search_tools(query_embedding, k, extension_name)
            .await
            .map_err(|e| ToolError::ExecutionError(format!("Failed to search tools: {}", e)))?;

        let selected_tools: Vec<Content> = tools
            .into_iter()
            .map(|tool| {
                let text = format!(
                    "Tool: {}\nDescription: {}\nSchema: {}",
                    tool.tool_name, tool.description, tool.schema
                );
                Content::text(text)
            })
            .collect();

        Ok(selected_tools)
    }

    async fn index_tools(&self, tools: &[Tool], extension_name: &str) -> Result<(), ToolError> {
        let texts_to_embed: Vec<String> = tools
            .iter()
            .map(|tool| {
                let schema_str = serde_json::to_string_pretty(&tool.input_schema)
                    .unwrap_or_else(|_| "{}".to_string());
                format!(
                    "{} {} {}",
                    tool.name,
                    tool.description
                        .as_ref()
                        .map(|d| d.as_ref())
                        .unwrap_or_default(),
                    schema_str
                )
            })
            .collect();

        if !self.embedding_provider.supports_embeddings() {
            return Err(ToolError::ExecutionError(
                "Embedding provider does not support embeddings".to_string(),
            ));
        }

        let embeddings = self
            .embedding_provider
            .create_embeddings(texts_to_embed)
            .await
            .map_err(|e| {
                ToolError::ExecutionError(format!("Failed to generate tool embeddings: {}", e))
            })?;

        // Create tool records
        let tool_records: Vec<crate::agents::tool_vectordb::ToolRecord> = tools
            .iter()
            .zip(embeddings.into_iter())
            .map(|(tool, vector)| {
                let schema_str = serde_json::to_string_pretty(&tool.input_schema)
                    .unwrap_or_else(|_| "{}".to_string());
                crate::agents::tool_vectordb::ToolRecord {
                    tool_name: tool.name.to_string(),
                    description: tool
                        .description
                        .as_ref()
                        .map(|d| d.to_string())
                        .unwrap_or_default(),
                    schema: schema_str,
                    vector,
                    extension_name: extension_name.to_string(),
                }
            })
            .collect();

        // Get vector_db lock
        let vector_db = self.vector_db.read().await;

        // Filter out tools that already exist in the database
        let mut new_tool_records = Vec::new();
        for record in tool_records {
            // Check if tool exists by searching for it
            let existing_tools = vector_db
                .search_tools(record.vector.clone(), 1, Some(&record.extension_name))
                .await
                .map_err(|e| {
                    ToolError::ExecutionError(format!("Failed to search for existing tools: {}", e))
                })?;

            // Only add if no exact match found
            if !existing_tools
                .iter()
                .any(|t| t.tool_name == record.tool_name)
            {
                new_tool_records.push(record);
            }
        }

        // Only index if there are new tools to add
        if !new_tool_records.is_empty() {
            vector_db
                .index_tools(new_tool_records)
                .await
                .map_err(|e| ToolError::ExecutionError(format!("Failed to index tools: {}", e)))?;
        }

        Ok(())
    }

    async fn remove_tool(&self, tool_name: &str) -> Result<(), ToolError> {
        let vector_db = self.vector_db.read().await;
        vector_db.remove_tool(tool_name).await.map_err(|e| {
            ToolError::ExecutionError(format!("Failed to remove tool {}: {}", tool_name, e))
        })?;
        Ok(())
    }

    async fn record_tool_call(&self, tool_name: &str) -> Result<(), ToolError> {
        let mut recent_calls = self.recent_tool_calls.write().await;
        if recent_calls.len() >= 100 {
            recent_calls.pop_front();
        }
        recent_calls.push_back(tool_name.to_string());
        Ok(())
    }

    async fn get_recent_tool_calls(&self, limit: usize) -> Result<Vec<String>, ToolError> {
        let recent_calls = self.recent_tool_calls.read().await;
        Ok(recent_calls.iter().rev().take(limit).cloned().collect())
    }

    fn selector_type(&self) -> RouterToolSelectionStrategy {
        RouterToolSelectionStrategy::Vector
    }
}

pub struct LLMToolSelector {
    llm_provider: Arc<dyn Provider>,
    tool_strings: Arc<RwLock<HashMap<String, String>>>, // extension_name -> tool_string
    recent_tool_calls: Arc<RwLock<VecDeque<String>>>,
}

impl LLMToolSelector {
    pub async fn new(provider: Arc<dyn Provider>) -> Result<Self> {
        Ok(Self {
            llm_provider: provider.clone(),
            tool_strings: Arc::new(RwLock::new(HashMap::new())),
            recent_tool_calls: Arc::new(RwLock::new(VecDeque::with_capacity(100))),
        })
    }
}

#[async_trait]
impl RouterToolSelector for LLMToolSelector {
    async fn select_tools(&self, params: Value) -> Result<Vec<Content>, ToolError> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'query' parameter".to_string()))?;

        let extension_name = params
            .get("extension_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // Get relevant tool strings based on extension_name
        let tool_strings = self.tool_strings.read().await;
        let relevant_tools = if let Some(ext) = &extension_name {
            tool_strings.get(ext).cloned()
        } else {
            // If no extension specified, use all tools
            Some(
                tool_strings
                    .values()
                    .cloned()
                    .collect::<Vec<String>>()
                    .join("\n"),
            )
        };

        if let Some(tools) = relevant_tools {
            // Use LLM to search through tools
            let prompt = format!(
                "Given the following tools:\n{}\n\nFind the most relevant tools for the query: {}\n\nReturn the tools in this exact format for each tool:\nTool: <tool_name>\nDescription: <tool_description>\nSchema: <tool_schema>",
                tools, query
            );
            let system_message = Message::user().with_text("You are a tool selection assistant. Your task is to find the most relevant tools based on the user's query.");
            let response = self
                .llm_provider
                .complete(&prompt, &[system_message], &[])
                .await
                .map_err(|e| ToolError::ExecutionError(format!("Failed to search tools: {}", e)))?;

            // Extract just the message content from the response
            let (message, _usage) = response;
            let text = message.content[0].as_text().unwrap_or_default();

            // Split the response into individual tool entries
            let tool_entries: Vec<Content> = text
                .split("\n\n")
                .filter(|entry| entry.trim().starts_with("Tool:"))
                .map(|entry| Content::text(entry.trim().to_string()))
                .collect();

            Ok(tool_entries)
        } else {
            Ok(vec![])
        }
    }

    async fn index_tools(&self, tools: &[Tool], extension_name: &str) -> Result<(), ToolError> {
        let mut tool_strings = self.tool_strings.write().await;

        for tool in tools {
            let tool_string = format!(
                "Tool: {}\nDescription: {}\nSchema: {}",
                tool.name,
                tool.description
                    .as_ref()
                    .map(|d| d.as_ref())
                    .unwrap_or_default(),
                serde_json::to_string_pretty(&tool.input_schema)
                    .unwrap_or_else(|_| "{}".to_string())
            );

            // Use the provided extension_name instead of parsing from tool name
            let entry = tool_strings.entry(extension_name.to_string()).or_default();

            // Check if this tool already exists in the entry
            if !entry.contains(&format!("Tool: {}", tool.name)) {
                if !entry.is_empty() {
                    entry.push_str("\n\n");
                }
                entry.push_str(&tool_string);
            }
        }

        Ok(())
    }
    async fn remove_tool(&self, tool_name: &str) -> Result<(), ToolError> {
        let mut tool_strings = self.tool_strings.write().await;
        if let Some(extension_name) = tool_name.split("__").next() {
            tool_strings.remove(extension_name);
        }
        Ok(())
    }

    async fn record_tool_call(&self, tool_name: &str) -> Result<(), ToolError> {
        let mut recent_calls = self.recent_tool_calls.write().await;
        if recent_calls.len() >= 100 {
            recent_calls.pop_front();
        }
        recent_calls.push_back(tool_name.to_string());
        Ok(())
    }

    async fn get_recent_tool_calls(&self, limit: usize) -> Result<Vec<String>, ToolError> {
        let recent_calls = self.recent_tool_calls.read().await;
        Ok(recent_calls.iter().rev().take(limit).cloned().collect())
    }

    fn selector_type(&self) -> RouterToolSelectionStrategy {
        RouterToolSelectionStrategy::Llm
    }
}

// Helper function to create a boxed tool selector
pub async fn create_tool_selector(
    strategy: Option<RouterToolSelectionStrategy>,
    provider: Arc<dyn Provider>,
    table_name: Option<String>,
) -> Result<Box<dyn RouterToolSelector>> {
    match strategy {
        Some(RouterToolSelectionStrategy::Vector) => {
            let selector = VectorToolSelector::new(provider, table_name.unwrap()).await?;
            Ok(Box::new(selector))
        }
        Some(RouterToolSelectionStrategy::Llm) => {
            let selector = LLMToolSelector::new(provider).await?;
            Ok(Box::new(selector))
        }
        None => {
            let selector = LLMToolSelector::new(provider).await?;
            Ok(Box::new(selector))
        }
    }
}
