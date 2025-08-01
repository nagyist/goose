use std::collections::HashMap;
use std::time::Duration;

use super::base::{ConfigKey, Provider, ProviderMetadata, ProviderUsage};
use super::errors::ProviderError;
use crate::impl_provider_default;
use crate::message::Message;
use crate::model::ModelConfig;
use crate::providers::utils::emit_debug_trace;
use anyhow::Result;
use async_trait::async_trait;
use aws_sdk_bedrockruntime::config::ProvideCredentials;
use aws_sdk_bedrockruntime::operation::converse::ConverseError;
use aws_sdk_bedrockruntime::{types as bedrock, Client};
use rmcp::model::Tool;
use serde_json::Value;
use tokio::time::sleep;

// Import the migrated helper functions from providers/formats/bedrock.rs
use super::formats::bedrock::{
    from_bedrock_message, from_bedrock_usage, to_bedrock_message, to_bedrock_tool_config,
};

pub const BEDROCK_DOC_LINK: &str =
    "https://docs.aws.amazon.com/bedrock/latest/userguide/models-supported.html";

pub const BEDROCK_DEFAULT_MODEL: &str = "anthropic.claude-3-5-sonnet-20240620-v1:0";
pub const BEDROCK_KNOWN_MODELS: &[&str] = &[
    "anthropic.claude-3-5-sonnet-20240620-v1:0",
    "anthropic.claude-3-5-sonnet-20241022-v2:0",
];

#[derive(Debug, serde::Serialize)]
pub struct BedrockProvider {
    #[serde(skip)]
    client: Client,
    model: ModelConfig,
}

impl BedrockProvider {
    pub fn from_env(model: ModelConfig) -> Result<Self> {
        let config = crate::config::Config::global();

        // Attempt to load config and secrets to get AWS_ prefixed keys
        // to re-export them into the environment for aws_config::load_from_env()
        let set_aws_env_vars = |res: Result<HashMap<String, Value>, _>| {
            if let Ok(map) = res {
                map.into_iter()
                    .filter(|(key, _)| key.starts_with("AWS_"))
                    .filter_map(|(key, value)| value.as_str().map(|s| (key, s.to_string())))
                    .for_each(|(key, s)| std::env::set_var(key, s));
            }
        };

        set_aws_env_vars(config.load_values());
        set_aws_env_vars(config.load_secrets());

        let sdk_config = futures::executor::block_on(aws_config::load_from_env());

        // validate credentials or return error back up
        futures::executor::block_on(
            sdk_config
                .credentials_provider()
                .unwrap()
                .provide_credentials(),
        )?;
        let client = Client::new(&sdk_config);

        Ok(Self { client, model })
    }
}

impl_provider_default!(BedrockProvider);

#[async_trait]
impl Provider for BedrockProvider {
    fn metadata() -> ProviderMetadata {
        ProviderMetadata::new(
            "aws_bedrock",
            "Amazon Bedrock",
            "Run models through Amazon Bedrock. You may have to set 'AWS_' environment variables to configure authentication.",
            BEDROCK_DEFAULT_MODEL,
            BEDROCK_KNOWN_MODELS.to_vec(),
            BEDROCK_DOC_LINK,
            vec![ConfigKey::new("AWS_PROFILE", true, false, Some("default"))],
        )
    }

    fn get_model_config(&self) -> ModelConfig {
        self.model.clone()
    }

    #[tracing::instrument(
        skip(self, system, messages, tools),
        fields(model_config, input, output, input_tokens, output_tokens, total_tokens)
    )]
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[Tool],
    ) -> Result<(Message, ProviderUsage), ProviderError> {
        let model_name = &self.model.model_name;

        let mut request = self
            .client
            .converse()
            .system(bedrock::SystemContentBlock::Text(system.to_string()))
            .model_id(model_name.to_string())
            .set_messages(Some(
                messages
                    .iter()
                    .map(to_bedrock_message)
                    .collect::<Result<_>>()?,
            ));

        if !tools.is_empty() {
            request = request.tool_config(to_bedrock_tool_config(tools)?);
        }

        // Retry configuration
        const MAX_RETRIES: u32 = 10;
        const INITIAL_BACKOFF_MS: u64 = 20_000; // 20 seconds
        const MAX_BACKOFF_MS: u64 = 120_000; // 120 seconds (2 minutes)

        let mut attempts = 0;
        let mut backoff_ms = INITIAL_BACKOFF_MS;

        loop {
            attempts += 1;

            match request.clone().send().await {
                Ok(response) => {
                    // Successful response, process it and return
                    return match response.output {
                        Some(bedrock::ConverseOutput::Message(message)) => {
                            let usage = response
                                .usage
                                .as_ref()
                                .map(from_bedrock_usage)
                                .unwrap_or_default();

                            let message = from_bedrock_message(&message)?;

                            // Add debug trace with input context
                            let debug_payload = serde_json::json!({
                                "system": system,
                                "messages": messages,
                                "tools": tools
                            });
                            emit_debug_trace(
                                &self.model,
                                &debug_payload,
                                &serde_json::to_value(&message).unwrap_or_default(),
                                &usage,
                            );

                            let provider_usage = ProviderUsage::new(model_name.to_string(), usage);
                            Ok((message, provider_usage))
                        }
                        _ => Err(ProviderError::RequestFailed(
                            "No output from Bedrock".to_string(),
                        )),
                    };
                }
                Err(err) => {
                    match err.into_service_error() {
                        ConverseError::ThrottlingException(throttle_err) => {
                            if attempts > MAX_RETRIES {
                                // We've exhausted our retries
                                tracing::error!(
                                    "Failed after {MAX_RETRIES} retries: {:?}",
                                    throttle_err
                                );
                                return Err(ProviderError::RateLimitExceeded(format!(
                                    "Failed to call Bedrock after {MAX_RETRIES} retries: {:?}",
                                    throttle_err
                                )));
                            }

                            // Log retry attempt
                            tracing::warn!(
                                "Bedrock throttling error (attempt {}/{}), retrying in {} ms: {:?}",
                                attempts,
                                MAX_RETRIES,
                                backoff_ms,
                                throttle_err
                            );

                            // Wait before retry with exponential backoff
                            sleep(Duration::from_millis(backoff_ms)).await;

                            // Calculate next backoff with exponential growth, capped at max
                            backoff_ms = (backoff_ms * 2).min(MAX_BACKOFF_MS);

                            // Continue to the next retry attempt
                            continue;
                        }
                        ConverseError::AccessDeniedException(err) => {
                            return Err(ProviderError::Authentication(format!(
                                "Failed to call Bedrock: {:?}",
                                err
                            )));
                        }
                        ConverseError::ValidationException(err)
                            if err
                                .message()
                                .unwrap_or_default()
                                .contains("Input is too long for requested model.") =>
                        {
                            return Err(ProviderError::ContextLengthExceeded(format!(
                                "Failed to call Bedrock: {:?}",
                                err
                            )));
                        }
                        ConverseError::ModelErrorException(err) => {
                            return Err(ProviderError::ExecutionError(format!(
                                "Failed to call Bedrock: {:?}",
                                err
                            )));
                        }
                        err => {
                            return Err(ProviderError::ServerError(format!(
                                "Failed to call Bedrock: {:?}",
                                err
                            )));
                        }
                    }
                }
            }
        }
    }
}
