use super::*;
use crate::{common::*, core::Header};

pub async fn sync_blocks<B: Blockchain>(
    context: &Arc<RwLock<NodeContext<B>>>,
) -> Result<(), NodeError> {
    let ctx = context.read().await;
    let net = ctx.outgoing.clone();
    let opts = ctx.opts.clone();
    let max_block_size = ctx.blockchain.config().max_block_size;
    let mut sorted_peers = ctx.peer_manager.get_peers().cloned().collect::<Vec<_>>();
    drop(ctx);

    sorted_peers.sort_by_key(|p| p.power);

    for peer in sorted_peers.iter().rev() {
        if peer.outdated_states > 0 {
            log::info!("Skipped syncing with {} (Outdated)", peer.address);
            continue;
        }
        let mut net_fail = false;
        let mut chain_fail = false;
        loop {
            let ctx = context.read().await;
            let min_target = ctx.blockchain.config().minimum_pow_difficulty;
            if peer.power <= ctx.blockchain.get_power()? {
                return Ok(());
            }

            println!(
                "Syncing blocks with: {} (Peer power: {})",
                peer.address, peer.power
            );

            let local_height = ctx.blockchain.get_height()?;
            let start_height = std::cmp::min(local_height, peer.height);
            drop(ctx);

            // WARN: Chain might change when getting responses from users, maybe get all data needed before dropping ctx

            // Get all headers starting from the indices that we don't have.
            let resp = if let Ok(resp) = net
                .bincode_get::<GetHeadersRequest, GetHeadersResponse>(
                    format!("http://{}/bincode/headers", peer.address),
                    GetHeadersRequest {
                        since: start_height,
                        count: opts.max_blocks_fetch,
                    },
                    Limit::default()
                        .size(opts.max_blocks_fetch * KB)
                        .time(5 * SECOND),
                )
                .await
            {
                resp
            } else {
                net_fail = true;
                break;
            };

            let (mut headers, pow_keys) = (resp.headers, resp.pow_keys);

            if headers.is_empty() || pow_keys.is_empty() {
                log::warn!("Peer returned no headers!");
                chain_fail = true;
                break;
            }

            if headers.len() != pow_keys.len() {
                log::warn!("PoW keys are not provided along headers!");
                chain_fail = true;
                break;
            }

            let ctx = context.read().await;
            for (i, (head, pow_key)) in headers.iter().zip(pow_keys.into_iter()).enumerate() {
                // perform the fastest to fail verification checks first
                if head.number != start_height + i as u64 {
                    log::warn!("Bad header number returned!");
                    chain_fail = true;
                    break;
                }
                if !validate_parent_hash(i, &headers[..], head) {
                    chain_fail = true;
                    break;
                }
                if head.proof_of_work.target < min_target || !head.meets_target(&pow_key) {
                    log::warn!("Header doesn't meet min target!");
                    chain_fail = true;
                    break;
                }

                if head.number < local_height && head == &ctx.blockchain.get_header(head.number)? {
                    log::warn!("Duplicate header given!");
                    chain_fail = true;
                    break;
                }
            }
            drop(ctx);

            if chain_fail || net_fail {
                break;
            }

            log::info!(
                "Got headers {}-{}...",
                start_height,
                start_height + headers.len() as u64
            );

            // The local blockchain and the peer blockchain both have all blocks
            // from 0 to height-1, though, the blocks might not be equal. Find
            // the header from which the fork has happened.
            for index in (0..start_height).rev() {
                let peer_resp = if let Ok(resp) = net
                    .bincode_get::<GetHeadersRequest, GetHeadersResponse>(
                        format!("http://{}/bincode/headers", peer.address),
                        GetHeadersRequest {
                            since: index,
                            count: 1,
                        },
                        Limit::default().size(1 * KB).time(3 * SECOND),
                    )
                    .await
                {
                    resp
                } else {
                    net_fail = true;
                    break;
                };
                if peer_resp.headers.is_empty() || peer_resp.pow_keys.is_empty() {
                    log::warn!("Peer is not providing claimed headers!");
                    chain_fail = true;
                    break;
                }

                let (peer_header, peer_pow_key) =
                    (peer_resp.headers[0].clone(), peer_resp.pow_keys[0].clone());

                if peer_header.number > 0
                    && (peer_header.proof_of_work.target < min_target
                        || !peer_header.meets_target(&peer_pow_key))
                {
                    log::warn!("Header doesn't meet min target!");
                    chain_fail = true;
                    break;
                }
                if peer_header.number != index as u64 {
                    log::warn!("Bad header number!");
                    chain_fail = true;
                    break;
                }
                if peer_header.hash() != headers[0].parent_hash {
                    log::warn!("Bad header hash!");
                    chain_fail = true;
                    break;
                }

                log::info!("Got header {}...", index);

                let ctx = context.read().await;
                let local_header = ctx.blockchain.get_headers(index, 1)?.first().cloned();
                drop(ctx);

                if let Some(local_header) = local_header {
                    if local_header.hash() != peer_header.hash() {
                        headers.insert(0, peer_header);
                    } else {
                        break;
                    }
                } else {
                    chain_fail = true;
                    break;
                }
            }

            if chain_fail || net_fail {
                break;
            }

            let ctx = context.read().await;
            if headers.iter().any(|h| ctx.banned_headers.contains_key(h)) {
                chain_fail = true;
                log::warn!("Chain has banned headers!");
                break;
            }

            let will_extend = match ctx
                .blockchain
                .will_extend(headers[0].number, &headers, true)
            {
                Ok(result) => {
                    if !result {
                        log::warn!("Chain is not powerful enough!");
                    }
                    result
                }
                Err(e) => {
                    log::warn!("Chain is invalid! Error: {}", e);
                    false
                }
            };

            if !will_extend {
                chain_fail = true;
                break;
            }

            drop(ctx);

            if let Ok(resp) = net
                .bincode_get::<GetBlocksRequest, GetBlocksResponse>(
                    format!("http://{}/bincode/blocks", peer.address).to_string(),
                    GetBlocksRequest {
                        since: headers[0].number,
                        count: opts.max_blocks_fetch,
                    },
                    Limit::default()
                        .size(opts.max_blocks_fetch as u64 * max_block_size as u64 * 2)
                        .time(opts.max_blocks_fetch as u32 * 30 * SECOND),
                )
                .await
            {
                let mut ctx = context.write().await;

                match ctx.blockchain.extend(headers[0].number, &resp.blocks) {
                    Ok(_) => {
                        println!("Height advanced to {}!", ctx.blockchain.get_height()?);
                        ctx.on_update()?;
                    }
                    Err(e) => {
                        chain_fail = true;
                        log::warn!("Cannot extend the blockchain. Error: {}", e);
                        break;
                    }
                }
            } else {
                net_fail = true;
                log::warn!("Network error! Cannot fetch blocks...");
                break;
            }
        }
        if chain_fail {
            context.write().await.punish_bad_behavior(
                peer.address,
                opts.incorrect_power_punish,
                "Cannot sync blocks!",
            );
        } else if net_fail {
            context.write().await.punish_unresponsive(peer.address);
        }
    }

    Ok(())
}

/// returns `true` if the parent hash verification is ok, false if it is not
fn validate_parent_hash(
    idx: usize,
    parent_headers: &[Header],
    header: &crate::core::Header,
) -> bool {
    if idx == 0 || idx - 1 > parent_headers.len() {
        return false;
    };
    let parent_header_hash = parent_headers[idx - 1].hash();
    let ok = if parent_header_hash.ne(&header.parent_hash) {
        log::warn!(
            "found bad parent hash. got {:?}, want {:?}, offender {:?}",
            header.parent_hash,
            parent_header_hash,
            header
        );
        false
    } else {
        true
    };
    ok
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::blockchain::test::easy_config;
    use crate::blockchain::KvStoreChain;
    use crate::db::RamKvStore;
    use crate::node::TxBuilder;
    use std::collections::HashMap;
    #[test]
    fn test_validate_parent_hash() {
        let miner = TxBuilder::new(&Vec::from("MINER"));

        let mut chain = KvStoreChain::new(RamKvStore::new(), easy_config()).unwrap();

        let new_block = chain
            .draft_block(60, &mut HashMap::new(), &miner, true)
            .unwrap()
            .unwrap()
            .block;
        chain.extend(1, &[new_block.clone()]).unwrap();

        let new_block = chain
            .draft_block(60, &mut HashMap::new(), &miner, true)
            .unwrap()
            .unwrap()
            .block;
        chain.extend(2, &[new_block.clone()]).unwrap();

        let new_block = chain
            .draft_block(60, &mut HashMap::new(), &miner, true)
            .unwrap()
            .unwrap()
            .block;
        chain.extend(3, &[new_block.clone()]).unwrap();

        let new_block = chain
            .draft_block(60, &mut HashMap::new(), &miner, true)
            .unwrap()
            .unwrap()
            .block;
        chain.extend(4, &[new_block.clone()]).unwrap();

        let genesis = chain.get_header(0).unwrap();
        let block_two = chain.get_header(1).unwrap();
        let block_three = chain.get_header(2).unwrap();
        let block_four = chain.get_header(3).unwrap();

        assert!(validate_parent_hash(1, &[genesis.clone()], &block_two) == true);
        assert!(
            validate_parent_hash(2, &[genesis.clone(), block_two.clone()], &block_three) == true
        );
        // verify invalid header detection
        assert!(
            validate_parent_hash(
                0,
                &[genesis.clone(), block_four.clone(), block_three.clone()],
                &block_two
            ) == false
        );
        // verify invalid header detection
        assert!(
            validate_parent_hash(
                3,
                &[genesis.clone(), block_four.clone(), block_three.clone()],
                &block_two
            ) == false
        );
        // verify attempting to access past the array length doesnt panic
        assert!(
            validate_parent_hash(
                10,
                &[genesis.clone(), block_four.clone(), block_three.clone()],
                &block_two
            ) == false
        );
    }
}
