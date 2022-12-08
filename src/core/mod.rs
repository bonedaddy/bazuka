mod address;
mod blocks;
pub mod hash;
mod header;
mod money;
mod transaction;

use crate::crypto;
use crate::zk;

pub use money::Money;

pub type Hasher = hash::Sha3Hasher;
pub type Signer = crypto::ed25519::Ed25519<Hasher>;

pub type ZkHasher = crate::zk::PoseidonHasher;
pub type ZkSigner = crypto::jubjub::JubJub<ZkHasher>;

pub type Address = address::Address<Signer>;
pub type ParseAddressError = address::ParseAddressError;
pub type Account = address::Account;
pub type Signature = address::Signature<Signer>;
pub type Transaction = transaction::Transaction<Hasher, Signer>;
pub type TransactionData = transaction::TransactionData<Hasher, Signer>;
pub type RegularSendEntry = transaction::RegularSendEntry<Signer>;
pub type ContractAccount = transaction::ContractAccount;
pub type ContractUpdate = transaction::ContractUpdate<Hasher, Signer>;
pub type ContractDeposit = transaction::ContractDeposit<Hasher, Signer>;
pub type ContractWithdraw = transaction::ContractWithdraw<Hasher, Signer>;
pub type MpnAddress = address::MpnAddress<ZkSigner>;
pub type MpnDeposit = transaction::MpnDeposit<Hasher, Signer, ZkSigner>;
pub type MpnWithdraw = transaction::MpnWithdraw<Hasher, Signer, ZkSigner>;
pub type MpnTransaction = zk::MpnTransaction;
pub type Header = header::Header<Hasher>;
pub type Block = blocks::Block<Hasher, Signer>;

pub type ProofOfWork = header::ProofOfWork;
pub type ContractId = transaction::ContractId<Hasher>;

pub type TransactionAndDelta = transaction::TransactionAndDelta<Hasher, Signer>;

// Transactions initiated from chain accounts
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub enum ChainSourcedTx {
    TransactionAndDelta(TransactionAndDelta),
    MpnDeposit(MpnDeposit),
}

impl ChainSourcedTx {
    pub fn nonce(&self) -> u32 {
        match self {
            ChainSourcedTx::TransactionAndDelta(tx_delta) => tx_delta.tx.nonce,
            ChainSourcedTx::MpnDeposit(mpn_deposit) => mpn_deposit.payment.nonce,
        }
    }
    pub fn validate_minimum_fee(&self, min_fee: &Money) -> bool {
        match self {
            ChainSourcedTx::TransactionAndDelta(tx_delta) => tx_delta.tx.fee.ge(min_fee),
            ChainSourcedTx::MpnDeposit(mpn_deposit) => mpn_deposit.payment.fee.ge(min_fee),
        }
    }
}

// Transactions initiated from MPN accounts
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub enum MpnSourcedTx {
    MpnTransaction(MpnTransaction),
    MpnWithdraw(MpnWithdraw),
}

impl MpnSourcedTx {
    pub fn nonce(&self) -> u64 {
        match self {
            MpnSourcedTx::MpnTransaction(mpn_tx) => mpn_tx.nonce,
            MpnSourcedTx::MpnWithdraw(mpn_withdraw) => mpn_withdraw.zk_nonce,
        }
    }
    pub fn validate_minimum_fee(&self, min_fee: &Money) -> bool {
        match self {
            MpnSourcedTx::MpnTransaction(mpn_tx) => mpn_tx.fee.ge(min_fee),
            MpnSourcedTx::MpnWithdraw(mpn_withdraw) => mpn_withdraw.payment.fee.ge(min_fee),
        }
    }
}

impl PartialEq<ChainSourcedTx> for ChainSourcedTx {
    fn eq(&self, other: &Self) -> bool {
        bincode::serialize(self).unwrap() == bincode::serialize(other).unwrap()
    }
}
impl Eq for ChainSourcedTx {}
impl std::hash::Hash for ChainSourcedTx {
    fn hash<Hasher>(&self, state: &mut Hasher)
    where
        Hasher: std::hash::Hasher,
    {
        state.write(&bincode::serialize(self).unwrap());
        state.finish();
    }
}

impl PartialEq<MpnSourcedTx> for MpnSourcedTx {
    fn eq(&self, other: &Self) -> bool {
        bincode::serialize(self).unwrap() == bincode::serialize(other).unwrap()
    }
}
impl Eq for MpnSourcedTx {}
impl std::hash::Hash for MpnSourcedTx {
    fn hash<Hasher>(&self, state: &mut Hasher)
    where
        Hasher: std::hash::Hasher,
    {
        state.write(&bincode::serialize(self).unwrap());
        state.finish();
    }
}

#[cfg(test)]
mod test {
    use std::str::FromStr;

    use super::*;
    use crate::node::test::sample_contract_call;

    #[test]
    fn validate_chain_sourced_tx_fee() {
        let min_fee = Money::from_str("0.0001").unwrap();
        let invalid_fee = Money::from_str("0.0000001").unwrap();
        let mut tx_delta_valid_fee = sample_contract_call();
        let mut tx_delta_invalid_fee = tx_delta_valid_fee.clone();

        tx_delta_valid_fee.tx.fee = min_fee;
        tx_delta_invalid_fee.tx.fee = invalid_fee;
        let valid_ctx_delta = ChainSourcedTx::TransactionAndDelta(tx_delta_valid_fee);
        let invalid_ctx_delta = ChainSourcedTx::TransactionAndDelta(tx_delta_invalid_fee);

        assert!(valid_ctx_delta.validate_minimum_fee(&min_fee));
        assert!(!invalid_ctx_delta.validate_minimum_fee(&min_fee));
    }
}
