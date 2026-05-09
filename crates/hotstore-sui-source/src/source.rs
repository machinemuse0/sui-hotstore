use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone)]
pub struct RpcSourceClient {
    client: Client,
    rpc_url: String,
    max_retries: usize,
    retry_backoff_ms: u64,
}

impl RpcSourceClient {
    pub fn new(rpc_url: String, max_retries: usize, retry_backoff_ms: u64) -> Result<Self> {
        let client = Client::builder()
            .user_agent("sui-hotstore-demo/0.1")
            .build()
            .context("failed to build reqwest client")?;

        Ok(Self {
            client,
            rpc_url,
            max_retries,
            retry_backoff_ms,
        })
    }

    pub async fn fetch_checkpoint(&self, checkpoint_seq: u64) -> Result<RpcCheckpoint> {
        self.call("sui_getCheckpoint", json!([checkpoint_seq.to_string()]))
            .await
    }

    pub async fn fetch_transaction_blocks(
        &self,
        digests: &[String],
        batch_size: usize,
    ) -> Result<Vec<RpcTransactionBlockResponse>> {
        if digests.is_empty() {
            return Ok(Vec::new());
        }

        let mut out = Vec::with_capacity(digests.len());
        let batch_size = batch_size.max(1);

        for chunk in digests.chunks(batch_size) {
            let params = json!([
                chunk,
                {
                    "showInput": true,
                    "showEffects": true,
                    "showEvents": true,
                    "showObjectChanges": true,
                }
            ]);

            match self
                .call::<Vec<RpcTransactionBlockResponse>>("sui_multiGetTransactionBlocks", params)
                .await
            {
                Ok(mut chunk_rows) => out.append(&mut chunk_rows),
                Err(_) => {
                    for digest in chunk {
                        out.push(self.fetch_transaction_block(digest).await?);
                    }
                }
            }
        }

        Ok(out)
    }

    async fn fetch_transaction_block(&self, digest: &str) -> Result<RpcTransactionBlockResponse> {
        self.call(
            "sui_getTransactionBlock",
            json!([
                digest,
                {
                    "showInput": true,
                    "showEffects": true,
                    "showEvents": true,
                    "showObjectChanges": true,
                }
            ]),
        )
        .await
    }

    async fn call<T>(&self, method: &str, params: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let mut attempts = 0usize;

        loop {
            match self.call_once(method, params.clone()).await {
                Ok(value) => return Ok(value),
                Err(error) => {
                    attempts += 1;
                    if attempts > self.max_retries {
                        return Err(error).with_context(|| {
                            format!("RPC method `{method}` failed after {} attempts", attempts)
                        });
                    }

                    sleep(Duration::from_millis(
                        self.retry_backoff_ms.saturating_mul(attempts as u64),
                    ))
                    .await;
                }
            }
        }
    }

    async fn call_once<T>(&self, method: &str, params: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let response = self
            .client
            .post(&self.rpc_url)
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params,
            }))
            .send()
            .await
            .with_context(|| format!("RPC request failed for method `{method}`"))?;

        let status = response.status();
        let envelope: JsonRpcEnvelope<T> = response
            .json()
            .await
            .with_context(|| format!("failed to decode RPC response for method `{method}`"))?;

        if !status.is_success() {
            bail!("RPC method `{method}` returned HTTP {status}");
        }

        if let Some(error) = envelope.error {
            bail!("RPC method `{method}` failed: {}", error.message);
        }

        envelope
            .result
            .ok_or_else(|| anyhow!("RPC method `{method}` returned no result"))
    }
}

#[derive(Debug, Deserialize)]
struct JsonRpcEnvelope<T> {
    result: Option<T>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    message: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcCheckpoint {
    pub sequence_number: StringOrNumber,
    pub timestamp_ms: StringOrNumber,
    #[serde(default)]
    pub transactions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcTransactionBlockResponse {
    pub digest: String,
    #[serde(default)]
    pub transaction: Option<RpcTransactionEnvelope>,
    #[serde(default)]
    pub effects: Option<RpcEffects>,
    #[serde(default)]
    pub events: Option<Vec<RpcEvent>>,
    #[serde(default)]
    pub object_changes: Option<Vec<RpcObjectChange>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcTransactionEnvelope {
    pub data: RpcTransactionData,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcTransactionData {
    #[serde(default)]
    pub sender: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcEffects {
    #[serde(default)]
    pub status: Option<RpcExecutionStatus>,
    #[serde(default)]
    pub gas_used: Option<RpcGasUsed>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RpcExecutionStatus {
    pub status: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcGasUsed {
    pub computation_cost: StringOrNumber,
    pub storage_cost: StringOrNumber,
    #[serde(default)]
    pub storage_rebate: Option<StringOrNumber>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcEvent {
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub package_id: Option<String>,
    #[serde(default)]
    pub transaction_module: Option<String>,
    #[serde(default)]
    pub sender: Option<String>,
    #[serde(default)]
    pub parsed_json: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcObjectChange {
    #[serde(rename = "type")]
    pub change_type: String,
    #[serde(default)]
    pub object_id: Option<String>,
    #[serde(default)]
    pub object_type: Option<String>,
    #[serde(default)]
    pub version: Option<StringOrNumber>,
    #[serde(default)]
    pub owner: Option<Value>,
    #[serde(default)]
    pub recipient: Option<Value>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum StringOrNumber {
    String(String),
    Number(u64),
}

impl StringOrNumber {
    pub fn as_u64(&self) -> Result<u64> {
        match self {
            Self::String(value) => value
                .parse::<u64>()
                .with_context(|| format!("failed to parse u64 from `{value}`")),
            Self::Number(value) => Ok(*value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StringOrNumber;

    #[test]
    fn string_or_number_parses_both_shapes() {
        assert_eq!(
            StringOrNumber::String("42".to_owned()).as_u64().unwrap(),
            42
        );
        assert_eq!(StringOrNumber::Number(9).as_u64().unwrap(), 9);
    }
}
