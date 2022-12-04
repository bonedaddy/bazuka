use super::*;
use crate::blockchain::TransactionStats;
use crate::common::*;

pub async fn sync_mempool<B: Blockchain>(
    context: &Arc<RwLock<NodeContext<B>>>,
) -> Result<(), NodeError> {
    let ctx = context.read().await;

    let net = ctx.outgoing.clone();
    let opts = ctx.opts.clone();

    let peer_addresses = ctx.peer_manager.random_peers(opts.num_peers);
    drop(ctx);

    log::info!("Syncing mempools...");
    let peer_responses: Vec<(Peer, Result<GetMempoolResponse, NodeError>)> =
        http::group_request(&peer_addresses, |peer| {
            net.bincode_get::<GetMempoolRequest, GetMempoolResponse>(
                format!("http://{}/bincode/mempool", peer.address),
                GetMempoolRequest {},
                Limit::default().size(10 * MB).time(10 * SECOND),
            )
        })
        .await;

    {
        let mut ctx = context.write().await;
        let now = ctx.local_timestamp();
        let resps = punish_non_responding(&mut ctx, &peer_responses)
            .into_iter()
            .map(|(_, r)| (r.tx, r.tx_zk, r.zk_tx, r.zk))
            .collect::<Vec<_>>();
        for (tx_s, tx_zk_s, zk_tx_s, zk_s) in resps {
            tx_s.into_iter()
                .filter(|tx| tx.tx.verify_signature())
                .for_each(|tx| {
                    ctx.mempool
                        .tx
                        .entry(tx)
                        .or_insert(TransactionStats { first_seen: now });
                });
            tx_zk_s
                .into_iter()
                .filter(|tx| tx.payment.verify_signature())
                .for_each(|tx| {
                    ctx.mempool
                        .tx_zk
                        .entry(tx)
                        .or_insert(TransactionStats { first_seen: now });
                });
            zk_tx_s.into_iter().for_each(|tx| {
                ctx.mempool
                    .zk_tx
                    .entry(tx)
                    .or_insert(TransactionStats { first_seen: now });
            });
            zk_s.into_iter().for_each(|tx| {
                ctx.mempool
                    .zk
                    .entry(tx)
                    .or_insert(TransactionStats { first_seen: now });
            });
        }
    }

    Ok(())
}
