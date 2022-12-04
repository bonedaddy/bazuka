use super::messages::{TransactRequest, TransactResponse};
use super::{NodeContext, NodeError};
use crate::blockchain::{Blockchain, TransactionStats};
use std::sync::Arc;
use tokio::sync::RwLock;

pub async fn transact<B: Blockchain>(
    context: Arc<RwLock<NodeContext<B>>>,
    req: TransactRequest,
) -> Result<TransactResponse, NodeError> {
    if req.tx_delta.tx.verify_signature() {
        return Err(NodeError::SignatureVerification);
    }
    let mut context = context.write().await;
    let now = context.local_timestamp();
    context
        .mempool
        .tx
        .insert(req.tx_delta, TransactionStats { first_seen: now });
    Ok(TransactResponse {})
}
